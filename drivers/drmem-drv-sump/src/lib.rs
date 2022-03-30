use async_trait::async_trait;
use drmem_api::{driver, types::Error, Result};
use std::net::{Ipv4Addr, SocketAddrV4};
use tokio::{
    io::{self, AsyncReadExt},
    net::{
        tcp::{OwnedReadHalf, OwnedWriteHalf},
        TcpStream,
    },
};
use tracing::{debug, error, info, warn};

const DESCRIPTION: &str = r#"
This driver monitors the state of a sump pump through a custom,
non-commercial interface and updates a set of devices based on its
behavior.

The sump pump state is obatined via TCP with a RaspberryPi that's
monitoring a GPIO pin for state changes of the sump pump. It sends a
12-byte packet whenever the state changes. The first 8 bytes holds a
millisecond timestamp in big-endian format. The following 4 bytes
holds the new state.

With these packets, the driver can use the timestamps to compute duty
cycles and incoming flows rates for the sump pit each time the pump
turns off. The `state`, `duty`, and `in-flow` parameters are updated
simulataneously and, hence will have the same timestamps.

# Configuration

The driver needs to know where to access the remote service. It also
needs to know how to scale the results. Two driver arguments are used
to specify this information:

- `addr` is a string containing the host name, or IP address, and port
  number of the machine that's actually monitoring the sump pump (in
  **"hostname:#"** or **"\#.#.#.#:#"** format.)
- `gpm` is an integer that repesents the gallons-per-minute capacity
  of the sump pump. The pump owner's manual will typically have a
  table indicating the flow rate based on the rise of the discharge
  pipe.

# Devices

The driver creates these devices:

| Base Name | Type     | Units | Comment                                                   |
|-----------|----------|-------|-----------------------------------------------------------|
| `service` | bool, RO |       | Set to `true` when communicating with the remote service. |
| `state`   | bool, RO |       | Set to `true` when the pump is running.                   |
| `duty`    | f64, RO  | %     | Indicates duty cycle of last cycle.                       |
| `in-flow` | f64, RO  | gpm   | Indicates the in-flow rate for the last cycle.            |

"#;

// The sump pump monitor uses a state machine to decide when to
// calculate the duty cycle and in-flow.

#[derive(Debug)]
enum State {
    Unknown,
    Off { off_time: u64 },
    On { off_time: u64, on_time: u64 },
}

// This interface allows a State value to update itself when an event
// occurs.

impl State {
    // This method is called when an off event occurs. The timestamp
    // of the off event needs to be provided. If the state machine has
    // enough information of the previous pump cycle, it will return
    // the duty cycle and in-flow rate. If the state machine is still
    // sync-ing with the state, the state will get updated, but `None`
    // will be returned.

    pub fn off_event(&mut self, stamp: u64) -> Option<(f64, f64)> {
        match *self {
            State::Unknown => {
                info!("sync-ed with OFF state");
                *self = State::Off { off_time: stamp };
                None
            }

            State::Off { .. } => {
                warn!("ignoring duplicate OFF event");
                None
            }

            State::On { off_time, on_time } => {
                // The time stamp of the OFF time should come after
                // the ON time. If it isn't, the sump pump task has a
                // problem (i.e. system time was adjusted.) We can't
                // give a decent computation, so just go into the DOWN
                // state.

                if on_time >= stamp {
                    warn!(
                        "timestamp for OFF event is {} ms ahead of ON event",
                        on_time - stamp
                    );
                    *self = State::Off { off_time: stamp };
                    return None;
                }

                let on_time = (stamp - on_time) as f64;

                // After the first storm, there was one entry that
                // glitched. The state of the motor registered "ON"
                // for 50 ms, turned off, turned on 400ms later, and
                // then stayed on for the rest of the normal,
                // six-second cycle.
                //
                // I'm going under the assumption that the pump wasn't
                // drawing enough current at the start of the cycle so
                // the current switch's detection "faded" in and out.
                // This could be due to not setting the sensitivity of
                // the switch high enough or, possibly, the pump
                // failing (once in a great while, we hear the pump go
                // through a strange-sounding cycle.)
                //
                // If the ON cycle is less than a half second, we'll
                // ignore it and stay in the ON state.

                if on_time > 500.0 {
                    let off_time = (stamp - off_time) as f64;
                    let duty = on_time * 1000.0 / off_time;
                    let in_flow = (2680.0 * duty / 60.0).round() / 1000.0;

                    *self = State::Off { off_time: stamp };
                    Some((duty.round() / 10.0, in_flow))
                } else {
                    warn!("ignoring short ON time -- {:.0} ms", on_time);
                    None
                }
            }
        }
    }

    // This method is called when updating the state with an on
    // event. The timestamp of the on event needs to be provided. If
    // the on event actually caused a state change, `true` is
    // returned.

    pub fn on_event(&mut self, stamp: u64) -> bool {
        match *self {
            State::Unknown => false,

            State::Off { off_time } => {
                // Make sure the ON time occurred *after* the OFF
                // time. This is necessary for the computations to
                // yield valid results.

                if stamp > off_time {
                    *self = State::On {
                        off_time,
                        on_time: stamp,
                    };
                    true
                } else {
                    warn!(
                        "timestamp for ON event is {} ms ahead of OFF event",
                        off_time - stamp
                    );
                    false
                }
            }

            State::On { .. } => {
                warn!("ignoring duplicate ON event");
                false
            }
        }
    }
}

pub struct Sump {
    rx: OwnedReadHalf,
    _tx: OwnedWriteHalf,
    state: State,
    d_service: driver::ReportReading,
    d_state: driver::ReportReading,
    d_duty: driver::ReportReading,
    d_inflow: driver::ReportReading,
}

impl Sump {
    pub async fn new(
        cfg: &driver::Config, core: driver::RequestChan,
    ) -> Result<Self> {
        // Validate the configuration.

        let _addr = match cfg.get("addr") {
            Some(addr) => addr,
            None => return Err(Error::BadConfig),
        };

        let _port = match cfg.get("port") {
            Some(port) => port,
            None => return Err(Error::BadConfig),
        };

        // Define the devices managed by this driver.

        let d_service = core.add_ro_device("service").await?;
        let d_state = core.add_ro_device("state").await?;
        let d_duty = core.add_ro_device("duty").await?;
        let d_inflow = core.add_ro_device("in-flow").await?;

        let addr = SocketAddrV4::new(Ipv4Addr::new(192, 168, 1, 101), 10_000);
        let s = TcpStream::connect(addr)
            .await
            .map_err(|_| Error::MissingPeer(String::from("sump pump")))?;

        // Unfortunately, we have to hang onto the xmt handle. The
        // peer process monitors the state of the socket and if we
        // close our send handle, it thinks we went away and closes
        // the other end.

        let (rx, tx) = s.into_split();

        Ok(Sump {
            rx,
            _tx: tx,
            state: State::Unknown,
            d_service,
            d_state,
            d_duty,
            d_inflow,
        })
    }

    // This function reads the next frame from the sump pump process.
    // It either returns `Ok()` with the two fields' values or `Err()`
    // if a socket error occurred.

    async fn get_reading(&mut self) -> io::Result<(u64, bool)> {
        let stamp = self.rx.read_u64().await?;
        let value = self.rx.read_u32().await?;

        Ok((stamp, value != 0))
    }
}

#[async_trait]
impl driver::API for Sump {
    fn create(
        _cfg: driver::Config, _drc: driver::RequestChan,
    ) -> Result<Box<dyn driver::API>>
    where
        Self: Sized,
    {
        todo!()
    }

    async fn run(mut self) -> Result<()> {
        (self.d_service)(true.into())?;

        loop {
            match self.get_reading().await {
                Ok((stamp, true)) => {
                    if self.state.on_event(stamp) {
                        (self.d_state)(true.into())?;
                    }
                }

                Ok((stamp, false)) => {
                    if let Some((duty, in_flow)) = self.state.off_event(stamp) {
                        info!("duty: {}%, in flow: {} gpm", duty, in_flow);

                        (self.d_state)(false.into())?;
                        (self.d_duty)(duty.into())?;
                        (self.d_inflow)(in_flow.into())?;
                    }
                }

                Err(e) => {
                    error!("couldn't read sump state -- {:?}", e);
                    (self.d_service)(false.into())?;
                    (self.d_state)(false.into())?;
                    break Err(Error::OperationError);
                }
            }
            debug!("state: {:?}", self.state);
        }
    }

    fn name(&self) -> &'static str {
        "sump"
    }

    fn description(&self) -> &'static str {
        DESCRIPTION
    }

    fn summary(&self) -> &'static str {
        "sump pump monitor"
    }
}

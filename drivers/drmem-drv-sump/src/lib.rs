use drmem_api::{
    device,
    driver::{self, DriverConfig},
    Error, Result,
};
use std::future::Future;
use std::net::SocketAddrV4;
use std::sync::Arc;
use std::{convert::Infallible, pin::Pin};
use tokio::{
    io::{self, AsyncReadExt},
    net::{
        tcp::{OwnedReadHalf, OwnedWriteHalf},
        TcpStream,
    },
    sync::Mutex,
    time,
};
use tracing::{debug, error, info, warn, Span};

// The sump pump monitor uses a state machine to decide when to
// calculate the duty cycle and in-flow.

#[cfg_attr(test, derive(Debug, PartialEq))]
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

    pub fn off_event(
        &mut self,
        stamp: u64,
        gpm: f64,
    ) -> Option<(u64, f64, f64)> {
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
                    let off_time = stamp - off_time;
                    let duty = on_time * 1000.0 / (off_time as f64);
                    let in_flow = (gpm * duty / 10.0).round() / 100.0;

                    *self = State::Off { off_time: stamp };
                    Some((off_time, duty.round() / 10.0, in_flow))
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

pub struct Instance {
    state: State,
    gpm: f64,
    rx: OwnedReadHalf,
    _tx: OwnedWriteHalf,
}

pub struct Devices {
    d_service: driver::ReportReading<bool>,
    d_state: driver::ReportReading<bool>,
    d_duty: driver::ReportReading<f64>,
    d_inflow: driver::ReportReading<f64>,
    d_duration: driver::ReportReading<f64>,
}

impl Instance {
    pub const NAME: &'static str = "sump-gpio";

    pub const SUMMARY: &'static str =
        "monitors and computes parameters for a sump pump";

    pub const DESCRIPTION: &'static str = include_str!("../README.md");

    fn elapsed(millis: u64) -> String {
        match (millis + 500) / 1000 {
            dur if dur >= 3600 * 24 - 30 => {
                let dur = dur + 30;

                format!(
                    "{}d{}h{}m",
                    dur / (3600 * 24),
                    (dur / 3600) % 24,
                    (dur / 60) % 60
                )
            }
            dur if dur >= 3570 => {
                let dur = dur + 30;

                format!("{}h{}m", dur / 3600, (dur / 60) % 60)
            }
            dur if dur >= 60 => {
                format!("{}m{}s", dur / 60, dur % 60)
            }
            dur => {
                format!("{}s", dur)
            }
        }
    }

    // Attempts to pull the hostname/port for the remote process.

    fn get_cfg_address(cfg: &DriverConfig) -> Result<SocketAddrV4> {
        match cfg.get("addr") {
            Some(toml::value::Value::String(addr)) => {
                if let Ok(addr) = addr.parse::<SocketAddrV4>() {
                    Ok(addr)
                } else {
                    Err(Error::ConfigError(String::from(
                        "'addr' not in hostname:port format",
                    )))
                }
            }
            Some(_) => Err(Error::ConfigError(String::from(
                "'addr' config parameter should be a string",
            ))),
            None => Err(Error::ConfigError(String::from(
                "missing 'addr' parameter in config",
            ))),
        }
    }

    // Attempts to pull the gal-per-min parameter from the driver's
    // configuration. The value can be specified as an integer or
    // floating point. It gets returned only as an `f64`.

    fn get_cfg_gpm(cfg: &DriverConfig) -> Result<f64> {
        match cfg.get("gpm") {
            Some(toml::value::Value::Integer(gpm)) => Ok(*gpm as f64),
            Some(toml::value::Value::Float(gpm)) => Ok(*gpm),
            Some(_) => Err(Error::ConfigError(String::from(
                "'gpm' config parameter should be a number",
            ))),
            None => Err(Error::ConfigError(String::from(
                "missing 'gpm' parameter in config",
            ))),
        }
    }

    fn connect(addr: &SocketAddrV4) -> Result<TcpStream> {
        use socket2::{Domain, Socket, TcpKeepalive, Type};

        let keepalive = TcpKeepalive::new()
            .with_time(time::Duration::from_secs(5))
            .with_interval(time::Duration::from_secs(5));
        let socket = Socket::new(Domain::IPV4, Type::STREAM, None)
            .expect("couldn't create socket");

        socket
            .set_tcp_keepalive(&keepalive)
            .expect("couldn't enable keep-alive on sump socket");

        match socket.connect_timeout(
            &<SocketAddrV4 as Into<socket2::SockAddr>>::into(*addr),
            time::Duration::from_millis(100),
        ) {
            Ok(()) => {
                info!("connected");

                // Before we move the socket into `tokio`'s control,
                // it must be placed in non-blocking mode.

                socket
                    .set_nonblocking(true)
                    .expect("couldn't make socket nonblocking");

                TcpStream::from_std(socket.into()).map_err(|_| {
                    error!("couldn't convert to tokio::TcpStream");
                    Error::MissingPeer(String::from("sump pump"))
                })
            }
            Err(_) => {
                error!("couldn't connect to {}", addr);
                Err(Error::MissingPeer(String::from("sump pump")))
            }
        }
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

impl driver::API for Instance {
    type DeviceSet = Devices;

    fn register_devices(
        core: driver::RequestChan,
        _: &DriverConfig,
        max_history: Option<usize>,
    ) -> Pin<Box<dyn Future<Output = Result<Self::DeviceSet>> + Send>> {
        let service_name = "service".parse::<device::Base>().unwrap();
        let state_name = "state".parse::<device::Base>().unwrap();
        let duty_name = "duty".parse::<device::Base>().unwrap();
        let in_flow_name = "in-flow".parse::<device::Base>().unwrap();
        let dur_name = "duration".parse::<device::Base>().unwrap();

        Box::pin(async move {
            // Define the devices managed by this driver.

            let (d_service, _) =
                core.add_ro_device(service_name, None, max_history).await?;
            let (d_state, _) =
                core.add_ro_device(state_name, None, max_history).await?;
            let (d_duty, _) = core
                .add_ro_device(duty_name, Some("%"), max_history)
                .await?;
            let (d_inflow, _) = core
                .add_ro_device(in_flow_name, Some("gpm"), max_history)
                .await?;
            let (d_duration, _) = core
                .add_ro_device(dur_name, Some("min"), max_history)
                .await?;

            Ok(Devices {
                d_service,
                d_state,
                d_duty,
                d_inflow,
                d_duration,
            })
        })
    }

    fn create_instance(
        cfg: &DriverConfig,
    ) -> Pin<Box<dyn Future<Output = Result<Box<Self>>> + Send>> {
        let addr = Instance::get_cfg_address(cfg);
        let gpm = Instance::get_cfg_gpm(cfg);

        let fut = async move {
            // Validate the configuration.

            let addr = addr?;
            let gpm = gpm?;

            Span::current().record("cfg", addr.to_string());

            // Connect with the remote process that is connected to
            // the sump pump.

            let (rx, _tx) = Instance::connect(&addr)?.into_split();

            Ok(Box::new(Instance {
                state: State::Unknown,
                gpm,
                rx,
                _tx,
            }))
        };

        Box::pin(fut)
    }

    fn run<'a>(
        &'a mut self,
        devices: Arc<Mutex<Devices>>,
    ) -> Pin<Box<dyn Future<Output = Infallible> + Send + 'a>> {
        let fut = async move {
            // Record the peer's address in the "cfg" field of the
            // span.

            {
                let addr = self
                    .rx
                    .peer_addr()
                    .map(|v| format!("{}", v))
                    .unwrap_or_else(|_| String::from("**unknown**"));

                Span::current().record("cfg", addr.as_str());
            }

            let devices = devices.lock().await;

            (*devices.d_service)(true).await;

            loop {
                match self.get_reading().await {
                    Ok((stamp, true)) => {
                        if self.state.on_event(stamp) {
                            (devices.d_state)(true).await;
                        }
                    }

                    Ok((stamp, false)) => {
                        let gpm = self.gpm;

                        if let Some((cycle, duty, in_flow)) =
                            self.state.off_event(stamp, gpm)
                        {
                            debug!(
                                "cycle: {}, duty: {:.1}%, inflow: {:.2} gpm",
                                Instance::elapsed(cycle),
                                duty,
                                in_flow
                            );

                            (devices.d_state)(false).await;
                            (devices.d_duty)(duty).await;
                            (devices.d_inflow)(in_flow).await;
                            (devices.d_duration)(
                                ((cycle as f64) / 600.0).round() / 100.0,
                            )
                            .await;
                        }
                    }

                    Err(e) => {
                        (devices.d_state)(false).await;
                        (devices.d_service)(false).await;
                        panic!("couldn't read sump state -- {:?}", e);
                    }
                }
            }
        };

        Box::pin(fut)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_states() {
        let mut state = State::Unknown;

        assert_eq!(state.on_event(0), false);
        assert_eq!(state, State::Unknown);

        state = State::Off { off_time: 100 };

        assert_eq!(state.on_event(0), false);
        assert_eq!(state, State::Off { off_time: 100 });
        assert_eq!(state.on_event(200), true);
        assert_eq!(
            state,
            State::On {
                off_time: 100,
                on_time: 200
            }
        );

        assert_eq!(state.on_event(200), false);
        assert_eq!(
            state,
            State::On {
                off_time: 100,
                on_time: 200
            }
        );

        state = State::Unknown;

        assert_eq!(state.off_event(1000, 50.0), None);
        assert_eq!(state, State::Off { off_time: 1000 });
        assert_eq!(state.off_event(1100, 50.0), None);
        assert_eq!(state, State::Off { off_time: 1000 });

        state = State::On {
            off_time: 1000,
            on_time: 101000,
        };

        assert_eq!(state.off_event(1000, 50.0), None);
        assert_eq!(state, State::Off { off_time: 1000 });

        state = State::On {
            off_time: 1000,
            on_time: 101000,
        };

        assert_eq!(state.off_event(101500, 50.0), None);
        assert_eq!(
            state,
            State::On {
                off_time: 1000,
                on_time: 101000
            }
        );

        assert!(state.off_event(101501, 50.0).is_some());
        assert_eq!(state, State::Off { off_time: 101501 });

        state = State::On {
            off_time: 0,
            on_time: 540000,
        };

        assert_eq!(state.off_event(600000, 50.0), Some((600000, 10.0, 5.0)));
        assert_eq!(state, State::Off { off_time: 600000 });

        state = State::On {
            off_time: 0,
            on_time: 54000,
        };

        assert_eq!(state.off_event(60000, 60.0), Some((60000, 10.0, 6.0)));
        assert_eq!(state, State::Off { off_time: 60000 });
    }

    #[test]
    fn test_elapsed() {
        assert_eq!(Instance::elapsed(0), "0s");
        assert_eq!(Instance::elapsed(1000), "1s");
        assert_eq!(Instance::elapsed(59000), "59s");
        assert_eq!(Instance::elapsed(60000), "1m0s");

        assert_eq!(Instance::elapsed(3569000), "59m29s");
        assert_eq!(Instance::elapsed(3570000), "1h0m");
        assert_eq!(Instance::elapsed(3599000), "1h0m");
        assert_eq!(Instance::elapsed(3600000), "1h0m");

        assert_eq!(Instance::elapsed(3600000 * 24 - 31000), "23h59m");
        assert_eq!(Instance::elapsed(3600000 * 24 - 30000), "1d0h0m");
        assert_eq!(Instance::elapsed(3600000 * 24 - 1000), "1d0h0m");
        assert_eq!(Instance::elapsed(3600000 * 24), "1d0h0m");
    }
}

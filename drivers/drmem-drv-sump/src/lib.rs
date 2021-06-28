// Copyright (c) 2020-2021, Richard M Neswold, Jr.
// All rights reserved.
//
// Redistribution and use in source and binary forms, with or without
// modification, are permitted provided that the following conditions are
// met:
//
// 1. Redistributions of source code must retain the above copyright
//    notice, this list of conditions and the following disclaimer.
//
// 2. Redistributions in binary form must reproduce the above copyright
//    notice, this list of conditions and the following disclaimer in the
//    documentation and/or other materials provided with the distribution.
//
// 3. Neither the name of the copyright holder nor the names of its
//    contributors may be used to endorse or promote products derived
//    from this software without specific prior written permission.
//
// THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS
// "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT
// LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR
// A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT
// HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
// SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT
// LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE,
// DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY
// THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
// (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
// OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use std::net::{Ipv4Addr, SocketAddrV4};
use async_trait::async_trait;
use tokio::{ io::{ self, AsyncReadExt },
	     net::{ TcpStream, tcp::{ OwnedReadHalf, OwnedWriteHalf } } };
use tracing::{ error, info, warn, debug };
use drmem_db_redis::RedisContext;
use drmem_config::DriverConfig;
use drmem_api::{ types, framework, DbContext, driver::Driver, device::Device,
		 Result };

const DESCRIPTION: &'static str = r#"
This driver monitors the state of a sump pump and updates a set of
devices based on its behavior.

This driver communicates, via TCP, with a RaspberryPi that's
monitoring a GPIO pin for state changes of the sump pump. It sends a
12-byte packet whenever the state changes. The first 8 bytes holds a
millisecond timestamp in big-endian format. The following 4 bytes
holds the new state.

With these packets, the driver can use the timestamps to compute duty
cycles and incoming flows rates for the sump pit.

# Configuration

Three parameters are used to configure the driver:

- `addr` is a string containing the host name, or IP address, of the
  machine that's actually monitoring the sump pump.
- `port` is an integer containing the port number of the service on
  the remote machine.
- `gpm` is an integer that repesents the gallons-per-minute capacity
  of the sump pump.

# Devices

The driver creates these devices:

| Base Name | Type | Units | Comment                                                   |
|-----------|------|-------|-----------------------------------------------------------|
| `service` | bool |       | Set to `true` when communicating with the remote service. |
| `state`   | bool |       | Set to `true` when the pump is running.                   |
| `duty`    | f64  | %     | Indicates duty cycle of last cycle.                       |
| `in-flow` | f64  | gpm   | Indicates the in-flow rate for the last cycle.            |

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
    tx: OwnedWriteHalf,
    state: State,
    d_service: Device<bool>,
    d_state: Device<bool>,
    d_duty: Device<f64>,
    d_inflow: Device<f64>,
    ctxt: RedisContext,
}

impl Sump {
    pub async fn new(mut ctxt: RedisContext,
		     cfg: &DriverConfig,
		     req_core: framework::DriverRequestChan) -> Result<Self> {
	// Validate the configuration.

        let addr = match cfg.get("addr") {
            Some(addr) => addr,
            None => return Err(types::DrMemError::BadConfig),
        };

        let port = match cfg.get("port") {
            Some(port) => port,
            None => return Err(types::DrMemError::BadConfig),
        };

        // Define the devices managed by this driver.

        let d_service: Device<bool> = ctxt
            .define_device(
                "service",
                "status of connection to sump pump module",
                None,
            )
            .await?;

        let d_state: Device<bool> = ctxt
            .define_device("state", "active state of sump pump", None)
            .await?;

        let d_duty: Device<f64> = ctxt
            .define_device(
                "duty",
                "sump pump on-time percentage during last cycle",
                Some(String::from("%")),
            )
            .await?;

        let d_inflow: Device<f64> = ctxt
            .define_device(
                "in-flow",
                "sump pit fill rate during last cycle",
                Some(String::from("gpm")),
            )
            .await?;

        let addr = SocketAddrV4::new(Ipv4Addr::new(192, 168, 1, 101), 10_000);
        let s = TcpStream::connect(addr).await.map_err(|_| {
            types::DrMemError::MissingPeer(String::from("sump pump"))
        })?;

        // Unfortunately, we have to hang onto the xmt handle

        let (rx, tx) = s.into_split();

        Ok(Sump {
            rx,
            tx,
            state: State::Unknown,
            ctxt,
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

        return Ok((stamp, value != 0));
    }
}

#[async_trait]
impl Driver for Sump {
    async fn run(&mut self) -> Result<()> {
	self.ctxt.write_values(&[self.d_service.set(true)]).await?;

        loop {
            match self.get_reading().await {
                Ok((stamp, true)) => {
                    if self.state.on_event(stamp) {
                        self.ctxt
                            .write_values(&[self.d_state.set(true)])
                            .await?;
                    }
                }

                Ok((stamp, false)) => {
                    if let Some((duty, in_flow)) = self.state.off_event(stamp) {
                        info!("duty: {}%, in flow: {} gpm", duty, in_flow);

                        self.ctxt
                            .write_values(&[
                                self.d_state.set(false),
                                self.d_duty.set(duty),
                                self.d_inflow.set(in_flow),
                            ])
                            .await?;
                    }
                }

                Err(e) => {
                    error!("couldn't read sump state -- {:?}", e);
                    self.ctxt
                        .write_values(&[
                            self.d_service.set(false),
                            self.d_state.set(false),
                        ])
                        .await?;
                    break Err(types::DrMemError::OperationError);
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

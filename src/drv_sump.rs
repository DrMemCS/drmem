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

use std::time::Duration;
use tokio::{ io::{ self, AsyncReadExt },
	     net::{ TcpStream, tcp::ReadHalf },
	     time::delay_for,
	     sync::mpsc };
use palette::{ named, Srgb, Yxy };
use tracing::{ error, info, warn };

use crate::config;
use crate::hue;
use crate::device::db::Context;

// The sump pump monitor uses a state machine to decide when to
// calculate the duty cycle and in-flow.

#[derive(Debug)]
enum State {
    Unknown,
    Off { off_time: u64 },
    On { off_time: u64, on_time: u64 }
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

    pub fn to_off(&mut self, stamp: u64) -> Option<(f64, f64)> {
	match *self {
	    State::Unknown => {
		info!("sync-ed with OFF state");
		*self = State::Off { off_time: stamp };
		None
	    },

	    State::Off { .. } => {
		warn!("ignoring duplicate OFF event");
		None
	    },

	    State::On { off_time, on_time } => {
		// The time stamp of the OFF time should come after
		// the ON time. If it isn't, the sump pump task has a
		// problem (i.e. system time was adjusted.) We can't
		// give a decent computation, so just go into the DOWN
		// state.

		if on_time >= stamp {
		    warn!("timestamp for OFF event is {} ms ahead of ON event",
			  on_time - stamp);
		    *self = State::Off { off_time: stamp };
		    return None
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
		    warn!("ignoring short ON time -- {:.3} s", on_time);
		    None
		}
	    }
	}
    }

    // This method is called when updating the state with an on
    // event. The timestamp of the on event needs to be provided. If
    // the on event actually caused a state change, `true` is
    // returned.

    pub fn to_on(&mut self, stamp: u64) -> bool {
	match *self {
	    State::Unknown => false,

	    State::Off { off_time } => {
		// Make sure the ON time occurred *after* the OFF
		// time. This is necessary for the computations to
		// yield valid results.

		if stamp > off_time {
		    *self = State::On { off_time, on_time: stamp };
		    true
		} else {
		    warn!("timestamp for ON event is {} ms ahead of OFF event",
			  off_time - stamp);
		    false
		}
	    },

	    State::On { .. } => {
		warn!("ignoring duplicate ON event");
		false
	    }
	}
    }
}

// This function reads the next frame from the sump pump process. It
// either returns `Ok()` with the two fields' values or `Err()` if a
// socket error occurred.

async fn get_reading(rx: &mut ReadHalf<'_>) -> io::Result<(u64, bool)> {
    let stamp = rx.read_u64().await?;
    let value = rx.read_u32().await?;

    return Ok((stamp, value != 0))
}

async fn lamp_alert(tx: &mut mpsc::Sender<hue::Program>) -> () {
    let r : Yxy = Srgb::<f32>::from_format(named::RED).into_linear().into();
    let prog =
	vec![hue::HueCommands::On { light: 5, bri: 255, color: Some(r) },
	     hue::HueCommands::On { light: 8, bri: 255, color: Some(r) }];

    if let Err(e) = tx.send(prog).await {
	warn!("sump alert returned error: {:?}", e)
    }
}

async fn lamp_off(tx: &mut mpsc::Sender<hue::Program>, duty: f64) -> () {
    if duty >= 10.0 {
	let b : Yxy = Srgb::<f32>::from_format(named::BLUE).into_linear().into();
	let cc = if duty < 30.0 { named::YELLOW } else { named::RED };
	let c : Yxy = Srgb::<f32>::from_format(cc).into_linear().into();
	let prog =
	    vec![hue::HueCommands::Off { light: 5 },
		 hue::HueCommands::Off { light: 8 },
		 hue::HueCommands::On { light: 5, bri: 255, color: Some(b) },
		 hue::HueCommands::On { light: 8, bri: 255, color: Some(b) },
		 hue::HueCommands::Pause { len: Duration::from_millis(1_000) },
		 hue::HueCommands::On { light: 5, bri: 255, color: Some(c) },
		 hue::HueCommands::On { light: 8, bri: 255, color: Some(c) },
		 hue::HueCommands::Pause { len: Duration::from_millis(4_000) },
		 hue::HueCommands::Off { light: 5 },
		 hue::HueCommands::Off { light: 8 }];

	if let Err(e) = tx.send(prog).await {
	    warn!("sump off returned error: {:?}", e)
	}
    }
}

// Returns an async function which monitors the sump pump, computes
// interesting, related values, and writes these details to associated
// devices' history.

pub async fn monitor(cfg: &config::Config,
		     mut tx: mpsc::Sender<hue::Program>)
		     -> redis::RedisResult<()> {
    use std::net::{Ipv4Addr, SocketAddrV4};

    let mut ctxt = Context::create("sump", cfg, None, None).await?;

    let d_service =
	ctxt.define_device::<bool>("service",
				   "status of connection to sump pump module",
				   None).await?;

    let d_state =
	ctxt.define_device::<bool>("state",
				   "active state of sump pump",
				   None).await?;

    let d_duty =
	ctxt.define_device::<f64>("duty",
				  "sump pump on-time percentage during last cycle",
				  Some(String::from("%"))).await?;

    let d_inflow =
	ctxt.define_device::<f64>("in-flow",
				  "sump pit fill rate during last cycle",
				  Some(String::from("gpm"))).await?;

    let addr = SocketAddrV4::new(Ipv4Addr::new(192, 168, 1, 101), 10_000);

    loop {
	match TcpStream::connect(addr).await {
	    Ok(mut s) => {
		let mut state = State::Unknown;
		let (mut rx, _) = s.split();

		ctxt.write_values(&[d_service.set(true)]).await?;

		loop {
		    match get_reading(&mut rx).await {
			Ok((stamp, true)) =>
			    if state.to_on(stamp) {
				ctxt.write_values(&[d_state.set(true)])
				    .await?;
			    },
			Ok((stamp, false)) => {
			    if let Some((duty, in_flow)) = state.to_off(stamp) {
				info!("duty: {}%, in flow: {} gpm", duty,
				      in_flow);

				ctxt.write_values(&[d_state.set(false),
						    d_duty.set(duty),
						    d_inflow.set(in_flow)])
				    .await?;
				lamp_off(&mut tx, duty).await
			    }
			},
			Err(e) => {
			    error!("couldn't read sump state -- {:?}", e);
			    ctxt.write_values(&[d_service.set(false),
						d_state.set(false)])
				.await?;
			    lamp_alert(&mut tx).await;
			    break;
			}
		    }
		    info!("state: {:?}", state);
		}
	    },
	    Err(e) => {
		ctxt.write_values(&[d_service.set(false),
				    d_state.set(false)]).await?;
		lamp_alert(&mut tx).await;
		error!("couldn't connect to pump process -- {:?}", e)
	    }
	}

	// Delay for 10 seconds before retrying.

	delay_for(Duration::from_millis(10_000)).await;
    }
}

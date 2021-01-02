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
use huebridge::{ HueResult, HueError, HueErrorKind, bridge::Bridge,
		 commandlight::CommandLight };
use tracing::warn;
use tokio::{ task::{ self, JoinHandle },
	     sync::mpsc,
	     time::delay_for };
use palette::Yxy;

use crate::config;

#[derive(Debug)]
pub enum HueCommands {
    Off { light: usize },
    On { light: usize, bri: u8, color: Option<Yxy> },
    Pause { len: Duration }
}

pub type Program = Vec<HueCommands>;

async fn update_light(bridge: Bridge,
		      light: usize,
		      state: CommandLight) -> Bridge {
    task::spawn_blocking(move || {
	if let Err(e) = bridge.set_light_state(light, &state) {
	    warn!("light {} : error {:?} sending state {:?}", light, e, state)
	}
	bridge
    }).await.unwrap()
}

async fn run(mut bridge: Bridge, program: Program) -> Bridge {
    for cmd in program {
	match cmd {
	    HueCommands::On { light, bri, color } => {
		let mut state = CommandLight::default().with_bri(bri).on();

		if let Some(c) = color {
		    state = state.with_xy(c.x, c.y)
		}
		bridge = update_light(bridge, light, state).await
	    },
	    HueCommands::Off { light } => {
		let state = CommandLight::default().off();

		bridge = update_light(bridge, light, state).await
	    },
	    HueCommands::Pause { len } => delay_for(len).await
	}
    }
    bridge
}

async fn controller(addr: String, key: String,
		    mut rx: mpsc::Receiver<Program>) -> () {
    let mut bridge = task::spawn_blocking(|| {
	Bridge::default()
	    .with_address(addr)
	    .with_username(key)
    }).await.unwrap();

    while let Some(prog) = rx.recv().await {
	bridge = run(bridge, prog).await
    }
}

pub fn manager(cfg: &config::Config)
	       -> HueResult<(mpsc::Sender<Program>, JoinHandle<()>)> {
    if let Some(key) = &cfg.hue_bridge.key {
	let (tx, rx) = mpsc::channel(20);

	Ok((tx, task::spawn(controller(cfg.hue_bridge.addr.clone(),
				       String::from(key), rx))))
    } else {
	Err(HueError::from_kind(HueErrorKind::Msg(String::from("no key defined"))))
    }
}

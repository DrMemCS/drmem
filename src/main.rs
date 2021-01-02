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
use tracing::{warn};
use palette::{named, Srgb, Yxy};

mod config;
mod data;
mod driver;
mod hue;
mod drv_sump;

#[tokio::main]
async fn main() -> redis::RedisResult<()> {
    if let Some(cfg) = config::get().await {

	// Initialize the log system. The max log level is determined
	// by the user (either through the config file or the command
	// line.)

	let subscriber = tracing_subscriber::fmt()
	    .with_max_level(cfg.get_log_level())
	    .finish();

	tracing::subscriber::set_global_default(subscriber)
	    .expect("Unable to set global default subscriber");

	match hue::manager(&cfg) {
	    Ok((mut tx, _join)) => {
		use hue::HueCommands;

		let c1 : Yxy = Srgb::<f32>::from_format(named::RED)
		    .into_linear().into();
		let c2 : Yxy = Srgb::<f32>::from_format(named::WHITE)
		    .into_linear().into();
		let c3 : Yxy = Srgb::<f32>::from_format(named::BLUE)
		    .into_linear().into();

		let prog =
		    vec![HueCommands::On { light: 5, bri: 255, color: Some(c1) },
			 HueCommands::On { light: 8, bri: 255, color: Some(c1) },
			 HueCommands::Pause { len: Duration::from_millis(1_000) },
			 HueCommands::On { light: 5, bri: 255, color: Some(c2) },
			 HueCommands::On { light: 8, bri: 255, color: Some(c2) },
			 HueCommands::Pause { len: Duration::from_millis(1_000) },
			 HueCommands::On { light: 5, bri: 255, color: Some(c3) },
			 HueCommands::On { light: 8, bri: 255, color: Some(c3) },
			 HueCommands::Pause { len: Duration::from_millis(1_000) },
			 HueCommands::Off { light: 5 },
			 HueCommands::Off { light: 8 }];

		if let Err(e) = tx.send(prog).await {
		    warn!("tx returned {:?}", e);
		}
		if let Err(e) = drv_sump::monitor(&cfg, tx).await {
		    warn!("monitor returned: {:?}", e);
		}
	    }
	    Err(e) =>
		warn!("no hue manager: {:?}", e)
	}
    }
    Ok(())
}

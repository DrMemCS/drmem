// Copyright (c) 2020-2022, Richard M Neswold, Jr.
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

use tracing::{warn};
use tokio::pin;
use drmem_api::{ driver::Driver, Result };
use drmem_config::DriverConfig;
use drmem_db_redis;

mod core;

#[cfg(graphql)]
mod httpd;

#[tokio::main]
async fn main() -> Result<()> {
    if let Some(cfg) = drmem_config::get().await {

	// Initialize the log system. The max log level is determined
	// by the user (either through the config file or the command
	// line.)

	let subscriber = tracing_subscriber::fmt()
	    .with_max_level(cfg.get_log_level())
	    .finish();

	tracing::subscriber::set_global_default(subscriber)
	    .expect("Unable to set global default subscriber");

	let (tx_drv_req, core_task) = core::start();

	let ctxt = drmem_db_redis::RedisContext::new("sump",
						     &cfg.get_backend(),
						     None, None).await?;

	let mut drv_pump =
	    drmem_drv_sump::Sump::new(ctxt, &DriverConfig::new(),
				      tx_drv_req).await?;
	let drv_pump = drv_pump.run();
	pin!(drv_pump);

	#[cfg(not(graphql))]
	{
	    tokio::select! {
		Err(e) = core_task => {
		    warn!("core returned: {:?}", e);
		}
		Err(e) = drv_pump => {
		    warn!("monitor returned: {:?}", e);
		}
	    }
	}
	#[cfg(graphql)]
	{
	    let svr_httpd = httpd::server();
	    pin!(svr_httpd);

	    tokio::select! {
		Err(e) = core_task => {
		    warn!("core returned: {:?}", e);
		}
		Err(e) = drv_pump => {
		    warn!("monitor returned: {:?}", e);
		}
		Err(e) = svr_httpd => {
		    warn!("httpd returned: {:?}", e);
		}
	    }
	}
    }
    Ok(())
}

use std::time::Duration;
use tracing::{warn};
use palette::{Srgb, Yxy};
use palette::named;

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

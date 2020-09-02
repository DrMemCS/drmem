use std::time::Duration;
use huebridge::{HueResult, bridge::Bridge, commandlight::CommandLight};
use tracing::warn;
use tokio::task::{self, JoinHandle};
use tokio::sync::mpsc;
use tokio::time::delay_for;
use palette::Yxy;

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

async fn controller(mut rx: mpsc::Receiver<Program>) -> () {
    let mut bridge = task::spawn_blocking(|| {
	let username = "????".to_string();

	Bridge::default()
	    .with_address("????".to_string())
	    .with_username(username)
    }).await.unwrap();

    while let Some(prog) = rx.recv().await {
	bridge = run(bridge, prog).await
    }
}

pub fn manager() -> HueResult<(mpsc::Sender<Program>, JoinHandle<()>)> {
    let (tx, rx) = mpsc::channel(20);

    Ok((tx, task::spawn(controller(rx))))
}

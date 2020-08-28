use std::net::{Ipv4Addr, SocketAddrV4};
use std::time::Duration;
use tokio::net::{TcpStream, tcp::ReadHalf};
use tokio::io::{self, AsyncReadExt};
use tokio::time::delay_for;
use tracing::{error, info, warn};
use tracing_subscriber::FmtSubscriber;

#[derive(Debug)]
enum State {
    Unknown,
    Off { off_time: u64 },
    On { off_time: u64, on_time: u64 }
}

impl State {
    pub fn to_off(&mut self, stamp: u64) -> Option<(f64, f64)> {
	match *self {
	    State::Unknown => {
		info!("sync-ed with OFF state");
		*self = State::Off { off_time: stamp };
		None
	    },
	    State::Off { off_time: _ } => {
		warn!("ignoring duplicate OFF event");
		None
	    },
	    State::On { off_time, on_time } => {
		let on_time = ((stamp - on_time) as f64) / 1000.0;
		let off_time = ((stamp - off_time) as f64) / 1000.0;
		let duty = (on_time * 100.0 / off_time).round();
		let in_flow = (2680.0 * duty / 60.0).round() / 100.0;

		*self = State::Off { off_time: stamp };
		Some((duty, in_flow))
	    }
	}
    }

    pub fn to_on(&mut self, stamp: u64) -> bool {
	match *self {
	    State::Unknown => false,
	    State::Off { off_time } => {
		*self = State::On { off_time, on_time: stamp };
		true
	    },
	    State::On { .. } => {
		warn!("ignoring duplicate ON event");
		false
	    }
	}
    }
}

async fn get_reading(rx: &mut ReadHalf<'_>) -> io::Result<(u64, bool)> {
    let stamp = rx.read_u64().await?;
    let value = rx.read_u32().await?;

    return Ok((stamp, value != 0))
}

async fn set_service_state(con: &mut redis::aio::Connection,
			   value: &str) -> redis::RedisResult<()> {
    redis::Cmd::xadd("sump.service", "*", &[("value", value)])
	.query_async(con).await
}

async fn mk_redis_conn() -> redis::RedisResult<redis::aio::Connection> {
    let addr = redis::ConnectionAddr::Tcp("127.0.0.1".to_string(), 6379);
    let info = redis::ConnectionInfo { addr: Box::new(addr), db: 0, username: None,
				       passwd: None };

    redis::aio::connect_tokio(&info).await
}

async fn monitor() -> redis::RedisResult<()> {
    let mut con = mk_redis_conn().await?;
    let addr = SocketAddrV4::new(Ipv4Addr::new(192, 168, 1, 101), 10_000);

    loop {
	let mut state = State::Unknown;

	match TcpStream::connect(addr).await {
	    Ok(mut s) => {
		let (mut rx, _) = s.split();
		set_service_state(&mut con, "up").await?;
		loop {
		    match get_reading(&mut rx).await {
			Ok((stamp, true)) => {
			    if state.to_on(stamp) {
				let _ : () =
				    redis::Cmd::xadd("sump.state",
						     stamp,
						     &[("value", "on")])
				    .query_async(&mut con).await?;
			    }
			},
			Ok((stamp, false)) => {
			    if let Some((duty, in_flow)) = state.to_off(stamp) {
				info!("duty: {}%, in flow: {} gpm", duty, in_flow);

				let _ : () = redis::pipe()
				    .atomic()
				    .cmd("XADD").arg("sump.state").arg(stamp)
				    .arg("value").arg("off").ignore()
				    .cmd("XADD").arg("sump.duty").arg(stamp)
				    .arg("value").arg(duty)
				    .arg("units").arg("%").ignore()
				    .cmd("XADD").arg("sump.inflow").arg(stamp)
				    .arg("value").arg(in_flow)
				    .arg("units").arg("gpm")
				    .query_async(&mut con).await?;
			    }
			},
			Err(e) => {
			    error!("couldn't read sump state -- {:?}", e);
			    set_service_state(&mut con, "crash").await?;
			    break;
			}
		    }
		    info!("state: {:?}", state);
		}
	    },
	    Err(e) => {
		set_service_state(&mut con, "down").await?;
		error!("couldn't connect to pump process -- {:?}", e)
	    }
	}

	delay_for(Duration::from_millis(10_000)).await;
    }
}

#[tokio::main]
async fn main() -> redis::RedisResult<()> {
    tracing::subscriber::set_global_default(FmtSubscriber::new())
        .expect("Unable to set global default subscriber");

    monitor().await
}

use std::sync::Arc;
use tokio::{sync::broadcast, time};
use tracing::{info, info_span, warn};
use tracing_futures::Instrument;

pub type Info = Arc<(
    chrono::DateTime<chrono::Utc>,
    chrono::DateTime<chrono::Local>,
)>;

fn initial_delay() -> u64 {
    let now = chrono::Utc::now();
    let extra = now.timestamp_subsec_millis();

    ((10020 - extra) % 1000) as u64
}

async fn run(tx: broadcast::Sender<Info>) {
    let mut interval = time::interval_at(
        time::Instant::now() + time::Duration::from_millis(initial_delay()),
        time::Duration::from_secs(1),
    );

    info!("starting time-of-day task");

    while tx
        .send(Arc::new((chrono::Utc::now(), chrono::Local::now())))
        .is_ok()
    {
        let _ = interval.tick().await;
    }
    warn!("no remaining clients ... terminating");
}

pub fn create_task() -> (broadcast::Sender<Info>, broadcast::Receiver<Info>) {
    let (tx, rx) = broadcast::channel(1);

    tokio::spawn(run(tx.clone())).instrument(info_span!("tod"));

    (tx, rx)
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_tod() {}
}

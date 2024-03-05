use chrono::{Datelike, Timelike};
use core::pin::Pin;
use core::task::{Context, Poll};
use std::sync::Arc;
use tokio::{sync::broadcast, time};
use tokio_stream::{wrappers::BroadcastStream, Stream};
use tracing::{info, info_span, warn};
use tracing_futures::Instrument;

// Information related to time-of-day. We keep both UTC and local time
// so clients don't have to convert between the time zones. It is
// stored in an `Arc` so it can be cheaply sent and received over a
// broadcast channel.

pub type Info = Arc<(
    chrono::DateTime<chrono::Utc>,
    chrono::DateTime<chrono::Local>,
)>;

// Each variant of this enumeration selects a field of a Date/Time
// type. They are defined in order of shortest time span to largest so
// they can be compared. This enumeration is used as an optimization
// in the state of a logic block, for instance, to select which time
// field should be checked for changes. Doing so prevents the task
// from recalculating all its expressions every second when it really
// only needed to do it once an hour.

#[derive(Eq, PartialEq, Ord, PartialOrd)]
pub enum TimeField {
    Second,
    Minute,
    Hour,
    Day,
    Month,
    Year,
    Forever,
}

pub struct TimeFilter {
    field: TimeField,
    prev: Option<Info>,
    inner: BroadcastStream<Info>,
}

impl TimeFilter {
    fn changed(&self, curr: &Info) -> bool {
        if let Some(ref v) = self.prev {
            match self.field {
                TimeField::Second => {
                    v.0.second() != curr.0.second()
                        || v.1.second() != curr.1.second()
                }
                TimeField::Minute => {
                    v.0.minute() != curr.0.minute()
                        || v.1.minute() != curr.1.minute()
                }
                TimeField::Hour => {
                    v.0.hour() != curr.0.hour() || v.1.hour() != curr.1.hour()
                }
                TimeField::Day => {
                    v.0.day() != curr.0.day() || v.1.day() != curr.1.day()
                }
                TimeField::Month => {
                    v.0.month() != curr.0.month()
                        || v.1.month() != curr.1.month()
                }
                TimeField::Year => {
                    v.0.year() != curr.0.year() || v.1.year() != curr.1.year()
                }
                TimeField::Forever => false,
            }
        } else {
            true
        }
    }
}

// Make TimeFilter able to be used as a BroadcastStream wrapper.

impl Stream for TimeFilter {
    type Item = Info;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        loop {
            match Pin::new(&mut self.inner).poll_next(cx) {
                // If the stream is done, or doesn't have a new value,
                // pass the return value to the caller.
                Poll::Pending => return Poll::Pending,
                Poll::Ready(None) => return Poll::Ready(None),

                // If we got a value, check to see if it has changed
                // enough from the previous value to return it. If
                // not, loop for the next value.
                Poll::Ready(Some(Ok(tod))) => {
                    if self.changed(&tod) {
                        self.prev = Some(tod.clone());
                        return Poll::Ready(Some(tod));
                    }
                }

                // The only error we can get is that we haven't read
                // the stream fast enough and that elements have been
                // dropped. In this case, we clear out the previous
                // value so that the next value always gets returned.
                Poll::Ready(Some(Err(_))) => self.prev = None,
            }
        }
    }
}

pub fn time_filter(
    stream: BroadcastStream<Info>,
    field: TimeField,
) -> TimeFilter {
    TimeFilter {
        inner: stream,
        field,
        prev: None,
    }
}

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
    let tx_copy = tx.clone();

    tokio::spawn(
        async move { run(tx_copy).await }.instrument(info_span!("tod")),
    );

    (tx, rx)
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_tod() {}
}

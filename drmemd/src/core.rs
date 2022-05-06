use drmem_api::{driver, types::Error, Result, Store};
use tokio::{
    sync::{mpsc, oneshot},
    task::JoinHandle,
};
use tracing::{info, info_span, warn};
use tracing_futures::Instrument;

// If the simple-backend feature is enabled, we define the `store`
// module and fill it.

#[cfg(feature = "simple-backend")]
mod store {
    pub use drmem_db_simple::open;
}

/// Holds the state of the core task in the framework.
///
/// The core task starts-up the necessary drivers and maintains a
/// table of active devices. Drivers and client communicate with the
/// core task through channels.
struct State {
    backend: Box<dyn Store + Send>,
}

impl State {
    /// Creates an initialized state for the core task.
    async fn create() -> Result<Self> {
        let backend = Box::new(store::open().await?);

        Ok(State { backend })
    }

    fn send_reply<T>(
        dev_name: &str, rpy_chan: oneshot::Sender<Result<T>>, val: Result<T>,
    ) {
        let result =
            val.map_err(|_| Error::DeviceDefined(String::from(dev_name)));

        if rpy_chan.send(result).is_err() {
            warn!("driver exited before a reply could be sent")
        }
    }

    async fn handle_driver_request(&mut self, req: driver::Request) {
        match req {
            driver::Request::AddReadonlyDevice {
                ref dev_name,
                rpy_chan,
            } => {
                let result =
                    self.backend.register_read_only_device(dev_name).await;

                State::send_reply(dev_name, rpy_chan, result)
            }

            driver::Request::AddReadWriteDevice {
                ref dev_name,
                rpy_chan,
            } => {
                let result =
                    self.backend.register_read_write_device(dev_name).await;

                State::send_reply(dev_name, rpy_chan, result)
            }
        }
    }

    /// Captures the State and runs as a async task using it as its
    /// mutable state. Normally it is run as a background task using
    /// `task::spawn`.
    async fn run(
        mut self, mut rx_drv_req: mpsc::Receiver<driver::Request>,
    ) -> Result<()> {
        loop {
            if let Some(req) = rx_drv_req.recv().await {
                self.handle_driver_request(req).await
            } else {
                warn!("no active drivers left ... exiting");
                return Ok(());
            }
        }
    }
}

pub async fn start(
) -> Result<(mpsc::Sender<driver::Request>, JoinHandle<Result<()>>)> {
    // Create a channel that drivers can use to make requests to the
    // framework. This task will hang onto the Receiver end and each
    // driver will get a .clone() of the transmit handle.

    let (tx_drv_req, rx_drv_req) = mpsc::channel(10);

    Ok((
        tx_drv_req,
        tokio::spawn(async {
            let state = State::create().await?;

            state
                .run(rx_drv_req)
                .instrument(info_span!("driver_manager"))
                .await
        }),
    ))
}

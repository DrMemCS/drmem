use drmem_api::{client, driver, types::Error, Result, Store};
use drmem_config::{backend, Config};
use tokio::{sync::mpsc, task::JoinHandle};
use tracing::{info, info_span, warn};
use tracing_futures::Instrument;

// Define a `store` module that pulls in the appropriate backend.

#[cfg(not(feature = "redis-backend"))]
mod store {
    pub use drmem_db_simple::open;
}

#[cfg(feature = "redis-backend")]
mod store {
    pub use drmem_db_redis::open;
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
    async fn create(cfg: backend::Config) -> Result<Self> {
        let backend = Box::new(store::open(&cfg).await?);

        Ok(State { backend })
    }

    /// Handles incoming requests and returns a reply.
    async fn handle_driver_request(&mut self, req: driver::Request) {
        match req {
            driver::Request::AddReadonlyDevice {
                ref driver_name,
                ref dev_name,
                ref dev_units,
                rpy_chan,
            } => {
                let result = self
                    .backend
                    .register_read_only_device(driver_name, dev_name, dev_units)
                    .await
                    .map_err(|_| Error::DeviceDefined(format!("{}", dev_name)));

                if rpy_chan.send(result).is_err() {
                    warn!("driver exited before a reply could be sent")
                }
            }

            driver::Request::AddReadWriteDevice {
                ref driver_name,
                ref dev_name,
                ref dev_units,
                rpy_chan,
            } => {
                let result = self
                    .backend
                    .register_read_write_device(
                        driver_name,
                        dev_name,
                        dev_units,
                    )
                    .await
                    .map_err(|_| Error::DeviceDefined(format!("{}", dev_name)));

                if rpy_chan.send(result).is_err() {
                    warn!("driver exited before a reply could be sent")
                }
            }
        }
    }

    async fn handle_client_request(&mut self, req: client::Request) {
        match req {
            client::Request::QueryDeviceInfo {
                ref pattern,
                rpy_chan,
            } => {
                let fut = self.backend.get_device_info(pattern);
                let result = fut.await;

                if rpy_chan.send(result.unwrap()).is_err() {
                    warn!("driver exited before a reply could be sent")
                }
            }
        }
    }

    /// Captures the State and runs as a async task using it as its
    /// mutable state. Normally it is run as a background task using
    /// `task::spawn`.
    async fn run(
        mut self, mut rx_drv_req: mpsc::Receiver<driver::Request>,
        mut rx_clnt_req: mpsc::Receiver<client::Request>,
    ) -> Result<()> {
        info!("starting");
        loop {
            tokio::select! {
            Some(req) = rx_drv_req.recv() =>
                        self.handle_driver_request(req).await,
            Some(req) = rx_clnt_req.recv() =>
                        self.handle_client_request(req).await,
            else => break
                }
        }
        warn!("no drivers or clients left");
        Ok(())
    }
}

/// Starts the core task. Returns an `mpsc::Sender<>` handle so other
/// tasks can send requests to it.

pub async fn start(
    cfg: &Config,
) -> Result<(
    mpsc::Sender<driver::Request>,
    client::RequestChan,
    JoinHandle<Result<()>>,
)> {
    // Create a channel that drivers can use to make requests to the
    // framework. This task will hang onto the Receiver end and each
    // driver will get a .clone() of the transmit handle.

    let (tx_drv_req, rx_drv_req) = mpsc::channel(10);
    let (tx_clnt_req, rx_clnt_req) = mpsc::channel(10);
    let be_cfg = cfg.get_backend().clone();

    Ok((
        tx_drv_req,
        client::RequestChan::new(tx_clnt_req),
        tokio::spawn(async {
            let state = State::create(be_cfg).await?;

            state
                .run(rx_drv_req, rx_clnt_req)
                .instrument(info_span!("driver_manager"))
                .await
        }),
    ))
}

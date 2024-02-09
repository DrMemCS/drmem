use super::store;
use drmem_api::{client, driver, Error, Result, Store};
use std::convert::Infallible;
use tokio::{sync::mpsc, task::JoinHandle};
use tracing::{info, info_span, warn};
use tracing_futures::Instrument;

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
    async fn create(cfg: store::config::Config) -> Result<Self> {
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
                max_history,
                rpy_chan,
            } => {
                let result = self
                    .backend
                    .register_read_only_device(
                        driver_name,
                        dev_name,
                        dev_units.as_ref(),
                        max_history,
                    )
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
                max_history,
                rpy_chan,
            } => {
                let result = self
                    .backend
                    .register_read_write_device(
                        driver_name,
                        dev_name,
                        dev_units.as_ref(),
                        max_history,
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
            client::Request::QueryDeviceInfo { pattern, rpy_chan } => {
                let result =
                    self.backend.get_device_info(pattern.as_deref()).await;

                if let Err(ref e) = result {
                    info!("get_device_info() returned '{}'", e);
                }

                if rpy_chan.send(result).is_err() {
                    warn!("client exited before a reply could be sent")
                }
            }

            client::Request::SetDevice {
                name,
                value,
                rpy_chan,
            } => {
                let fut = self.backend.set_device(name, value);

                if rpy_chan.send(fut.await).is_err() {
                    warn!("client exited before a reply could be sent")
                }
            }

            client::Request::GetSettingChan {
                name,
                _own,
                rpy_chan,
            } => {
                let fut = self.backend.get_setting_chan(name, _own);

                if rpy_chan.send(fut.await).is_err() {
                    warn!("client exited before a reply could be sent")
                }
            }

            client::Request::MonitorDevice {
                name,
                rpy_chan,
                start,
                end,
            } => {
                let fut = self.backend.monitor_device(name, start, end);

                if rpy_chan.send(fut.await).is_err() {
                    warn!("client exited before a reply could be sent")
                }
            }
        }
    }

    /// Captures the State and runs as a async task using it as its
    /// mutable state. Normally it is run as a background task using
    /// `task::spawn`.
    async fn run(
        mut self,
        mut rx_drv_req: mpsc::Receiver<driver::Request>,
        mut rx_clnt_req: mpsc::Receiver<client::Request>,
    ) -> Result<Infallible> {
        info!("starting");
        loop {
            #[rustfmt::skip]
            tokio::select! {
		Some(req) = rx_drv_req.recv() =>
                    self
		    .handle_driver_request(req)
		    .instrument(info_span!("driver_req"))
		    .await,
		Some(req) = rx_clnt_req.recv() =>
                    self
		    .handle_client_request(req)
		    .instrument(info_span!("client_req"))
		    .await,
		else => break
            }
        }

        const ERR_MSG: &str = "no drivers or clients left";

        warn!(ERR_MSG);
        Err(Error::MissingPeer(ERR_MSG.to_string()))
    }
}

/// Starts the core task. Returns an `mpsc::Sender<>` handle so other
/// tasks can send requests to it.

pub async fn start(
    cfg: &super::config::Config,
) -> Result<(
    mpsc::Sender<driver::Request>,
    client::RequestChan,
    JoinHandle<Result<Infallible>>,
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
                .instrument(info_span!("drmem"))
                .await
        }),
    ))
}

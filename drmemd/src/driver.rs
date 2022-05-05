use futures::future::Future;
use std::collections::HashMap;
use std::pin::Pin;
use tokio::task::JoinHandle;
use tracing::{error, info, warn};
//use tracing_futures::Instrument;
use drmem_api::{
    driver::{DriverConfig, RequestChan, API},
    Result,
};

type DriverInst = Box<dyn API + Send>;

type Factory = fn(
    DriverConfig,
    RequestChan,
) -> Pin<Box<dyn Future<Output = Result<DriverInst>> + Send>>;

pub struct Driver {
    pub driver_name: &'static str,
    pub summary: &'static str,
    pub description: &'static str,
    factory: Factory,
}

impl Driver {
    pub fn create(
        name: &'static str, summary: &'static str, description: &'static str,
        factory: Factory,
    ) -> Driver {
        Driver {
            driver_name: name,
            summary,
            description,
            factory,
        }
    }

    // Runs an instance of the driver using the provided configuration
    // parameters.

    pub fn run_instance(
        &self, cfg: DriverConfig, req_chan: RequestChan,
    ) -> JoinHandle<Result<()>> {
        let factory = self.factory;

        // Spawn a task that supervises the driver task. If the driver
        // panics, this supervisor "catches" it and reports a
        // problem. It then restarts the driver.

        tokio::spawn(async move {
            loop {
                info!("initializing driver");

                // Create an instance of the driver which uses the
                // configuration parameters.

                let mut instance =
                    factory(cfg.clone(), req_chan.clone()).await?;

                // Start the driver instance as a background task and
                // monitor the return value.

                match tokio::spawn(async move { instance.run().await }).await {
                    // This exit value means the driver exited
                    // intentionally. This shouldn't happen normally.
                    // If this happens, the supervisor exits which
                    // should shutdown the application.
                    Ok(Ok(())) => {
                        warn!("driver exited intentionally");
                        return Ok(());
                    }

                    // If the driver exits with an error, report it
                    // and restart the driver (after a delay.)
                    Ok(Err(e)) => warn!("driver exited due to error -- {}", e),

                    // If `spawn()` returns this value, the driver
                    // exited abnormally. Report it and restart the
                    // driver.
                    Err(e) => {
                        if e.is_panic() {
                            error!("driver panicked -- {}", e);
                        } else {
                            error!("driver was canceled -- {}", e);
                        }
                    }
                }

                // Delay before restarting the driver. This prevents
                // the system from being compute-bound if the driver
                // panics right away.

                tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
            }
        })
    }
}

pub type Table = HashMap<&'static str, Driver>;

pub fn load_table() -> Table {
    let mut table = HashMap::new();

    #[cfg(feature = "driver-sump")]
    {
        use drmem_drv_sump::Sump;

        table.insert(
            "sump",
            Driver::create(
                "sump",
                "driver to monitor a sump pump",
                "description",
                Sump::create_instance,
            ),
        );
    }

    table
}

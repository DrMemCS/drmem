use drmem_api::{
    driver::{DriverConfig, DriverType, RequestChan, API},
    Result,
};
use futures::future::Future;
use std::collections::HashMap;
use std::pin::Pin;
use tokio::task::JoinHandle;
use tracing::{error, field, info, info_span, warn};
use tracing_futures::Instrument;

type Factory = fn(
    DriverConfig,
    RequestChan,
) -> Pin<Box<dyn Future<Output = Result<DriverType>> + Send>>;

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

    async fn manage_instance(
        name: &'static str, factory: Factory, cfg: DriverConfig,
        req_chan: RequestChan,
    ) -> Result<()> {
        loop {
            // Create a Future that creates an instance of the driver
            // using the provided configuration parameters.

            let result = factory(cfg.clone(), req_chan.clone())
                .instrument(info_span!("init"));

            match result.await {
                Ok(mut instance) => {
                    // Start the driver instance as a background task
                    // and monitor the return value.

                    match tokio::spawn(async move {
                        instance
                            .run()
                            .instrument(info_span!(
                                "drvr",
                                name,
                                cfg = field::Empty
                            ))
                            .await
                    })
                    .await
                    {
                        // This exit value means the driver exited
                        // intentionally. This shouldn't happen
                        // normally. If this happens, the supervisor
                        // exits which should shutdown the
                        // application.
                        Ok(Ok(())) => {
                            warn!("driver exited intentionally");
                            return Ok(());
                        }

                        // If the driver exits with an error, report
                        // it and restart the driver (after a delay.)
                        Ok(Err(e)) => {
                            warn!("driver exited due to error -- {}", e)
                        }

                        // If `spawn()` returns this value, the driver
                        // exited abnormally. Report it and restart
                        // the driver.
                        Err(e) => error!("{}", e),
                    }
                }
                Err(e) => {
                    error!("init error -- {}", e)
                }
            }

            // Delay before restarting the driver. This prevents the
            // system from being compute-bound if the driver panics
            // right away.

            info!("restarting driver after a short delay");
            tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
        }
    }

    // Runs an instance of the driver using the provided configuration
    // parameters.

    pub fn run_instance(
        &self, cfg: DriverConfig, req_chan: RequestChan,
    ) -> JoinHandle<Result<()>> {
        // Spawn a task that supervises the driver task. If the driver
        // panics, this supervisor "catches" it and reports a problem.
        // It then restarts the driver.

        tokio::spawn(
            Driver::manage_instance(
                self.driver_name,
                self.factory,
                cfg,
                req_chan,
            )
            .instrument(info_span!("mngr", drvr = self.driver_name)),
        )
    }
}

pub type Table = HashMap<&'static str, Driver>;

pub fn load_table() -> Table {
    let mut table = HashMap::new();

    // Load the set-up for the NTP monitor.

    #[cfg(feature = "driver-ntp")]
    {
        use drmem_drv_ntp::NtpState;

        table.insert(
            "ntp",
            Driver::create(
                NtpState::NAME,
                NtpState::SUMMARY,
                NtpState::DESCRIPTION,
                <NtpState as API>::create_instance,
            ),
        );
    }

    // Load the set-up for the GPIO sump pump monitor.

    #[cfg(feature = "driver-sump")]
    {
        use drmem_drv_sump::Sump;

        table.insert(
            "sump",
            Driver::create(
                Sump::NAME,
                Sump::SUMMARY,
                Sump::DESCRIPTION,
                <Sump as API>::create_instance,
            ),
        );
    }

    // Load the set-up for the GPIO sump pump monitor.

    #[cfg(feature = "driver-weather-wu")]
    {
        use drmem_drv_weather_wu::State;

        table.insert(
            "weather",
            Driver::create(
                State::NAME,
                State::SUMMARY,
                State::DESCRIPTION,
                <State as API>::create_instance,
            ),
        );
    }

    table
}

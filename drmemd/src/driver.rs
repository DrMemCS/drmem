use drmem_api::{
    driver::{DriverConfig, DriverType, RequestChan, API},
    Result,
};
use futures::future::Future;
use std::collections::HashMap;
use std::{pin::Pin, sync::Arc};
use tokio::task::JoinHandle;
use tracing::{error, field, info, info_span, warn};
use tracing_futures::Instrument;

type Factory = fn(
    DriverConfig,
    RequestChan,
) -> Pin<Box<dyn Future<Output = Result<DriverType>> + Send>>;

pub struct Driver {
    pub summary: &'static str,
    pub description: &'static str,
    factory: Factory,
}

impl Driver {
    pub fn create(
        summary: &'static str, description: &'static str, factory: Factory,
    ) -> Driver {
        Driver {
            summary,
            description,
            factory,
        }
    }

    async fn manage_instance(
        name: String, factory: Factory, cfg: DriverConfig,
        req_chan: RequestChan,
    ) -> Result<()> {
        loop {
            // Create a Future that creates an instance of the driver
            // using the provided configuration parameters.

            let result = factory(cfg.clone(), req_chan.clone())
                .instrument(info_span!("init"));

            match result.await {
                Ok(mut instance) => {
                    let name = name.clone();

                    // Start the driver instance as a background task
                    // and monitor the return value.

                    match tokio::spawn(async move {
                        instance
                            .run()
                            .instrument(info_span!(
                                "drvr",
                                name = name.as_str(),
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
        &self, name: String, cfg: DriverConfig, req_chan: RequestChan,
    ) -> JoinHandle<Result<()>> {
        // Spawn a task that supervises the driver task. If the driver
        // panics, this supervisor "catches" it and reports a problem.
        // It then restarts the driver.

        tokio::spawn(
            Driver::manage_instance(name.clone(), self.factory, cfg, req_chan)
                .instrument(info_span!("mngr", drvr = name.as_str())),
        )
    }
}

#[derive(Clone)]
pub struct DriverDb(Arc<HashMap<&'static str, Driver>>);

impl DriverDb {
    pub fn create() -> DriverDb {
        let mut table = HashMap::new();

        {
            use drmem_drv_timer::Timer;

            table.insert(
                Timer::NAME,
                Driver::create(
                    Timer::SUMMARY,
                    Timer::DESCRIPTION,
                    <Timer as API>::create_instance,
                ),
            );
        }

        // Load the set-up for the NTP monitor.

        #[cfg(feature = "driver-ntp")]
        {
            use drmem_drv_ntp::NtpState;

            table.insert(
                NtpState::NAME,
                Driver::create(
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
                Sump::NAME,
                Driver::create(
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
                State::NAME,
                Driver::create(
                    State::SUMMARY,
                    State::DESCRIPTION,
                    <State as API>::create_instance,
                ),
            );
        }

        DriverDb(Arc::new(table))
    }

    pub fn get_driver(&self, key: &str) -> Option<&Driver> {
        self.0.get(key)
    }

    pub fn find(
        &self, key: &str,
    ) -> Option<(String, &'static str, &'static str)> {
        self.0
            .get(key)
            .map(|info| (key.to_string(), info.summary, info.description))
    }

    pub fn get_all(
        &self,
    ) -> impl Iterator<Item = (String, &'static str, &'static str)> + '_ {
        self.0.iter().map(
            |(
                k,
                Driver {
                    summary,
                    description,
                    ..
                },
            )| { (k.to_string(), *summary, *description) },
        )
    }
}

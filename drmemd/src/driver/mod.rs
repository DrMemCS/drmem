use drmem_api::{
    driver::{DriverConfig, DriverType, RequestChan, API},
    Result,
};
use futures::future::Future;
use std::collections::HashMap;
use std::{convert::Infallible, pin::Pin, sync::Arc};
use tokio::task::JoinHandle;
use tracing::{error, field, info, info_span};
use tracing_futures::Instrument;

mod drv_cycle;
mod drv_memory;
mod drv_timer;

type Factory = fn(
    &DriverConfig,
    RequestChan,
    Option<usize>,
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
        req_chan: RequestChan, max_history: Option<usize>,
    ) -> Result<Infallible> {
        loop {
            // Create a Future that creates an instance of the driver
            // using the provided configuration parameters.

            let result = factory(&cfg, req_chan.clone(), max_history)
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
                        Ok(_) => unreachable!(),

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
        &self, name: String, max_history: Option<usize>, cfg: DriverConfig,
        req_chan: RequestChan,
    ) -> JoinHandle<Result<Infallible>> {
        // Spawn a task that supervises the driver task. If the driver
        // panics, this supervisor "catches" it and reports a problem.
        // It then restarts the driver.

        tokio::spawn(
            Driver::manage_instance(
                name.clone(),
                self.factory,
                cfg,
                req_chan,
                max_history,
            )
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
            use drv_memory::Instance;

            table.insert(
                Instance::NAME,
                Driver::create(
                    Instance::SUMMARY,
                    Instance::DESCRIPTION,
                    <Instance as API>::create_instance,
                ),
            );
        }

        {
            use drv_timer::Instance;

            table.insert(
                Instance::NAME,
                Driver::create(
                    Instance::SUMMARY,
                    Instance::DESCRIPTION,
                    <Instance as API>::create_instance,
                ),
            );
        }

        {
            use drv_cycle::Instance;

            table.insert(
                Instance::NAME,
                Driver::create(
                    Instance::SUMMARY,
                    Instance::DESCRIPTION,
                    <Instance as API>::create_instance,
                ),
            );
        }

        // Load the set-up for the NTP monitor.

        #[cfg(feature = "driver-ntp")]
        {
            use drmem_drv_ntp::Instance;

            table.insert(
                Instance::NAME,
                Driver::create(
                    Instance::SUMMARY,
                    Instance::DESCRIPTION,
                    <Instance as API>::create_instance,
                ),
            );
        }

        // Load the set-up for the GPIO sump pump monitor.

        #[cfg(feature = "driver-sump")]
        {
            use drmem_drv_sump::Instance;

            table.insert(
                Instance::NAME,
                Driver::create(
                    Instance::SUMMARY,
                    Instance::DESCRIPTION,
                    <Instance as API>::create_instance,
                ),
            );
        }

        // Load the set-up for the GPIO sump pump monitor.

        #[cfg(feature = "driver-elgato")]
        {
            use drmem_drv_elgato::Instance;

            table.insert(
                Instance::NAME,
                Driver::create(
                    Instance::SUMMARY,
                    Instance::DESCRIPTION,
                    <Instance as API>::create_instance,
                ),
            );
        }

        // Load the set-up for the GPIO sump pump monitor.

        #[cfg(feature = "driver-weather-wu")]
        {
            use drmem_drv_weather_wu::Instance;

            table.insert(
                Instance::NAME,
                Driver::create(
                    Instance::SUMMARY,
                    Instance::DESCRIPTION,
                    <Instance as API>::create_instance,
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

use drmem_api::{driver, Result};
use futures::future::Future;
use std::collections::HashMap;
use std::{convert::Infallible, pin::Pin, sync::Arc};
use tokio::sync::Mutex;
use tracing::{error, field, info, info_span, warn};
use tracing_futures::Instrument;

mod drv_cycle;
mod drv_memory;
mod drv_timer;

pub type AsyncRet<T> = Pin<Box<dyn Future<Output = T> + Send>>;
pub type DriverRet = Result<Infallible>;

pub type Launcher = dyn Fn(
        String,
        driver::DriverConfig,
        driver::RequestChan,
        Option<usize>,
    ) -> AsyncRet<DriverRet>
    + Send
    + Sync;

pub type DriverInfo = (&'static str, &'static str, Box<Launcher>);

fn manage_instance<T: driver::API + Send + 'static>(
    name: String, cfg: driver::DriverConfig, req_chan: driver::RequestChan,
    max_history: Option<usize>,
) -> AsyncRet<DriverRet> {
    const START_DELAY: u64 = 5;
    const MAX_DELAY: u64 = 600;

    let drv_name = name.clone();

    Box::pin(
        async move {
            let mut restart_delay = START_DELAY;

            // Let the driver register its devices. This step is only done
            // once.

            let devices = Arc::new(Mutex::new(
                T::register_devices(req_chan, &cfg, max_history).await?,
            ));

            loop {
                // Create a Future that creates an instance of the driver
                // using the provided configuration parameters.

                let result = T::create_instance(&cfg)
                    .instrument(info_span!("driver-init", name = &name));

                if let Ok(mut instance) = result.await {
                    let name = name.clone();
                    let devices = devices.clone();

                    restart_delay = START_DELAY;

                    // Start the driver instance as a background task and
                    // monitor the return value.

                    let task = tokio::spawn(async move {
                        instance
                            .run(devices)
                            .instrument(info_span!(
                                "driver",
                                name = name.as_str(),
                                cfg = field::Empty
                            ))
                            .await
                    });

                    match task.await {
                        Ok(_) => unreachable!(),

                        // If `spawn()` returns this value, the driver
                        // exited abnormally. Report it and restart the
                        // driver.
                        Err(e) => error!("{}", e),
                    }
                }

                // Delay before restarting the driver. This prevents the
                // system from being compute-bound if the driver panics
                // right away.

                warn!("delay before restarting driver ...");
                tokio::time::sleep(tokio::time::Duration::from_secs(
                    restart_delay,
                ))
                .await;

                // Stretch the timeout each time we have to
                // restart. Set the max timeout to 10 minutes.

                restart_delay = std::cmp::min(restart_delay * 2, MAX_DELAY);
                info!("restarting instance of driver");
            }
        }
        .instrument(info_span!("mngr", drvr = drv_name)),
    )
}

#[derive(Clone)]
pub struct DriverDb(Arc<HashMap<&'static str, DriverInfo>>);

impl DriverDb {
    pub fn create() -> DriverDb {
        let mut table: HashMap<&'static str, DriverInfo> = HashMap::new();

        {
            use drv_memory::Instance;

            table.insert(
                Instance::NAME,
                (
                    Instance::SUMMARY,
                    Instance::DESCRIPTION,
                    Box::new(manage_instance::<Instance>),
                ),
            );
        }

        {
            use drv_timer::Instance;

            table.insert(
                Instance::NAME,
                (
                    Instance::SUMMARY,
                    Instance::DESCRIPTION,
                    Box::new(manage_instance::<Instance>),
                ),
            );
        }

        {
            use drv_cycle::Instance;

            table.insert(
                Instance::NAME,
                (
                    Instance::SUMMARY,
                    Instance::DESCRIPTION,
                    Box::new(manage_instance::<Instance>),
                ),
            );
        }

        // Load the set-up for the NTP monitor.

        #[cfg(feature = "driver-ntp")]
        {
            use drmem_drv_ntp::Instance;

            table.insert(
                Instance::NAME,
                (
                    Instance::SUMMARY,
                    Instance::DESCRIPTION,
                    Box::new(manage_instance::<Instance>),
                ),
            );
        }

        // Load the set-up for the GPIO sump pump monitor.

        #[cfg(feature = "driver-sump")]
        {
            use drmem_drv_sump::Instance;

            table.insert(
                Instance::NAME,
                (
                    Instance::SUMMARY,
                    Instance::DESCRIPTION,
                    Box::new(manage_instance::<Instance>),
                ),
            );
        }

        // Load the set-up for the GPIO sump pump monitor.

        #[cfg(feature = "driver-weather-wu")]
        {
            use drmem_drv_weather_wu::Instance;

            table.insert(
                Instance::NAME,
                (
                    Instance::SUMMARY,
                    Instance::DESCRIPTION,
                    Box::new(manage_instance::<Instance>),
                ),
            );
        }

        DriverDb(Arc::new(table))
    }

    /// Searches the map for a driver with the specified name. If
    /// present, the driver's information is returned.

    pub fn get_driver(&self, key: &str) -> Option<&DriverInfo> {
        self.0.get(key)
    }

    /// Searches the map for a driver with the specified name. If
    /// found, it extracts the information needed for the GraphQL
    /// query and returns it.

    pub fn find(
        &self, key: &str,
    ) -> Option<(String, &'static str, &'static str)> {
        self.get_driver(key)
            .map(|info| (key.to_string(), info.0, info.1))
    }

    /// Similar to `.find()`, but returns all the drivers'
    /// information.

    pub fn get_all(
        &self,
    ) -> impl Iterator<Item = (String, &'static str, &'static str)> + '_ {
        self.0.iter().map(|(k, (summary, description, _))| {
            (k.to_string(), *summary, *description)
        })
    }
}

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

pub type Fut<T> = Pin<Box<dyn Future<Output = T> + Send>>;
pub type MgrTask = Fut<Infallible>;
pub type MgrFuncRet = Fut<Result<MgrTask>>;

pub type Launcher = fn(
    driver::Name,
    driver::DriverConfig,
    driver::RequestChan,
    Option<usize>,
) -> MgrFuncRet;

pub type DriverInfo = (&'static str, &'static str, Launcher);

// This is the main loop of the driver manager. It only returns if the
// driver panics.

fn mgr_body<T>(
    name: driver::Name,
    devices: T::DeviceSet,
    cfg: driver::DriverConfig,
) -> MgrTask
where
    T: driver::API + Send + 'static,
{
    Box::pin(async move {
        const START_DELAY: u64 = 5;
        const MAX_DELAY: u64 = 600;

        let mut restart_delay = START_DELAY;
        let devices = Arc::new(Mutex::new(devices));

        info!("starting instance of driver");

        loop {
            // Create a Future that creates an instance of the driver
            // using the provided configuration parameters.

            let result = T::create_instance(&cfg)
                .instrument(info_span!("init", cfg = field::Empty));

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
                            name = name.as_ref(),
                            cfg = field::Empty
                        ))
                        .await
                });

                // Drivers are never supposed to exit so the JoinHandle
                // will never return an `Ok()` value. We can't stop
                // drivers from panicking, however, so we have to look for
                // an `Err()` value.
                //
                // (When Rust officially supports the `!` type, we will be
                // able to convert this from an `if-statement` to a simple
                // assignment.)

                if let Err(e) = task.await {
                    error!("driver exited unexpectedly -- {}", e)
                }
            }

            // Delay before restarting the driver. This prevents the
            // system from being compute-bound if the driver panics right
            // away.

            warn!("delay before restarting driver ...");
            tokio::time::sleep(tokio::time::Duration::from_secs(restart_delay))
                .await;

            // Stretch the timeout each time we have to restart. Set the
            // max timeout to 10 minutes.

            restart_delay = std::cmp::min(restart_delay * 2, MAX_DELAY);
            info!("restarting instance of driver");
        }
    })
}

// This generic function manages an instance of a specific driver. We
// use generics because each driver has a different set of devices
// (T::DeviceSet), so one function wouldn't be able to handle every
// type.

fn manage_instance<T>(
    name: driver::Name,
    cfg: driver::DriverConfig,
    req_chan: driver::RequestChan,
    max_history: Option<usize>,
) -> MgrFuncRet
where
    T: driver::API + Send + 'static,
{
    // Return a future that returns an error if the devices couldn't
    // be registered, or returns a future that manages the running
    // instance.

    Box::pin(async move {
        // Let the driver API register the necessary devices.

        let devices = T::register_devices(req_chan, &cfg, max_history)
            .instrument(info_span!("one-time-init", name = name.as_ref()))
            .await?;

        // Create a future that manages the instance.

        Ok(Box::pin(async move {
            let drv_name = name.clone();

            mgr_body::<T>(name, devices, cfg)
                .instrument(info_span!("mngr", drvr = drv_name.as_ref()))
                .await
        }) as MgrTask)
    }) as MgrFuncRet
}

#[derive(Clone)]
pub struct DriverDb(Arc<HashMap<driver::Name, DriverInfo>>);

impl DriverDb {
    pub fn create() -> DriverDb {
        let mut table: HashMap<driver::Name, DriverInfo> = HashMap::new();

        {
            use drv_memory::Instance;

            table.insert(
                Instance::NAME.into(),
                (
                    Instance::SUMMARY,
                    Instance::DESCRIPTION,
                    manage_instance::<Instance>,
                ),
            );
        }

        {
            use drv_timer::Instance;

            table.insert(
                Instance::NAME.into(),
                (
                    Instance::SUMMARY,
                    Instance::DESCRIPTION,
                    manage_instance::<Instance>,
                ),
            );
        }

        {
            use drv_cycle::Instance;

            table.insert(
                Instance::NAME.into(),
                (
                    Instance::SUMMARY,
                    Instance::DESCRIPTION,
                    manage_instance::<Instance>,
                ),
            );
        }

        // Load the set-up for the NTP monitor.

        #[cfg(feature = "driver-ntp")]
        {
            use drmem_drv_ntp::Instance;

            table.insert(
                Instance::NAME.into(),
                (
                    Instance::SUMMARY,
                    Instance::DESCRIPTION,
                    manage_instance::<Instance>,
                ),
            );
        }

        // Load the set-up for the GPIO sump pump monitor.

        #[cfg(feature = "driver-sump")]
        {
            use drmem_drv_sump::Instance;

            table.insert(
                Instance::NAME.into(),
                (
                    Instance::SUMMARY,
                    Instance::DESCRIPTION,
                    manage_instance::<Instance>,
                ),
            );
        }

        // Load the set-up for the Weather Underground driver.

        #[cfg(feature = "driver-weather-wu")]
        {
            use drmem_drv_weather_wu::Instance;

            table.insert(
                Instance::NAME.into(),
                (
                    Instance::SUMMARY,
                    Instance::DESCRIPTION,
                    manage_instance::<Instance>,
                ),
            );
        }

        // Load the set-up for the TP-Link driver.

        #[cfg(feature = "driver-tplink")]
        {
            use drmem_drv_tplink::Instance;

            table.insert(
                Instance::NAME.into(),
                (
                    Instance::SUMMARY,
                    Instance::DESCRIPTION,
                    manage_instance::<Instance>,
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
        &self,
        key: &str,
    ) -> Option<(driver::Name, &'static str, &'static str)> {
        self.get_driver(key)
            .map(|info| (key.into(), info.0, info.1))
    }

    /// Similar to `.find()`, but returns all the drivers'
    /// information.

    pub fn get_all(
        &self,
    ) -> impl Iterator<Item = (driver::Name, &'static str, &'static str)> + '_
    {
        self.0.iter().map(|(k, (summary, description, _))| {
            (k.clone(), *summary, *description)
        })
    }
}

use drmem_api::{
    driver::{self, Registrator, ResettableState, API},
    Result,
};
use futures::future::Future;
use std::collections::HashMap;
use std::{convert::Infallible, pin::Pin, sync::Arc};
use tokio::sync::Barrier;
use tracing::{error, field, info, info_span, warn, Instrument};

mod drv_cycle;
mod drv_latch;
mod drv_map;
mod drv_memory;
mod drv_timer;

pub type Fut<T> = Pin<Box<dyn Future<Output = T> + Send>>;
pub type MgrTask = Fut<Result<Infallible>>;

pub type Launcher = fn(
    driver::DriverConfig,
    driver::RequestChan,
    Option<usize>,
    barrier: Arc<Barrier>,
) -> MgrTask;

type DriverInfo = (&'static str, &'static str, Launcher);

// This is the main loop of the driver manager. It only returns if the
// driver panics.

async fn mgr_body<T>(mut devices: T::HardwareType, cfg: T::Config) -> Infallible
where
    T: API + Send + 'static,
    <T::Config as TryFrom<driver::DriverConfig>>::Error: std::fmt::Display,
{
    const START_DELAY: u64 = 5;
    const MAX_DELAY: u64 = 600;

    let mut restart_delay = START_DELAY;

    loop {
        info!("initializing");

        // Create a Future that creates an instance of the driver
        // using the provided configuration parameters.

        match T::create_instance(&cfg).await {
            Ok(mut instance) => {
                use futures::FutureExt;
                use std::panic::AssertUnwindSafe;

                restart_delay = START_DELAY;
                info!("running");

                let run = instance.run(&mut devices);

                // Drivers are never supposed to exit so
                // catch_unwind() will only catch panics which means
                // we need to only look for `Err(_)` values.

                let Err(e) = AssertUnwindSafe(run).catch_unwind().await;

                error!("exited unexpectedly -- {e:?}")
            }
            Err(e) => error!("couldn't create instance -- {e}"),
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

        devices.reset_state();
    }
}

// This generic function manages an instance of a specific driver. We
// use generics because each driver has a different set of devices
// (T::HardwareType), so one function wouldn't be able to handle every
// type.

fn manage_instance<T>(
    cfg: driver::DriverConfig,
    mut req_chan: driver::RequestChan,
    max_history: Option<usize>,
    barrier: Arc<Barrier>,
) -> MgrTask
where
    T: API + Send + 'static,
    <T::Config as TryFrom<driver::DriverConfig>>::Error:
        std::fmt::Display + Send,
    drmem_api::Error: From<<<T as API>::Config as TryFrom<driver::DriverConfig>>::Error>
        + Send,
{
    Box::pin(async move {
        let cfg = match T::Config::try_from(cfg) {
            Ok(cfg) => Ok(cfg),
            err @ Err(_) => {
                barrier.wait().await;
                err
            }
        }?;

        // Let the driver API register the necessary devices.

        let devices =
            T::HardwareType::register_devices(&mut req_chan, &cfg, max_history)
                .instrument(info_span!("register", cfg = field::Empty))
                .await;

        // When we reached this location, all devices have been
        // registered (or not, if an error occurred). Sync to the
        // barrier so the main loop and move on to the next driver.

        barrier.wait().await;

        // Create a future that manages the instance.

        Ok(mgr_body::<T>(devices?, cfg).await)
    })
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
            use drv_latch::Instance;

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
            use drv_map::Instance;

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

        #[cfg(feature = "drmem-drv-ntp")]
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

        #[cfg(feature = "drmem-drv-sump")]
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

        #[cfg(feature = "drmem-drv-weather-wu")]
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

        #[cfg(feature = "drmem-drv-tplink")]
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
    #[cfg(feature = "graphql")]
    pub fn find(
        &self,
        key: &str,
    ) -> Option<(driver::Name, &'static str, &'static str)> {
        self.get_driver(key)
            .map(|info| (key.into(), info.0, info.1))
    }

    /// Similar to `.find()`, but returns all the drivers'
    /// information.
    #[cfg(feature = "graphql")]
    pub fn get_all(
        &self,
    ) -> impl Iterator<Item = (driver::Name, &'static str, &'static str)> + '_
    {
        self.0.iter().map(|(k, (summary, description, _))| {
            (k.clone(), *summary, *description)
        })
    }
}

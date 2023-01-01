use drmem_api::{
    driver::{self, DriverConfig},
    types::{device, Error},
    Result,
};
use std::{convert::Infallible, future::Future, pin::Pin};
use tracing::{self, error};

pub struct Instance {
    d_memory: driver::ReportReading<device::Value>,
    s_memory: driver::RxDeviceSetting,
}

impl Instance {
    pub const NAME: &'static str = "memory";

    pub const SUMMARY: &'static str = "An area in memory to set values.";

    pub const DESCRIPTION: &'static str = include_str!("drv_memory.md");

    /// Creates a new `Instance` instance.

    pub fn new(
        d_memory: driver::ReportReading<device::Value>,
        s_memory: driver::RxDeviceSetting,
    ) -> Instance {
        Instance { d_memory, s_memory }
    }

    // Gets the name of the device from the configuration.

    fn get_cfg_name(cfg: &DriverConfig) -> Result<device::Base> {
        match cfg.get("name") {
            Some(toml::value::Value::String(name)) => {
                if let Ok(name) = name.parse::<device::Base>() {
                    return Ok(name);
                } else {
                    error!("'name' isn't a proper, base name for a device")
                }
            }
            Some(_) => error!("'name' config parameter should be a string"),
            None => error!("missing 'name' parameter in config"),
        }

        Err(Error::BadConfig)
    }
}

impl driver::API for Instance {
    fn create_instance(
        cfg: DriverConfig, core: driver::RequestChan,
        max_history: Option<usize>,
    ) -> Pin<
        Box<dyn Future<Output = Result<driver::DriverType>> + Send + 'static>,
    > {
        let fut = async move {
            let name = Instance::get_cfg_name(&cfg)?;

            // This device is settable. Any setting is forwarded to
            // the backend.

            let (d_memory, s_memory, _) =
                core.add_rw_device(name, None, max_history).await?;

            // Build and return the future.

            Ok(Box::new(Instance::new(d_memory, s_memory))
                as driver::DriverType)
        };

        Box::pin(fut)
    }

    fn run<'a>(
        &'a mut self,
    ) -> Pin<Box<dyn Future<Output = Infallible> + Send + 'a>> {
        let fut = async {
            loop {
                if let Some((v, tx)) = self.s_memory.recv().await {
                    let _ = tx.send(Ok(v.clone()));

                    (self.d_memory)(v).await
                } else {
                    panic!("can no longer receive settings");
                }
            }
        };

        Box::pin(fut)
    }
}

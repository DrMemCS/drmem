use drmem_api::{
    device,
    driver::{self, DriverConfig},
    Error, Result,
};
use std::{convert::Infallible, future::Future, pin::Pin, sync::Arc};
use tokio::sync::Mutex;

pub struct Instance;

pub struct Devices {
    d_memory: driver::ReadWriteDevice<device::Value>,
}

impl Instance {
    pub const NAME: &'static str = "memory";

    pub const SUMMARY: &'static str = "An area in memory to set values.";

    pub const DESCRIPTION: &'static str = include_str!("drv_memory.md");

    /// Creates a new `Instance` instance.

    pub fn new() -> Instance {
        Instance {}
    }

    // Gets the name of the device from the configuration.

    fn get_cfg_name(cfg: &DriverConfig) -> Result<device::Base> {
        match cfg.get("name") {
            Some(toml::value::Value::String(name)) => {
                if let v @ Ok(_) = name.parse::<device::Base>() {
                    v
                } else {
                    Err(Error::ConfigError(String::from(
                        "'name' isn't a proper, base name for a device",
                    )))
                }
            }
            Some(_) => Err(Error::ConfigError(String::from(
                "'name' config parameter should be a string",
            ))),
            None => Err(Error::ConfigError(String::from(
                "missing 'name' parameter in config",
            ))),
        }
    }

    // Gets the initial value of the device from the configuration.

    fn get_cfg_init_val(cfg: &DriverConfig) -> Option<device::Value> {
        cfg.get("initial")
            .and_then(|v| device::Value::try_from(v).ok())
    }
}

impl driver::API for Instance {
    type DeviceSet = Devices;

    fn register_devices(
        core: driver::RequestChan,
        cfg: &DriverConfig,
        max_history: Option<usize>,
    ) -> Pin<Box<dyn Future<Output = Result<Self::DeviceSet>> + Send>> {
        let name = Instance::get_cfg_name(cfg);
        let init_value = Instance::get_cfg_init_val(cfg);

        Box::pin(async move {
            let name = name?;

            // This device is settable. Any setting is forwarded to
            // the backend.

            let mut d_memory =
                core.add_rw_device(name, None, max_history).await?;

            // If the user configured an initial value and there was
            // no previous value, immediately set it.

            if d_memory.get_last().is_none() {
                if let Some(v) = init_value {
                    d_memory.report_update(v).await
                }
            }

            Ok(Devices { d_memory })
        })
    }

    fn create_instance(
        _cfg: &DriverConfig,
    ) -> Pin<Box<dyn Future<Output = Result<Box<Self>>> + Send>> {
        let fut = async move {
            // Build and return the future.

            Ok(Box::new(Instance::new()))
        };

        Box::pin(fut)
    }

    fn run<'a>(
        &'a mut self,
        devices: Arc<Mutex<Self::DeviceSet>>,
    ) -> Pin<Box<dyn Future<Output = Infallible> + Send + 'a>> {
        let fut = async move {
            let mut devices = devices.lock().await;

            while let Some((v, reply)) = devices.d_memory.next_setting().await {
                reply(Ok(v.clone()));
                devices.d_memory.report_update(v).await
            }
            panic!("can no longer receive settings");
        };

        Box::pin(fut)
    }
}

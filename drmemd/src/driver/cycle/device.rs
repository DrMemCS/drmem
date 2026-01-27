use drmem_api::{
    device,
    driver::{self, ResettableState},
    Result,
};

use super::config;

pub struct Set {
    pub d_output: driver::ReadOnlyDevice<device::Value>,
    pub d_enable: driver::ReadWriteDevice<bool>,
}

impl driver::Registrator for Set {
    type Config = config::Params;

    async fn register_devices(
        core: &mut driver::RequestChan,
        _cfg: &Self::Config,
        max_history: Option<usize>,
    ) -> Result<Self> {
        // Define the devices managed by this driver.
        //
        // This first device is the output signal. It toggles between
        // `false` and `true` at a rate determined by the `interval`
        // config option.

        let d_output = core.add_ro_device("output", None, max_history).await?;

        // This device is settable. Any time it transitions from
        // `false` to `true`, the output device begins a cycling.
        // When this device is set to `false`, the device stops
        // cycling.

        let d_enable = core.add_rw_device("enable", None, max_history).await?;

        Ok(Set { d_output, d_enable })
    }
}

impl ResettableState for Set {}

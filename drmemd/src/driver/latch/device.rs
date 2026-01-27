use drmem_api::{
    device,
    driver::{self, ResettableState},
    Result,
};

use crate::driver::latch::config;

pub struct Set {
    pub d_output: driver::ReadOnlyDevice<device::Value>,
    pub d_trigger: driver::ReadWriteDevice<bool>,
    pub d_reset: driver::ReadWriteDevice<bool>,
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
        // This first device is the output of the timer.

        let d_output = core.add_ro_device("output", None, max_history).await?;

        let d_trigger =
            core.add_rw_device("trigger", None, max_history).await?;

        let d_reset = core.add_rw_device("reset", None, max_history).await?;

        Ok(Set {
            d_output,
            d_trigger,
            d_reset,
        })
    }
}

impl ResettableState for Set {}

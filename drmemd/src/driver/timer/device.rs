use drmem_api::{
    device::Value,
    driver::{self, ResettableState},
    Result,
};

use super::{config, device};

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
        // This first device is the output of the timer. When it's not
        // timing, this device's value with be `!level`. While it's
        // timing, `level`.

        let d_output = core.add_ro_device("output", None, max_history).await?;

        // This device is settable. Any time it transitions from
        // `false` to `true`, the timer begins a timing cycle.

        let d_enable = core.add_rw_device("enable", None, max_history).await?;

        Ok(Set { d_output, d_enable })
    }
}

impl ResettableState for Set {}

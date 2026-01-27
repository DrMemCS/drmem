use drmem_api::{
    device::Value,
    driver::{self, ResettableState},
    Result,
};

use super::config;

pub struct Set {
    pub d_output: driver::ReadOnlyDevice<Value>,
    pub d_index: driver::ReadWriteDevice<i32>,
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
        // This first device is the output of the map.

        let d_output = core.add_ro_device("output", None, max_history).await?;

        // This device is settable. Any setting is forwarded to
        // the backend.

        let d_index = core.add_rw_device("index", None, max_history).await?;

        Ok(Set { d_output, d_index })
    }
}

impl ResettableState for Set {}

use drmem_api::{
    device::Value,
    driver::{self, Reporter, ResettableState},
    Result,
};

use super::config;

pub struct Set<R: Reporter> {
    pub d_output: driver::ReadOnlyDevice<Value, R>,
    pub d_index: driver::ReadWriteDevice<i32, R>,
}

impl<R: Reporter> driver::Registrator<R> for Set<R> {
    type Config = config::Params;

    async fn register_devices(
        core: &mut driver::RequestChan<R>,
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

impl<R: Reporter> ResettableState for Set<R> {}

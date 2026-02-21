use drmem_api::{
    device::{self, Path},
    driver::{self, Reporter, ResettableState},
    Result,
};

use crate::driver::latch::config;

pub struct Set<R: Reporter> {
    pub d_output: driver::ReadOnlyDevice<device::Value, R>,
    pub d_trigger: driver::ReadWriteDevice<bool, R>,
    pub d_reset: driver::ReadWriteDevice<bool, R>,
}

impl<R: Reporter> driver::Registrator<R> for Set<R> {
    type Config = config::Params;

    async fn register_devices(
        core: &mut driver::RequestChan<R>,
        subpath: Option<&Path>,
        _cfg: &Self::Config,
        max_history: Option<usize>,
    ) -> Result<Self> {
        // Define the devices managed by this driver.
        //
        // This first device is the output of the timer.

        let d_output = core
            .add_ro_device("output", subpath, None, max_history)
            .await?;

        let d_trigger = core
            .add_rw_device("trigger", subpath, None, max_history)
            .await?;

        let d_reset = core
            .add_rw_device("reset", subpath, None, max_history)
            .await?;

        Ok(Set {
            d_output,
            d_trigger,
            d_reset,
        })
    }
}

impl<R: Reporter> ResettableState for Set<R> {}

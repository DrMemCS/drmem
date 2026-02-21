use drmem_api::{
    device::{Path, Value},
    driver::{self, Reporter, ResettableState},
    Result,
};

use super::config;

pub struct Set<R: Reporter> {
    pub d_output: driver::ReadOnlyDevice<Value, R>,
    pub d_enable: driver::ReadWriteDevice<bool, R>,
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
        // This first device is the output signal. It toggles between
        // `false` and `true` at a rate determined by the `interval`
        // config option.

        let d_output = core
            .add_ro_device("output", subpath, None, max_history)
            .await?;

        // This device is settable. Any time it transitions from
        // `false` to `true`, the output device begins a cycling.
        // When this device is set to `false`, the device stops
        // cycling.

        let d_enable = core
            .add_rw_device("enable", subpath, None, max_history)
            .await?;

        Ok(Set { d_output, d_enable })
    }
}

impl<R: Reporter> ResettableState for Set<R> {}

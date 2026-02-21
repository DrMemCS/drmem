use drmem_api::{
    device::Path,
    driver::{self, Reporter, ResettableState},
    Result,
};

use super::config;

pub struct Set<R: Reporter> {
    pub d_count: driver::ReadOnlyDevice<i32, R>,
    pub d_increment: driver::ReadWriteDevice<bool, R>,
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
        // This first device is the count value.

        let d_count = core
            .add_ro_device("count", subpath, None, max_history)
            .await?;

        // Any time it transitions from `false` to `true`, the count increases
        // by one.

        let d_increment = core
            .add_rw_device("increment", subpath, None, max_history)
            .await?;

        // Any time it transitions from `false` to `true`, the count resets to 0.

        let d_reset = core
            .add_rw_device("reset", subpath, None, max_history)
            .await?;

        Ok(Set {
            d_count,
            d_increment,
            d_reset,
        })
    }
}

impl<R: Reporter> ResettableState for Set<R> {}

use drmem_api::{
    device::Path,
    driver::{self, Reporter, ResettableState},
    Result,
};

use super::config;

pub struct Set<R: Reporter> {
    pub d_sum: driver::ReadOnlyDevice<f64, R>,
    pub d_data: driver::ReadWriteDevice<f64, R>,
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

        let d_sum = core
            .add_ro_device("sum", subpath, None, max_history)
            .await?;
        let d_data = core
            .add_rw_device("data", subpath, None, max_history)
            .await?;
        let d_reset = core
            .add_rw_device("reset", subpath, None, max_history)
            .await?;

        Ok(Set {
            d_sum,
            d_data,
            d_reset,
        })
    }
}

impl<R: Reporter> ResettableState for Set<R> {}

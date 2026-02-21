use super::config;
use drmem_api::{
    device::Path,
    driver::{self, Reporter, ResettableState},
    Result,
};

pub struct Set<R: Reporter> {
    pub d_state: driver::ReadOnlyDevice<bool, R>,
    pub d_source: driver::ReadOnlyDevice<String, R>,
    pub d_offset: driver::ReadOnlyDevice<f64, R>,
    pub d_delay: driver::ReadOnlyDevice<f64, R>,
}

impl<R: Reporter> driver::Registrator<R> for Set<R> {
    type Config = config::Params;

    async fn register_devices(
        core: &mut driver::RequestChan<R>,
        subpath: Option<&Path>,
        _: &Self::Config,
        max_history: Option<usize>,
    ) -> Result<Self> {
        // Define the devices managed by this driver.

        let d_state = core
            .add_ro_device("state", subpath, None, max_history)
            .await?;
        let d_source = core
            .add_ro_device("source", subpath, None, max_history)
            .await?;
        let d_offset = core
            .add_ro_device("offset", subpath, Some("ms"), max_history)
            .await?;
        let d_delay = core
            .add_ro_device("delay", subpath, Some("ms"), max_history)
            .await?;

        Ok(Set {
            d_state,
            d_source,
            d_offset,
            d_delay,
        })
    }
}

impl<R: Reporter> ResettableState for Set<R> {}

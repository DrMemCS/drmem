use super::config;
use drmem_api::{
    device::Path,
    driver::{self, Reporter, ResettableState},
    Result,
};

pub struct Set<R: Reporter> {
    pub d_service: driver::ReadOnlyDevice<bool, R>,
    pub d_state: driver::ReadOnlyDevice<bool, R>,
    pub d_duty: driver::ReadOnlyDevice<f64, R>,
    pub d_inflow: driver::ReadOnlyDevice<f64, R>,
    pub d_duration: driver::ReadOnlyDevice<f64, R>,
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

        let d_service = core
            .add_ro_device("service", subpath, None, max_history)
            .await?;
        let d_state = core
            .add_ro_device("state", subpath, None, max_history)
            .await?;
        let d_duty = core
            .add_ro_device("duty", subpath, Some("%"), max_history)
            .await?;
        let d_inflow = core
            .add_ro_device("in-flow", subpath, Some("gpm"), max_history)
            .await?;
        let d_duration = core
            .add_ro_device("duration", subpath, Some("min"), max_history)
            .await?;

        Ok(Set {
            d_service,
            d_state,
            d_duty,
            d_inflow,
            d_duration,
        })
    }
}

impl<R: Reporter> ResettableState for Set<R> {}

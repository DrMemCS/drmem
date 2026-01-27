use super::config;
use drmem_api::{
    driver::{self, ResettableState},
    Result,
};

pub struct Set {
    pub d_service: driver::ReadOnlyDevice<bool>,
    pub d_state: driver::ReadOnlyDevice<bool>,
    pub d_duty: driver::ReadOnlyDevice<f64>,
    pub d_inflow: driver::ReadOnlyDevice<f64>,
    pub d_duration: driver::ReadOnlyDevice<f64>,
}

impl driver::Registrator for Set {
    type Config = config::Params;

    async fn register_devices(
        core: &mut driver::RequestChan,
        _: &Self::Config,
        max_history: Option<usize>,
    ) -> Result<Self> {
        // Define the devices managed by this driver.

        let d_service =
            core.add_ro_device("service", None, max_history).await?;
        let d_state = core.add_ro_device("state", None, max_history).await?;
        let d_duty = core.add_ro_device("duty", Some("%"), max_history).await?;
        let d_inflow = core
            .add_ro_device("in-flow", Some("gpm"), max_history)
            .await?;
        let d_duration = core
            .add_ro_device("duration", Some("min"), max_history)
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

impl ResettableState for Set {}

use super::config;
use drmem_api::{
    driver::{self, ResettableState},
    Result,
};

pub struct Set {
    pub d_state: driver::ReadOnlyDevice<bool>,
    pub d_source: driver::ReadOnlyDevice<String>,
    pub d_offset: driver::ReadOnlyDevice<f64>,
    pub d_delay: driver::ReadOnlyDevice<f64>,
}

impl driver::Registrator for Set {
    type Config = config::Params;

    async fn register_devices(
        core: &mut driver::RequestChan,
        _: &Self::Config,
        max_history: Option<usize>,
    ) -> Result<Self> {
        // Define the devices managed by this driver.

        let d_state = core.add_ro_device("state", None, max_history).await?;
        let d_source = core.add_ro_device("source", None, max_history).await?;
        let d_offset = core
            .add_ro_device("offset", Some("ms"), max_history)
            .await?;
        let d_delay =
            core.add_ro_device("delay", Some("ms"), max_history).await?;

        Ok(Set {
            d_state,
            d_source,
            d_offset,
            d_delay,
        })
    }
}

impl ResettableState for Set {}

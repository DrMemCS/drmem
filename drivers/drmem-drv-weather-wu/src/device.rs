use drmem_api::{
    driver::{classes, Registrator, RequestChan, ResettableState},
    Result,
};

use crate::config;

pub struct Set(pub classes::Weather);

impl Registrator for Set {
    type Config = config::Params;

    async fn register_devices(
        drc: &mut RequestChan,
        cfg: &Self::Config,
        max_history: Option<usize>,
    ) -> Result<Self> {
        Ok(Set(classes::Weather::register_devices(
            drc,
            &classes::WeatherConfig {
                units: cfg.units.clone(),
            },
            max_history,
        )
        .await?))
    }
}

impl ResettableState for Set {
    fn reset_state(&mut self) {
        self.0.reset_state()
    }
}

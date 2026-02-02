use drmem_api::{
    driver::{classes, Registrator, Reporter, RequestChan, ResettableState},
    Result,
};

use crate::config;

pub struct Set<R: Reporter>(pub classes::Weather<R>);

impl<R: Reporter> Registrator<R> for Set<R> {
    type Config = config::Params;

    async fn register_devices(
        drc: &mut RequestChan<R>,
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

impl<R: Reporter> ResettableState for Set<R> {
    fn reset_state(&mut self) {
        self.0.reset_state()
    }
}

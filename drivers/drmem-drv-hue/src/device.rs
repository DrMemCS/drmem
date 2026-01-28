use super::config;
use drmem_api::{
    Result,
    driver::{Registrator, Reporter, ResettableState, classes},
};
use std::{collections::HashMap, sync::Arc};
use tokio::time::Duration;

// Each type of device has a specific set of channels. In order to
// allow a map of different types, we wrap it in an enum.
pub enum DeviceSet<R: Reporter> {
    Switch(classes::Switch<R>),
    Bulb(classes::Dimmer<R>),
    ColorBulb(classes::ColorBulb<R>),
    Group(classes::ColorBulb<R>),
}

// The set of devices for an instance of this driver is held in a map
// which maps the device ID to its set of device channels.
pub struct Set<R: Reporter> {
    pub map: HashMap<Arc<str>, DeviceSet<R>>,
}

impl<R: Reporter> Set<R> {
    pub async fn from_devcfg(
        drc: &mut drmem_api::driver::RequestChan<R>,
        cfg: &config::DeviceConfig,
        max_history: Option<usize>,
    ) -> Result<(Arc<str>, DeviceSet<R>)> {
        let tmo = cfg.override_timeout.map(|v| Duration::from_secs(v * 60));

        Ok((
            cfg.id.clone(),
            match cfg.r#type {
                config::DevCfgType::Switch => DeviceSet::Switch(
                    classes::Switch::register_devices(drc, &tmo, max_history)
                        .await?,
                ),
                config::DevCfgType::Dimmer | config::DevCfgType::Bulb => {
                    DeviceSet::Bulb(
                        classes::Dimmer::register_devices(
                            drc,
                            &tmo,
                            max_history,
                        )
                        .await?,
                    )
                }
                config::DevCfgType::ColorBulb => DeviceSet::ColorBulb(
                    classes::ColorBulb::register_devices(
                        drc,
                        &tmo,
                        max_history,
                    )
                    .await?,
                ),
                config::DevCfgType::Group => DeviceSet::Group(
                    classes::ColorBulb::register_devices(
                        drc,
                        &tmo,
                        max_history,
                    )
                    .await?,
                ),
            },
        ))
    }
}

impl<R: Reporter> Registrator<R> for Set<R> {
    type Config = config::Params;

    async fn register_devices<'a>(
        drc: &'a mut drmem_api::driver::RequestChan<R>,
        cfg: &'a Self::Config,
        max_history: Option<usize>,
    ) -> Result<Self> {
        let mut map = HashMap::new();

        for dcfg in &cfg.devices {
            match Set::from_devcfg(drc, dcfg, max_history).await {
                Ok((k, v)) => {
                    let _ = map.insert(k, v);
                }
                Err(e) => return Err(e),
            }
        }
        Ok(Set { map })
    }
}

impl<R: Reporter> ResettableState for Set<R> {
    fn reset_state(&mut self) {
        for (_, entry) in self.map.iter_mut() {
            match entry {
                DeviceSet::Switch(dev) => {
                    dev.state.reset_state();
                    dev.indicator.reset_state();
                }
                DeviceSet::Bulb(dev) => {
                    dev.brightness.reset_state();
                    dev.indicator.reset_state();
                }
                DeviceSet::ColorBulb(dev) => {
                    dev.brightness.reset_state();
                    dev.color.reset_state();
                }
                DeviceSet::Group(dev) => {
                    dev.brightness.reset_state();
                    dev.color.reset_state();
                }
            }
        }
    }
}

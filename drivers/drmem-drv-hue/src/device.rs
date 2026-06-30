use super::{config, driver::device_traits};
use drmem_api::{
    Result,
    device::Path,
    driver::{Registrator, Reporter, ResettableState, classes},
};
use std::{collections::HashMap, sync::Arc};
use tokio::time::Duration;

use device_traits::DeviceWrapper;

// The set of devices for an instance of this driver is held in a map
// which maps the device ID to its set of device channels.
pub struct Set<R: Reporter> {
    pub map: HashMap<Arc<str>, DeviceWrapper<R>>,
}

impl<R: Reporter> Set<R> {
    pub async fn from_devcfg(
        drc: &mut drmem_api::driver::RequestChan<R>,
        cfg: &config::DeviceConfig,
        max_history: Option<usize>,
    ) -> Result<(Arc<str>, DeviceWrapper<R>)> {
        let tmo = cfg.override_timeout.map(|v| Duration::from_secs(v));

        Ok((
            cfg.id.clone(),
            match cfg.r#type {
                config::DevCfgType::Switch => {
                    DeviceWrapper::Switch(device_traits::SwitchDevice {
                        inner: classes::Switch::register_devices(
                            drc,
                            Some(cfg.subpath.as_ref()),
                            &tmo,
                            max_history,
                        )
                        .await?,
                    })
                }
                config::DevCfgType::Dimmer | config::DevCfgType::Bulb => {
                    DeviceWrapper::Dimmer(device_traits::DimmerDevice {
                        inner: classes::Dimmer::register_devices(
                            drc,
                            Some(cfg.subpath.as_ref()),
                            &tmo,
                            max_history,
                        )
                        .await?,
                    })
                }
                config::DevCfgType::ColorBulb => DeviceWrapper::ColorBulb(
                    device_traits::ColorBulbDevice::new(
                        classes::ColorBulb::register_devices(
                            drc,
                            Some(cfg.subpath.as_ref()),
                            &tmo,
                            max_history,
                        )
                        .await?,
                        false, // not a group
                    ),
                ),
                config::DevCfgType::Group => DeviceWrapper::ColorBulb(
                    device_traits::ColorBulbDevice::new(
                        classes::ColorBulb::register_devices(
                            drc,
                            Some(cfg.subpath.as_ref()),
                            &tmo,
                            max_history,
                        )
                        .await?,
                        true, // is a group
                    ),
                ),
            },
        ))
    }
}

impl<R: Reporter> Registrator<R> for Set<R> {
    type Config = config::Params;

    async fn register_devices(
        drc: &mut drmem_api::driver::RequestChan<R>,
        _subpath: Option<&Path>,
        cfg: &Self::Config,
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
            entry.reset();
        }
    }
}

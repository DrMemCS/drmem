use drmem_api::{
    driver::{classes, Registrator, Reporter, RequestChan, ResettableState},
    Result,
};
use tokio::time::Duration;

use crate::config;

// An instance of a driver can be a switch, dimmer, or outlet device.
// This type specifies the device channels for the given types. The
// driver instance will have one of these variants for its device set.
pub enum Set<R: Reporter> {
    Switch(classes::Switch<R>),
    Dimmer(classes::Dimmer<R>),
}

// A set of devices must be resettable (in case the device gets
// rebooted.) This implementation simply resets the devices in the set
// used by the driver instance.
impl<R: Reporter> ResettableState for Set<R> {
    fn reset_state(&mut self) {
        match self {
            Set::Switch(dev) => {
                dev.state.reset_state();
                dev.indicator.reset_state();
            }
            Set::Dimmer(dev) => {
                dev.brightness.reset_state();
                dev.indicator.reset_state();
            }
        }
    }
}

impl<R: Reporter> Registrator<R> for Set<R> {
    type Config = config::Params;

    // Defines the registration interface for the device set.
    async fn register_devices<'a>(
        drc: &'a mut RequestChan<R>,
        cfg: &'a Self::Config,
        max_history: Option<usize>,
    ) -> Result<Self> {
        match cfg.r#type {
            config::DevCfgType::Switch | config::DevCfgType::Outlet => {
                Ok(Set::Switch(
                    classes::Switch::register_devices(
                        drc,
                        &cfg.override_timeout
                            .map(|v| Duration::from_secs(60 * v)),
                        max_history,
                    )
                    .await?,
                ))
            }
            config::DevCfgType::Dimmer => Ok(Set::Dimmer(
                classes::Dimmer::register_devices(
                    drc,
                    &cfg.override_timeout.map(|v| Duration::from_secs(60 * v)),
                    max_history,
                )
                .await?,
            )),
        }
    }
}

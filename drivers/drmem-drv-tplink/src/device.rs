use drmem_api::{
    device::Path,
    driver::{
        classes, OverridableDevice, OverrideConfig, Registrator, Reporter,
        RequestChan, ResettableState,
    },
    Result,
};
use tokio::time::Duration;

use crate::config;

pub enum Setting {
    Dimmer(classes::DimmerProperty),
    Indicator(bool),
}

pub struct DimmerWithIndicator<R: Reporter> {
    pub dimmer: classes::Dimmer<R>,
    pub indicator: OverridableDevice<bool, R>,
}

impl<R: Reporter> DimmerWithIndicator<R> {
    /// Reports any new properties specified in the `prop` parameter.
    pub async fn report_update(&mut self, prop: classes::DimmerProperty) {
        self.dimmer.report_update(prop).await
    }

    pub async fn next_setting(&mut self) -> Option<Setting> {
        tokio::select! {
            Some(res) = self.dimmer.next_setting() => {
                Some(Setting::Dimmer(res))
            },
            res = self.indicator.next_setting() => {
                if let Some((value, resp)) = res {
                    if let Some(resp) = resp {
                        resp.ok(value);
                    }
                    Some(Setting::Indicator(value))
                } else {
                    None
                }
            }
        }
    }
}

impl<R: Reporter> Registrator<R> for DimmerWithIndicator<R> {
    type Config = OverrideConfig;

    async fn register_devices(
        drc: &mut RequestChan<R>,
        subpath: Option<&Path>,
        cfg: &Self::Config,
        max_history: Option<usize>,
    ) -> Result<Self> {
        Ok(DimmerWithIndicator {
            dimmer: classes::Dimmer::register_devices(
                drc,
                subpath,
                cfg,
                max_history,
            )
            .await?,
            indicator: drc
                .add_overridable_device(
                    "indicator",
                    subpath,
                    None,
                    cfg.override_duration,
                    cfg.envelope,
                    max_history,
                )
                .await?,
        })
    }
}

impl<R: Reporter> ResettableState for DimmerWithIndicator<R> {}

// An instance of a driver can be a switch, dimmer, or outlet device.
// This type specifies the device channels for the given types. The
// driver instance will have one of these variants for its device set.
pub enum Set<R: Reporter> {
    Switch(classes::Switch<R>),
    Dimmer(DimmerWithIndicator<R>),
}

// A set of devices must be resettable (in case the device gets
// rebooted.) This implementation simply resets the devices in the set
// used by the driver instance.
impl<R: Reporter> ResettableState for Set<R> {
    fn reset_state(&mut self) {
        match self {
            Set::Switch(dev) => {
                dev.reset_state();
            }
            Set::Dimmer(dev) => {
                dev.dimmer.reset_state();
                dev.indicator.reset_state();
            }
        }
    }
}

impl<R: Reporter> Registrator<R> for Set<R> {
    type Config = config::Params;

    // Defines the registration interface for the device set.
    async fn register_devices(
        drc: &mut RequestChan<R>,
        subpath: Option<&Path>,
        cfg: &Self::Config,
        max_history: Option<usize>,
    ) -> Result<Self> {
        let override_cfg = OverrideConfig {
            override_duration: cfg
                .override_timeout
                .map(|v| Duration::from_secs(60 * v)),
            envelope: Some(Duration::from_secs(5)),
        };

        match cfg.r#type {
            config::DevCfgType::Switch | config::DevCfgType::Outlet => {
                Ok(Set::Switch(
                    classes::Switch::register_devices(
                        drc,
                        subpath,
                        &override_cfg,
                        max_history,
                    )
                    .await?,
                ))
            }
            config::DevCfgType::Dimmer => Ok(Set::Dimmer(
                DimmerWithIndicator::register_devices(
                    drc,
                    subpath,
                    &override_cfg,
                    max_history,
                )
                .await?,
            )),
        }
    }
}

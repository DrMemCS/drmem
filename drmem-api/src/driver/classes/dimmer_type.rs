//! Define device representation of wall dimmer switches.
//!
//! Defines a `Registrator` that registers device names and typed
//! channels to control it. A driver which controls a switch should
//! use this instead of registering their own device channels:
//!
//! ```rust,ignore
//! use drmem_api::driver::{self, classes};
//!
//! struct MyDimmerDriver { ... };
//!
//! impl driver::API for MyDimmerDriver {
//!     type HardwareType = classes::Dimmer;
//!
//!     ...
//! }
//! ```

use crate::{
    device::Path,
    driver::{
        overridable_device::OverridableDevice, ro_device::ReadOnlyDevice,
        Registrator, Reporter, RequestChan, Result,
    },
};
use tokio::time::Duration;

pub struct DimmerProperty {
    pub brightness: Option<f64>,
    pub indicator: Option<bool>,
}

/// Defines the common API used by Dimmers.
pub struct Dimmer<R: Reporter> {
    /// This device returns `true` when the driver has a problem
    /// communicating with the hardware.
    pub error: ReadOnlyDevice<bool, R>,
    /// Controls the brightness setting of the dimmer. Off is 0.0 and
    /// full-on is 100.0.
    pub brightness: OverridableDevice<f64, R>,
    /// A product might include an indicator. If the hardware does,
    /// this device can turn it on and off.
    pub indicator: OverridableDevice<bool, R>,
}

impl<R: Reporter> Dimmer<R> {
    // Reports any new properties specified in the `prop` parameter.
    pub async fn report_update(&mut self, prop: DimmerProperty) {
        if let Some(v) = prop.brightness {
            self.brightness.report_update(v).await
        }

        if let Some(v) = prop.indicator {
            self.indicator.report_update(v).await
        }
    }

    pub async fn next_setting(&mut self) -> DimmerProperty {
        tokio::select! {
            Some((value, resp)) = self.brightness.next_setting() => {
                let value = value.clamp(0.0, 100.0);

                if let Some(resp) = resp {
                    resp.ok(value);
                }
                DimmerProperty { brightness: Some(value), indicator: None }
            }
            Some((value, resp)) = self.indicator.next_setting() => {
                if let Some(resp) = resp {
                    resp.ok(value);
                }
                DimmerProperty { brightness: None, indicator: Some(value) }
            }
        }
    }
}

impl<R: Reporter> Registrator<R> for Dimmer<R> {
    type Config = Option<Duration>;

    async fn register_devices(
        drc: &mut RequestChan<R>,
        subpath: Option<&Path>,
        cfg: &Self::Config,
        max_history: Option<usize>,
    ) -> Result<Self> {
        Ok(Dimmer {
            error: drc
                .add_ro_device("error", subpath, None, max_history)
                .await?,
            brightness: drc
                .add_overridable_device(
                    "brightness",
                    subpath,
                    Some("%"),
                    *cfg,
                    max_history,
                )
                .await?,
            indicator: drc
                .add_overridable_device(
                    "indicator",
                    subpath,
                    None,
                    *cfg,
                    max_history,
                )
                .await?,
        })
    }
}

impl<R: Reporter> crate::driver::ResettableState for Dimmer<R> {}

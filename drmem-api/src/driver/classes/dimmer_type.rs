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
        overridable_device::{OverridableDevice, OverrideConfig},
        ro_device::ReadOnlyDevice,
        Registrator, Reporter, RequestChan, Result,
    },
};

pub struct DimmerProperty {
    pub brightness: f64,
}

/// Defines the common API used by Dimmers.
pub struct Dimmer<R: Reporter> {
    // This device returns `true` when the driver has a problem
    // communicating with the hardware.
    error: ReadOnlyDevice<bool, R>,
    // Controls the brightness setting of the dimmer. Off is 0.0 and
    // full-on is 100.0.
    brightness: OverridableDevice<f64, R>,
}

impl<R: Reporter> Dimmer<R> {
    // Reports any new properties specified in the `prop` parameter.
    pub async fn report_update(&mut self, prop: DimmerProperty) {
        self.brightness.report_update(prop.brightness).await;
    }

    pub async fn report_error(&mut self, error: bool) {
        self.error.report_update(error).await;
    }

    pub async fn next_setting(&mut self) -> Option<DimmerProperty> {
        if let Some((value, resp)) = self.brightness.next_setting().await {
            let value = value.clamp(0.0, 100.0);

            if let Some(resp) = resp {
                resp.ok(value);
            }
            return Some(DimmerProperty { brightness: value });
        } else {
            None
        }
    }
}

impl<R: Reporter> Registrator<R> for Dimmer<R> {
    type Config = OverrideConfig;

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
                    cfg.override_duration,
                    cfg.envelope,
                    max_history,
                )
                .await?,
        })
    }
}

impl<R: Reporter> crate::driver::ResettableState for Dimmer<R> {}

//! Define device representation of color LED bulbs.
//!
//! Defines a `Registrator` that registers device names and typed
//! channels to control it. A driver which controls a color bulb
//! should use this instead of registering their own device channels:
//!
//! ```rust,ignore
//! use drmem_api::driver::{self, classes};
//!
//! struct MyBulbDriver { ... };
//!
//! impl driver::API for MyBulbDriver {
//!     type HardwareType = classes::ColorBulb;
//!
//!     ...
//! }
//! ```

use crate::{
    device::{ColorType, Path},
    driver::{
        OverridableDevice, OverrideConfig, ReadOnlyDevice, Registrator,
        Reporter, RequestChan, Result,
    },
};

/// Defines the common API used by Dimmers.
pub struct ColorBulb<R: Reporter> {
    /// This device returns `true` when the driver has a problem
    /// communicating with the hardware.
    pub error: ReadOnlyDevice<bool, R>,
    /// Controls the color of the bulb. Brightness is conveyed via the
    /// color's alpha channel: 0 is off and 255 is full brightness.
    pub color: OverridableDevice<ColorType, R>,
}

impl<R: Reporter> Registrator<R> for ColorBulb<R> {
    type Config = OverrideConfig;

    async fn register_devices(
        drc: &mut RequestChan<R>,
        subpath: Option<&Path>,
        cfg: &Self::Config,
        max_history: Option<usize>,
    ) -> Result<Self> {
        Ok(ColorBulb {
            error: drc
                .add_ro_device("error", subpath, None, max_history)
                .await?,
            color: drc
                .add_overridable_device(
                    "color",
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

impl<R: Reporter> crate::driver::ResettableState for ColorBulb<R> {
    fn reset_state(&mut self) {
        self.color.reset_state();
    }
}

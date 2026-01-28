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

use crate::driver::{
    OverridableDevice, ReadOnlyDevice, Registrator, Reporter, RequestChan,
    Result,
};
use tokio::time::Duration;

/// Defines the common API used by Dimmers.
pub struct ColorBulb<R: Reporter> {
    /// This device returns `true` when the driver has a problem
    /// communicating with the hardware.
    pub error: ReadOnlyDevice<bool, R>,
    /// Controls the brightness setting of the bulb. Off is 0.0 and
    /// full-on is 100.0.
    pub brightness: OverridableDevice<f64, R>,
    pub color: OverridableDevice<palette::LinSrgba<u8>, R>,
}

impl<R: Reporter> Registrator<R> for ColorBulb<R> {
    type Config = Option<Duration>;

    async fn register_devices(
        drc: &mut RequestChan<R>,
        cfg: &Self::Config,
        max_history: Option<usize>,
    ) -> Result<Self> {
        Ok(ColorBulb {
            error: drc.add_ro_device("error", None, max_history).await?,
            brightness: drc
                .add_overridable_device(
                    "brightness",
                    Some("%"),
                    *cfg,
                    max_history,
                )
                .await?,
            color: drc
                .add_overridable_device("color", None, *cfg, max_history)
                .await?,
        })
    }
}

impl<R: Reporter> crate::driver::ResettableState for ColorBulb<R> {
    fn reset_state(&mut self) {
        self.brightness.reset_state();
        self.color.reset_state();
    }
}

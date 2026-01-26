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

use crate::driver::{
    overridable_device::OverridableDevice, ro_device::ReadOnlyDevice,
    Registrator, RequestChan, Result,
};
use tokio::time::Duration;

/// Defines the common API used by Dimmers.
pub struct Dimmer {
    /// This device returns `true` when the driver has a problem
    /// communicating with the hardware.
    pub error: ReadOnlyDevice<bool>,
    /// Controls the brightness setting of the dimmer. Off is 0.0 and
    /// full-on is 100.0.
    pub brightness: OverridableDevice<f64>,
    /// A product might include an indicator. If the hardware does,
    /// this device can turn it on and off.
    pub indicator: OverridableDevice<bool>,
}

impl Registrator for Dimmer {
    type Config = Option<Duration>;

    async fn register_devices(
        drc: &mut RequestChan,
        cfg: &Self::Config,
        max_history: Option<usize>,
    ) -> Result<Self> {
        Ok(Dimmer {
            error: drc.add_ro_device("error", None, max_history).await?,
            brightness: drc
                .add_overridable_device(
                    "brightness",
                    Some("%"),
                    *cfg,
                    max_history,
                )
                .await?,
            indicator: drc
                .add_overridable_device("indicator", None, *cfg, max_history)
                .await?,
        })
    }
}

impl crate::driver::ResettableState for Dimmer {}

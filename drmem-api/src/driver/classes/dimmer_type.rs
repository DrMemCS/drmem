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
    ro_device::ReadOnlyDevice, rw_device::ReadWriteDevice, DriverConfig,
    Registrator, RequestChan, Result,
};
use std::future::Future;

/// Defines the common API used by Dimmers.
pub struct Dimmer {
    /// This device returns `true` when the driver has a problem
    /// communicating with the hardware.
    pub error: ReadOnlyDevice<bool>,
    /// Controls the brightness setting of the dimmer. Off is 0.0 and
    /// full-on is 100.0.
    pub brightness: ReadWriteDevice<f64>,
    /// A product might include an indicator. If the hardware does,
    /// this device can turn it on and off.
    pub indicator: ReadWriteDevice<bool>,
}

impl Registrator for Dimmer {
    fn register_devices<'a>(
        drc: &'a mut RequestChan,
        _cfg: &DriverConfig,
        max_history: Option<usize>,
    ) -> impl Future<Output = Result<Self>> + Send + 'a {
        let nm_error = "error".parse();
        let nm_brightness = "brightness".parse();
        let nm_indicator = "indicator".parse();

        async move {
            // Report any errors before creating any device channels.

            let nm_error = nm_error?;
            let nm_brightness = nm_brightness?;
            let nm_indicator = nm_indicator?;

            // Build the set of channels.

            Ok(Dimmer {
                error: drc
                    .add_ro_device::<bool>(nm_error, None, max_history)
                    .await?,
                brightness: drc
                    .add_rw_device::<f64>(nm_brightness, Some("%"), max_history)
                    .await?,
                indicator: drc
                    .add_rw_device::<bool>(nm_indicator, None, max_history)
                    .await?,
            })
        }
    }
}

//! Define device representation of wall switches.
//!
//! Defines a `Registrator` that registers a device name and typed
//! channel to control it. A driver which controls a switch should use
//! this instead of registering their own device channels:
//!
//! ```rust,ignore
//! use drmem_api::driver::{self, classes};
//!
//! struct MySwitchDriver { ... };
//!
//! impl driver::API for MySwitchDriver {
//!     type HardwareType = classes::Switch;
//!
//!     ...
//! }
//! ```

use crate::driver::{
    rw_device::ReadWriteDevice, DriverConfig, Registrator, RequestChan, Result,
};
use std::future::Future;

// Define a "marker" trait for registering switches.

pub struct Switch;

impl Registrator for Switch {
    type DeviceSet = ReadWriteDevice<bool>;

    fn register_devices<'a>(
        drc: &'a mut RequestChan,
        _cfg: &DriverConfig,
        max_history: Option<usize>,
    ) -> impl Future<Output = Result<Self::DeviceSet>> + Send + 'a {
        async move {
            drc.add_rw_device::<bool>("state".parse()?, None, max_history)
                .await
        }
    }
}

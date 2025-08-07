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

/// Defines the common API used by Switches.
pub struct Switch {
    /// Indicates the state of the switch. Writing `true` or `false`
    /// turns the switch on and off, respectively.
    pub state: ReadWriteDevice<bool>,
}

impl Registrator for Switch {
    fn register_devices<'a>(
        drc: &'a mut RequestChan,
        _cfg: &DriverConfig,
        max_history: Option<usize>,
    ) -> impl Future<Output = Result<Self>> + Send + 'a {
        async move {
            Ok(Switch {
                state: drc
                    .add_rw_device::<bool>("state".parse()?, None, max_history)
                    .await?,
            })
        }
    }
}

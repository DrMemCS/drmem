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
    ro_device::ReadOnlyDevice, shared_rw_device::SharedReadWriteDevice,
    DriverConfig, Registrator, RequestChan, Result,
};
use tokio::time::Duration;

/// Defines the common API used by Switches.
pub struct Switch {
    /// This device returns `true` when the driver has a problem
    /// communicating with the hardware.
    pub error: ReadOnlyDevice<bool>,
    /// Indicates the state of the switch. Writing `true` or `false`
    /// turns the switch on and off, respectively.
    pub state: SharedReadWriteDevice<bool>,
    /// A product might include an indicator. If the hardware does,
    /// this device can turn it on and off.
    pub indicator: SharedReadWriteDevice<bool>,
}

impl Registrator for Switch {
    async fn register_devices(
        drc: &mut RequestChan,
        _cfg: &DriverConfig,
        override_timeout: Option<Duration>,
        max_history: Option<usize>,
    ) -> Result<Self> {
        let nm_error = "error".parse()?;
        let nm_state = "state".parse()?;
        let nm_indicator = "indicator".parse()?;

        Ok(Switch {
            error: drc
                .add_ro_device::<bool>(nm_error, None, max_history)
                .await?,
            state: drc
                .add_shared_rw_device::<bool>(
                    nm_state,
                    None,
                    override_timeout,
                    max_history,
                )
                .await?,
            indicator: drc
                .add_shared_rw_device::<bool>(
                    nm_indicator,
                    None,
                    override_timeout,
                    max_history,
                )
                .await?,
        })
    }
}

impl crate::driver::ResettableState for Switch {}

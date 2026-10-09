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

use crate::{
    device::Path,
    driver::{
        overridable_device::{OverridableDevice, OverrideConfig},
        ro_device::ReadOnlyDevice,
        Registrator, Reporter, RequestChan, Result,
    },
};

pub struct SwitchProperty {
    pub state: bool,
}

/// Defines the common API used by Switches.
pub struct Switch<R: Reporter> {
    /// This device returns `true` when the driver has a problem
    /// communicating with the hardware.
    error: ReadOnlyDevice<bool, R>,
    /// Indicates the state of the switch. Writing `true` or `false`
    /// turns the switch on and off, respectively.
    state: OverridableDevice<bool, R>,
}

impl<R: Reporter> Switch<R> {
    // Reports any new properties specified in the `prop` parameter.
    pub async fn report_update(&mut self, prop: SwitchProperty) {
        self.state.report_update(prop.state).await
    }

    pub async fn report_error(&mut self, error: bool) {
        self.error.report_update(error).await
    }

    pub async fn next_setting(&mut self) -> Option<SwitchProperty> {
        let (value, resp) = self.state.next_setting().await?;

        if let Some(resp) = resp {
            resp.ok(value);
        }
        Some(SwitchProperty { state: value })
    }
}

impl<R: Reporter> Registrator<R> for Switch<R> {
    type Config = OverrideConfig;

    async fn register_devices(
        drc: &mut RequestChan<R>,
        subpath: Option<&Path>,
        cfg: &Self::Config,
        max_history: Option<usize>,
    ) -> Result<Self> {
        Ok(Switch {
            error: drc
                .add_ro_device("error", subpath, None, max_history)
                .await?,
            state: drc
                .add_overridable_device(
                    "state",
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

impl<R: Reporter> crate::driver::ResettableState for Switch<R> {}

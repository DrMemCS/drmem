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
    overridable_device::OverridableDevice, ro_device::ReadOnlyDevice,
    Registrator, Reporter, RequestChan, Result,
};
use tokio::time::Duration;

pub struct SwitchProperty {
    pub state: Option<bool>,
    pub indicator: Option<bool>,
}

/// Defines the common API used by Switches.
pub struct Switch<R: Reporter> {
    /// This device returns `true` when the driver has a problem
    /// communicating with the hardware.
    pub error: ReadOnlyDevice<bool, R>,
    /// Indicates the state of the switch. Writing `true` or `false`
    /// turns the switch on and off, respectively.
    pub state: OverridableDevice<bool, R>,
    /// A product might include an indicator. If the hardware does,
    /// this device can turn it on and off.
    pub indicator: OverridableDevice<bool, R>,
}

impl<R: Reporter> Switch<R> {
    // Reports any new properties specified in the `prop` parameter.
    pub async fn report_update(&mut self, prop: SwitchProperty) {
        if let Some(v) = prop.state {
            self.state.report_update(v).await
        }

        if let Some(v) = prop.indicator {
            self.indicator.report_update(v).await
        }
    }

    pub async fn next_setting(&mut self) -> SwitchProperty {
        tokio::select! {
            Some((value, resp)) = self.state.next_setting() => {
                if let Some(resp) = resp {
                    resp.ok(value);
                }
                SwitchProperty { state: Some(value), indicator: None }
            }
            Some((value, resp)) = self.indicator.next_setting() => {
                if let Some(resp) = resp {
                    resp.ok(value);
                }
                SwitchProperty { state: None, indicator: Some(value) }
            }
        }
    }
}

impl<R: Reporter> Registrator<R> for Switch<R> {
    type Config = Option<Duration>;

    async fn register_devices(
        drc: &mut RequestChan<R>,
        cfg: &Self::Config,
        max_history: Option<usize>,
    ) -> Result<Self> {
        Ok(Switch {
            error: drc.add_ro_device("error", None, max_history).await?,
            state: drc
                .add_overridable_device("state", None, *cfg, max_history)
                .await?,
            indicator: drc
                .add_overridable_device("indicator", None, *cfg, max_history)
                .await?,
        })
    }
}

impl<R: Reporter> crate::driver::ResettableState for Switch<R> {}

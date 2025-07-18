use drmem_api::{
    device,
    driver::{self, DriverConfig},
    Error, Result,
};
use std::{convert::Infallible, future::Future, pin::Pin, sync::Arc};
use tokio::sync::Mutex;

// This enum represents the two states in which the latch can be.

#[derive(Debug, PartialEq)]
enum LatchState {
    Idle,
    Tripped,
}

pub struct Instance {
    state: LatchState,
    active_value: device::Value,
    inactive_value: device::Value,
}

pub struct Devices {
    d_output: driver::ReadOnlyDevice<device::Value>,
    d_trigger: driver::ReadWriteDevice<bool>,
    d_reset: driver::ReadWriteDevice<bool>,
}

impl<'a> Instance {
    pub const NAME: &'static str = "latch";

    pub const SUMMARY: &'static str = "Latches between two values.";

    pub const DESCRIPTION: &'static str = include_str!("drv_latch.md");

    /// Creates a new `Instance` instance.
    pub fn new(
        active_value: device::Value,
        inactive_value: device::Value,
    ) -> Instance {
        Instance {
            state: LatchState::Idle,
            active_value,
            inactive_value,
        }
    }

    // Validates the active value parameter.

    fn get_active_value(cfg: &DriverConfig) -> Result<device::Value> {
        match cfg.get("enabled") {
            Some(value) => value.try_into(),
            None => Err(Error::ConfigError(String::from(
                "missing 'enabled' parameter in config",
            ))),
        }
    }

    // Validates the inactive value parameter.

    fn get_inactive_value(cfg: &DriverConfig) -> Result<device::Value> {
        match cfg.get("disabled") {
            Some(value) => value.try_into(),
            None => Err(Error::ConfigError(String::from(
                "missing 'disabled' parameter in config",
            ))),
        }
    }

    // Updates the state based on new inputs. Returns an optional
    // value. If `None`, then nothing needs to be reported.

    fn update_state<'b: 'a>(
        &'b mut self,
        reset: bool,
        delta_trigger: bool,
    ) -> (Option<&'a device::Value>, Option<&'a device::Value>) {
        match self.state {
            LatchState::Idle if !delta_trigger => (None, None),
            LatchState::Idle if reset => {
                (Some(&self.active_value), Some(&self.inactive_value))
            }
            LatchState::Idle => {
                self.state = LatchState::Tripped;
                (Some(&self.active_value), None)
            }

            LatchState::Tripped if reset => {
                self.state = LatchState::Idle;
                (Some(&self.inactive_value), None)
            }

            LatchState::Tripped => (None, None),
        }
    }
}

impl driver::Registrator for Instance {
    type DeviceSet = Devices;

    fn register_devices<'a>(
        core: &'a mut driver::RequestChan,
        _cfg: &DriverConfig,
        max_history: Option<usize>,
    ) -> impl Future<Output = Result<Self::DeviceSet>> + Send + 'a {
        let output_name = "output".parse::<device::Base>().unwrap();
        let trigger_name = "trigger".parse::<device::Base>().unwrap();
        let reset_name = "reset".parse::<device::Base>().unwrap();

        Box::pin(async move {
            // Define the devices managed by this driver.
            //
            // This first device is the output of the timer.

            let d_output =
                core.add_ro_device(output_name, None, max_history).await?;

            let d_trigger =
                core.add_rw_device(trigger_name, None, max_history).await?;

            let d_reset =
                core.add_rw_device(reset_name, None, max_history).await?;

            Ok(Devices {
                d_output,
                d_trigger,
                d_reset,
            })
        })
    }
}

impl driver::API for Instance {
    fn create_instance(
        cfg: &DriverConfig,
    ) -> Pin<Box<dyn Future<Output = Result<Box<Self>>> + Send>> {
        let active_value = Instance::get_active_value(cfg);
        let inactive_value = Instance::get_inactive_value(cfg);

        let fut = async move {
            // Validate the configuration.

            let active_value = active_value?;
            let inactive_value = inactive_value?;

            // Build and return the future.

            Ok(Box::new(Instance::new(active_value, inactive_value)))
        };

        Box::pin(fut)
    }

    fn run<'a>(
        &'a mut self,
        devices: Arc<Mutex<Self::DeviceSet>>,
    ) -> Pin<Box<dyn Future<Output = Infallible> + Send + 'a>> {
        let fut = async move {
            let mut devices = devices.lock().await;

            let mut reset = false;
            let mut trigger = false;

            // Initialize the reported state of the latch.

            devices.d_trigger.report_update(false).await;
            devices.d_reset.report_update(false).await;
            devices
                .d_output
                .report_update(self.inactive_value.clone())
                .await;

            loop {
                let Devices {
                    d_trigger, d_reset, ..
                } = &mut *devices;

                #[rustfmt::skip]
                let result = tokio::select! {
                    Some((b, reply)) = d_trigger.next_setting() => {
                        let result = self.update_state(reset, !trigger && b);

                        reply(Ok(b));
                        devices.d_trigger.report_update(b).await;
                        trigger = b;
                        result
                    }

                    Some((b, reply)) = d_reset.next_setting() => {
                        let result = self.update_state(reset, false);

                        reply(Ok(b));
                        devices.d_reset.report_update(b).await;
                        reset = b;
                        result
                    }
                };

                if let Some(v) = result.0 {
                    devices.d_output.report_update(v.clone()).await;
                    if let Some(v) = result.1 {
                        devices.d_output.report_update(v.clone()).await;
                    }
                }
            }
        };

        Box::pin(fut)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_state_changes() {
        let mut latch = Instance::new(
            device::Value::Bool(true),
            device::Value::Bool(false),
        );

        assert_eq!(latch.state, LatchState::Idle);
        assert_eq!((None, None), latch.update_state(false, false));

        {
            let (a, b) = latch.update_state(false, true);

            assert_eq!(Some(&device::Value::Bool(true)), a);
            assert!(b.is_none());
        }
        assert_eq!(latch.state, LatchState::Tripped);

        {
            let (a, b) = latch.update_state(false, true);

            assert!(a.is_none());
            assert!(b.is_none());
        }
        assert_eq!(latch.state, LatchState::Tripped);

        {
            let (a, b) = latch.update_state(false, false);

            assert!(a.is_none());
            assert!(b.is_none());
        }
        assert_eq!(latch.state, LatchState::Tripped);

        {
            let (a, b) = latch.update_state(false, true);

            assert!(a.is_none());
            assert!(b.is_none());
        }
        assert_eq!(latch.state, LatchState::Tripped);

        {
            let (a, b) = latch.update_state(false, false);

            assert!(a.is_none());
            assert!(b.is_none());
        }
        assert_eq!(latch.state, LatchState::Tripped);

        {
            let (a, b) = latch.update_state(true, false);

            assert_eq!(Some(&device::Value::Bool(false)), a);
            assert!(b.is_none());
        }
        assert_eq!(latch.state, LatchState::Idle);

        {
            let (a, b) = latch.update_state(false, false);

            assert!(a.is_none());
            assert!(b.is_none());
        }
        assert_eq!(latch.state, LatchState::Idle);

        {
            let (a, b) = latch.update_state(true, true);

            assert_eq!(Some(&device::Value::Bool(true)), a);
            assert_eq!(Some(&device::Value::Bool(false)), b);
        }
        assert_eq!(latch.state, LatchState::Idle);
    }
}

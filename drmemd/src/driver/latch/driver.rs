use drmem_api::{device::Value, driver, Result};
use std::convert::Infallible;

use super::{config, device};

// This enum represents the two states in which the latch can be.

#[derive(Debug, PartialEq)]
enum LatchState {
    Idle,
    Tripped,
}

pub struct Instance {
    state: LatchState,
    active_value: Value,
    inactive_value: Value,
}

impl<'a> Instance {
    pub const NAME: &'static str = "latch";

    pub const SUMMARY: &'static str = "Latches between two values.";

    pub const DESCRIPTION: &'static str = include_str!("drv_latch.md");

    /// Creates a new `Instance` instance.
    pub fn new(active_value: Value, inactive_value: Value) -> Instance {
        Instance {
            state: LatchState::Idle,
            active_value,
            inactive_value,
        }
    }

    // Updates the state based on new inputs. Returns an optional
    // value. If `None`, then nothing needs to be reported.

    fn update_state<'b: 'a>(
        &'b mut self,
        reset: bool,
        delta_trigger: bool,
    ) -> (Option<&'a Value>, Option<&'a Value>) {
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

impl driver::API for Instance {
    type Config = config::Params;
    type HardwareType = device::Set;

    async fn create_instance(cfg: &Self::Config) -> Result<Box<Self>> {
        // Build and return the future.

        Ok(Box::new(Instance::new(
            cfg.enabled.clone(),
            cfg.disabled.clone(),
        )))
    }

    async fn run(&mut self, devices: &mut Self::HardwareType) -> Infallible {
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
            let device::Set {
                d_trigger, d_reset, ..
            } = &mut *devices;

            #[rustfmt::skip]
            let result = tokio::select! {
                Some((b, reply)) = d_trigger.next_setting() => {
                    let result = self.update_state(reset, !trigger && b);

                    reply.ok(b);
                    devices.d_trigger.report_update(b).await;
                    trigger = b;
                    result
                }

                Some((b, reply)) = d_reset.next_setting() => {
                    let result = self.update_state(reset, false);

                    reply.ok(b);
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_state_changes() {
        let mut latch = Instance::new(Value::Bool(true), Value::Bool(false));

        assert_eq!(latch.state, LatchState::Idle);
        assert_eq!((None, None), latch.update_state(false, false));

        {
            let (a, b) = latch.update_state(false, true);

            assert_eq!(Some(&Value::Bool(true)), a);
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

            assert_eq!(Some(&Value::Bool(false)), a);
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

            assert_eq!(Some(&Value::Bool(true)), a);
            assert_eq!(Some(&Value::Bool(false)), b);
        }
        assert_eq!(latch.state, LatchState::Idle);
    }
}

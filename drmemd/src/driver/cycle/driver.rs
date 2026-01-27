use drmem_api::{
    device::Value,
    driver::{self},
    Result,
};
use std::convert::Infallible;
use tokio::time::{self, Duration};
use tracing::{self, debug};

use super::{config, device};

// This enum represents the three states in which the device can be.

#[derive(Debug, PartialEq)]
enum CycleState {
    Idle,
    Cycling,
}

// The state of a driver instance.

pub struct Instance {
    enabled_at_boot: bool,
    disabled: Value,
    enabled: Vec<Value>,
    state: CycleState,
    index: usize,
    millis: Duration,
}

impl Instance {
    pub const NAME: &'static str = "cycle";

    pub const SUMMARY: &'static str =
        "Provides a cycling output that can be enabled and disabled.";

    pub const DESCRIPTION: &'static str = include_str!("drv_cycle.md");

    /// Creates a new, idle `Instance`.
    pub fn new(
        enabled_at_boot: bool,
        millis: Duration,
        disabled: Value,
        enabled: Vec<Value>,
    ) -> Instance {
        Instance {
            enabled_at_boot,
            state: CycleState::Idle,
            disabled,
            enabled,
            index: 0,
            millis,
        }
    }

    fn time_expired(&mut self) -> Option<Value> {
        match self.state {
            CycleState::Idle => None,

            CycleState::Cycling => {
                let current = &self.enabled[self.index];

                self.index = (self.index + 1) % self.enabled.len();

                if &self.enabled[self.index] != current {
                    Some(self.enabled[self.index].clone())
                } else {
                    None
                }
            }
        }
    }

    // Updates the state based on new `enable`. Returns a 2-tuple
    // where the first element is a boolean which indicates whether
    // the interval timer should be reset. The second element is the
    // value with to set the output. If `None`, the output remains
    // unchanged.

    fn update_state(&mut self, val: bool) -> (bool, Option<Value>) {
        match self.state {
            CycleState::Idle => {
                if val {
                    self.state = CycleState::Cycling;
                    self.index = 0;

                    let value = &self.enabled[self.index];

                    (
                        true,
                        // If the output is already at the desired
                        // value, don't emit it again.
                        if value != &self.disabled {
                            Some(value.clone())
                        } else {
                            None
                        },
                    )
                } else {
                    (false, None)
                }
            }

            CycleState::Cycling => (
                false,
                if val {
                    None
                } else {
                    self.state = CycleState::Idle;

                    // If the output is already at the desired value,
                    // don't emit it again.

                    if self.disabled != self.enabled[self.index] {
                        Some(self.disabled.clone())
                    } else {
                        None
                    }
                },
            ),
        }
    }
}

impl driver::API for Instance {
    type Config = config::Params;
    type HardwareType = device::Set;

    async fn create_instance(cfg: &Self::Config) -> Result<Box<Self>> {
        Ok(Box::new(Instance::new(
            cfg.enabled_at_boot,
            Duration::from_millis(cfg.millis),
            cfg.disabled.clone(),
            cfg.enabled.clone(),
        )))
    }

    async fn run(&mut self, devices: &mut Self::HardwareType) -> Infallible {
        let mut timer = time::interval(self.millis);

        if self.enabled_at_boot {
            self.state = CycleState::Cycling;
            devices.d_enable.report_update(true).await;
            devices
                .d_output
                .report_update(self.enabled[self.index].clone())
                .await;
        } else {
            devices.d_enable.report_update(false).await;
            devices.d_output.report_update(self.disabled.clone()).await;
        }

        loop {
            debug!("state {:?} : waiting for event", &self.state);

            #[rustfmt::skip]
            tokio::select! {
                // If the driver is in a timing cycle, add the sleep
                // future to the list of futures to await.

                _ = timer.tick() => {

		    // If the timeout occurs, update the state and set
		    // the output to the inactive value.

		    if let Some(v) = self.time_expired() {
			debug!("state {:?} : timeout occurred -- output {}",
			       &self.state, v);
			devices.d_output.report_update(v).await;
		    }
                }

                // Always look for settings. We're pattern matching
                // so, if all clients close their handles, this branch
                // will forever be disabled. That should never happen
                // since one handle is saved in the device look-up
                // table. All other handles are cloned from it.

                Some((b, reply)) = devices.d_enable.next_setting() => {
                    let (reset, out) = self.update_state(b);

                    if reset {
			timer.reset()
                    }

                    reply.ok(b);

                    debug!("state {:?} : new input -> {}",
			   &self.state, b);

                    devices.d_enable.report_update(b).await;

                    if let Some(out) = out {
			devices.d_output.report_update(out.clone()).await;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::Duration;

    #[tokio::test]
    async fn test_cfg() {
        {
            let cfg = toml::from_str::<config::Params>(
                "millis = 500
enabled_at_boot = true
disabled = false
enabled = [true, false]",
            )
            .unwrap();

            assert_eq!(cfg.enabled_at_boot, true);
            assert_eq!(cfg.millis, 500);
            assert_eq!(cfg.disabled, Value::Bool(false));
            assert_eq!(
                cfg.enabled,
                vec![Value::Bool(true), Value::Bool(false)]
            );
        }

        {
            let cfg = toml::from_str::<config::Params>(
                "millis = 500
disabled = false
enabled = [true, false]
",
            )
            .unwrap();

            assert_eq!(cfg.enabled_at_boot, false);
            assert_eq!(cfg.millis, 500);
            assert_eq!(cfg.disabled, Value::Bool(false));
            assert_eq!(
                cfg.enabled,
                vec![Value::Bool(true), Value::Bool(false)]
            );
        }

        {
            let cfg = toml::from_str::<config::Params>(
                "
disabled = false
enabled = [true, false]",
            );

            assert!(cfg.is_err());
        }

        {
            let cfg = toml::from_str::<config::Params>(
                "
millis = 500
enabled = [true, false]",
            );

            assert!(cfg.is_err());
        }

        {
            let cfg = toml::from_str::<config::Params>(
                "
millis = 500
disabled = false",
            );

            assert!(cfg.is_err());
        }

        {
            let cfg = toml::from_str::<config::Params>(
                "millis = 500
disabled = false
enabled = [true, false]",
            )
            .unwrap();

            assert_eq!(cfg.enabled_at_boot, false);
            assert_eq!(cfg.millis, 500);
            assert_eq!(cfg.disabled, Value::Bool(false));
            assert_eq!(
                cfg.enabled,
                vec![Value::Bool(true), Value::Bool(false)]
            );
        }
    }

    #[test]
    fn test_state_changes() {
        let mut timer = Instance::new(
            false,
            Duration::from_millis(1000),
            Value::Bool(false),
            vec![Value::Bool(true), Value::Bool(false)],
        );

        // Verify that, when in the Idle state, an input of `false` or
        // a timer timeout doesn't move the FSM out of the Idle state.

        assert_eq!(timer.state, CycleState::Idle);
        assert_eq!((false, None), timer.update_state(false));
        assert_eq!(None, timer.time_expired());
        assert_eq!(timer.state, CycleState::Idle);

        // Verify that a `true` input in the Idle state requires the
        // timer to be reset and the output to be reported. Verify a
        // second `true` has no effect.

        assert_eq!((true, Some(true.into())), timer.update_state(true));
        assert_eq!((false, None), timer.update_state(true));

        // Verify timeouts result in the toggling of the output.

        assert_eq!(Some(false.into()), timer.time_expired());
        assert_eq!(Some(true.into()), timer.time_expired());
        assert_eq!(Some(false.into()), timer.time_expired());
        assert_eq!(Some(true.into()), timer.time_expired());

        // Verify that, while cycling, a `false` input brings us back
        // to the Idle state.

        assert_eq!((false, Some(false.into())), timer.update_state(false));
        assert_eq!(timer.state, CycleState::Idle);
    }

    #[test]
    fn test_numeric_cycles() {
        let mut timer = Instance::new(
            false,
            Duration::from_millis(1000),
            Value::Int(0),
            vec![Value::Int(1), Value::Int(2), Value::Int(3)],
        );

        // Verify that, when in the Idle state, an input of `false` or
        // a timer timeout doesn't move the FSM out of the Idle state.

        assert_eq!(timer.state, CycleState::Idle);
        assert_eq!((false, None), timer.update_state(false));
        assert_eq!(None, timer.time_expired());
        assert_eq!(timer.state, CycleState::Idle);

        // Verify that a `true` input in the Idle state requires the
        // timer to be reset and the output to be reported. Verify a
        // second `true` has no effect.

        assert_eq!((true, Some((1).into())), timer.update_state(true));
        assert_eq!((false, None), timer.update_state(true));

        // Verify timeouts result in the cycling of the output.

        assert_eq!(Some((2).into()), timer.time_expired());
        assert_eq!(Some((3).into()), timer.time_expired());
        assert_eq!(Some((1).into()), timer.time_expired());
        assert_eq!(Some((2).into()), timer.time_expired());

        // Verify that, while cycling, a `false` input brings us back
        // to the Idle state.

        assert_eq!((false, Some((0).into())), timer.update_state(false));
        assert_eq!(timer.state, CycleState::Idle);

        // Now verify restarting the cycle emits the first value in
        // the array.

        assert_eq!((true, Some((1).into())), timer.update_state(true));
    }

    #[test]
    fn test_duplicate_skips() {
        let mut timer = Instance::new(
            false,
            Duration::from_millis(1000),
            Value::Bool(false),
            vec![Value::Bool(true), Value::Bool(false), Value::Bool(false)],
        );

        // Verify that, when in the Idle state, an input of `false` or
        // a timer timeout doesn't move the FSM out of the Idle state.

        assert_eq!(timer.state, CycleState::Idle);
        assert_eq!((false, None), timer.update_state(false));
        assert_eq!(None, timer.time_expired());
        assert_eq!(timer.state, CycleState::Idle);

        // Verify that a `true` input in the Idle state requires the
        // timer to be reset and the output to be reported. Verify a
        // second `true` has no effect.

        assert_eq!((true, Some(true.into())), timer.update_state(true));
        assert_eq!((false, None), timer.update_state(true));

        // Verify timeouts result in the toggling of the output.

        assert_eq!(Some(false.into()), timer.time_expired());
        assert_eq!(None, timer.time_expired());
        assert_eq!(Some(true.into()), timer.time_expired());
        assert_eq!(Some(false.into()), timer.time_expired());
        assert_eq!(None, timer.time_expired());
        assert_eq!(Some(true.into()), timer.time_expired());

        // Verify that, while cycling, a `false` input brings us back
        // to the Idle state.

        assert_eq!((false, Some(false.into())), timer.update_state(false));
        assert_eq!(timer.state, CycleState::Idle);
    }

    #[test]
    fn test_transition_skips() {
        let mut timer = Instance::new(
            false,
            Duration::from_millis(1000),
            Value::Bool(false),
            vec![Value::Bool(false), Value::Bool(true), Value::Bool(false)],
        );

        // Verify that, when in the Idle state, an input of `false` or
        // a timer timeout doesn't move the FSM out of the Idle state.

        assert_eq!(timer.state, CycleState::Idle);
        assert_eq!((false, None), timer.update_state(false));
        assert_eq!(None, timer.time_expired());
        assert_eq!(timer.state, CycleState::Idle);

        // Verify that a `true` input in the Idle state requires the
        // timer to be reset but the output is skipped (because it's
        // the same value.) Verify a second `true` has no effect.

        assert_eq!((true, None), timer.update_state(true));
        assert_eq!((false, None), timer.update_state(true));

        // Verify timeouts result in the toggling of the output.

        assert_eq!(Some(true.into()), timer.time_expired());
        assert_eq!(Some(false.into()), timer.time_expired());
        assert_eq!(None, timer.time_expired());
        assert_eq!(Some(true.into()), timer.time_expired());
        assert_eq!(Some(false.into()), timer.time_expired());

        // Verify that, while cycling, a `false` input brings us back
        // to the Idle state and the output is skipped because we're
        // already at `false`.

        assert_eq!((false, None), timer.update_state(false));
        assert_eq!(timer.state, CycleState::Idle);
    }
}

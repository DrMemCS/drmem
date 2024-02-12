use drmem_api::{
    device,
    driver::{self, DriverConfig},
    Error, Result,
};
use std::{convert::Infallible, future::Future, pin::Pin, sync::Arc};
use tokio::{sync::Mutex, time};
use tokio_stream::StreamExt;
use tracing::{self, debug};

// This enum represents the three states in which the device can be.

#[derive(Debug, PartialEq)]
enum CycleState {
    Idle,
    Cycling,
}

// The state of a driver instance.

pub struct Instance {
    enabled_at_boot: bool,
    disabled: device::Value,
    enabled: Vec<device::Value>,
    state: CycleState,
    index: usize,
    millis: time::Duration,
}

pub struct Devices {
    d_output: driver::ReportReading<device::Value>,
    d_enable: driver::ReportReading<bool>,
    s_enable: driver::SettingStream<bool>,
}

impl Instance {
    pub const NAME: &'static str = "cycle";

    pub const SUMMARY: &'static str =
        "Provides a cycling output that can be enabled and disabled.";

    pub const DESCRIPTION: &'static str = include_str!("drv_cycle.md");

    /// Creates a new, idle `Instance`.

    pub fn new(
        enabled_at_boot: bool,
        millis: time::Duration,
        disabled: device::Value,
        enabled: Vec<device::Value>,
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

    // Validates the time duration from the driver configuration.

    fn get_cfg_millis(cfg: &DriverConfig) -> Result<time::Duration> {
        match cfg.get("millis") {
            Some(toml::value::Value::Integer(millis)) => {
                // DrMem's official sample rate is 20 Hz, so the cycle
                // shouldn't change faster than that. Limit the
                // `cycle` driver's output to 20 hz so we can see the
                // output change 20 times a second.
                //
                // XXX: Should there be a global constant in the
                // drmem-api crate indicating the max sample rate?

                if (50..=3_600_000).contains(millis) {
                    Ok(time::Duration::from_millis(*millis as u64))
                } else {
                    Err(Error::ConfigError(String::from(
                        "'millis' out of range",
                    )))
                }
            }
            Some(_) => Err(Error::ConfigError(String::from(
                "'millis' config parameter should be an integer",
            ))),
            None => Err(Error::ConfigError(String::from(
                "missing 'millis' parameter in config",
            ))),
        }
    }

    // Validates the enable-at-boot parameter.

    fn get_cfg_enabled(cfg: &DriverConfig) -> Result<bool> {
        match cfg.get("enabled_at_boot") {
            Some(toml::value::Value::Boolean(level)) => Ok(*level),
            Some(_) => Err(Error::ConfigError(String::from(
                "'enabled_at_boot' config parameter should be a boolean",
            ))),
            None => Ok(false),
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

    fn get_active_values(cfg: &DriverConfig) -> Result<Vec<device::Value>> {
        match cfg.get("enabled") {
            Some(toml::value::Value::Array(value)) => {
                if value.len() > 1 {
                    value.iter().map(|v| v.try_into()).collect()
                } else {
                    Err(Error::ConfigError(String::from(
                        "'enabled' array should have at least 2 values",
                    )))
                }
            }
            Some(_) => Err(Error::ConfigError(String::from(
                "'enabled' parameter should be an array of values",
            ))),
            None => Err(Error::ConfigError(String::from(
                "missing 'enabled' parameter in config",
            ))),
        }
    }

    fn time_expired(&mut self) -> Option<device::Value> {
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

    fn update_state(&mut self, val: bool) -> (bool, Option<device::Value>) {
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
    type DeviceSet = Devices;

    fn register_devices(
        core: driver::RequestChan,
        _cfg: &DriverConfig,
        max_history: Option<usize>,
    ) -> Pin<Box<dyn Future<Output = Result<Self::DeviceSet>> + Send>> {
        let output_name = "output".parse::<device::Base>().unwrap();
        let enable_name = "enable".parse::<device::Base>().unwrap();

        Box::pin(async move {
            // Define the devices managed by this driver.
            //
            // This first device is the output signal. It toggles
            // between `false` and `true` at a rate determined by
            // the `interval` config option.

            let (d_output, _) =
                core.add_ro_device(output_name, None, max_history).await?;

            // This device is settable. Any time it transitions
            // from `false` to `true`, the output device begins a
            // cycling.  When this device is set to `false`, the
            // device stops cycling.

            let (d_enable, rx_set, _) =
                core.add_rw_device(enable_name, None, max_history).await?;

            Ok(Devices {
                d_output,
                d_enable,
                s_enable: rx_set,
            })
        })
    }

    fn create_instance(
        cfg: &DriverConfig,
    ) -> Pin<Box<dyn Future<Output = Result<Box<Self>>> + Send>> {
        let millis = Instance::get_cfg_millis(cfg);
        let enabled_at_boot = Instance::get_cfg_enabled(cfg);
        let disabled = Instance::get_inactive_value(cfg);
        let enabled = Instance::get_active_values(cfg);

        let fut = async move {
            Ok(Box::new(Instance::new(
                enabled_at_boot?,
                millis?,
                disabled?,
                enabled?,
            )))
        };

        Box::pin(fut)
    }

    fn run<'a>(
        &'a mut self,
        devices: Arc<Mutex<Self::DeviceSet>>,
    ) -> Pin<Box<dyn Future<Output = Infallible> + Send + 'a>> {
        let fut = async move {
            let mut timer = time::interval(self.millis);
            let mut devices = devices.lock().await;

            if self.enabled_at_boot {
                self.state = CycleState::Cycling;
                (devices.d_enable)(true).await;
                (devices.d_output)(self.enabled[self.index].clone()).await;
            } else {
                (devices.d_enable)(false).await;
                (devices.d_output)(self.disabled.clone()).await;
            }

            loop {
                debug!("state {:?} : waiting for event", &self.state);

                #[rustfmt::skip]
                tokio::select! {
                    // If the driver is in a timing cycle, add the
                    // sleep future to the list of futures to await.

                    _ = timer.tick() => {

			// If the timeout occurs, update the state and
			// set the output to the inactive value.

			if let Some(v) = self.time_expired() {
			    debug!("state {:?} : timeout occurred -- output {}",
				   &self.state, v);
			    (devices.d_output)(v).await;
			}
                    }

                    // Always look for settings. We're pattern
                    // matching so, if all clients close their
                    // handles, this branch will forever be
                    // disabled. That should never happen since one
                    // handle is saved in the device look-up
                    // table. All other handles are cloned from it.

                    Some((b, reply)) = devices.s_enable.next() => {
                        let (reset, out) = self.update_state(b);

                        if reset {
			    timer.reset()
                        }

                        reply(Ok(b));

                        debug!("state {:?} : new input -> {}",
			       &self.state, b);

                        (devices.d_enable)(b).await;

                        if let Some(out) = out {
			    (devices.d_output)(out.clone()).await;
                        }
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
    use drmem_api::driver::API;
    use std::time::Duration;

    #[tokio::test]
    async fn test_cfg() {
        {
            let mut cfg = DriverConfig::new();

            cfg.insert("millis".to_owned(), toml::value::Value::Integer(500));
            cfg.insert(
                "enabled_at_boot".to_owned(),
                toml::value::Value::Boolean(true),
            );
            cfg.insert(
                "disabled".to_owned(),
                toml::value::Value::Boolean(false),
            );
            cfg.insert(
                "enabled".to_owned(),
                toml::value::Value::Array(vec![
                    toml::value::Value::Boolean(true),
                    toml::value::Value::Boolean(false),
                ]),
            );

            let inst = Instance::create_instance(&cfg).await.unwrap();

            assert_eq!(inst.enabled_at_boot, true);
            assert_eq!(inst.millis, Duration::from_millis(500));
            assert_eq!(inst.disabled, device::Value::Bool(false));
            assert_eq!(
                inst.enabled,
                vec![device::Value::Bool(true), device::Value::Bool(false)]
            );
        }

        {
            let mut cfg = DriverConfig::new();

            cfg.insert("millis".to_owned(), toml::value::Value::Integer(500));
            cfg.insert(
                "disabled".to_owned(),
                toml::value::Value::Boolean(false),
            );
            cfg.insert(
                "enabled".to_owned(),
                toml::value::Value::Array(vec![
                    toml::value::Value::Boolean(true),
                    toml::value::Value::Boolean(false),
                ]),
            );

            let inst = Instance::create_instance(&cfg).await.unwrap();

            assert_eq!(inst.enabled_at_boot, false);
            assert_eq!(inst.millis, Duration::from_millis(500));
            assert_eq!(inst.disabled, device::Value::Bool(false));
            assert_eq!(
                inst.enabled,
                vec![device::Value::Bool(true), device::Value::Bool(false)]
            );
        }

        {
            let mut cfg = DriverConfig::new();

            cfg.insert(
                "disabled".to_owned(),
                toml::value::Value::Boolean(false),
            );
            cfg.insert(
                "enabled".to_owned(),
                toml::value::Value::Array(vec![
                    toml::value::Value::Boolean(true),
                    toml::value::Value::Boolean(false),
                ]),
            );

            assert!(Instance::create_instance(&cfg).await.is_err());
        }

        {
            let mut cfg = DriverConfig::new();

            cfg.insert("millis".to_owned(), toml::value::Value::Integer(500));
            cfg.insert(
                "enabled".to_owned(),
                toml::value::Value::Array(vec![
                    toml::value::Value::Boolean(true),
                    toml::value::Value::Boolean(false),
                ]),
            );

            assert!(Instance::create_instance(&cfg).await.is_err());
        }

        {
            let mut cfg = DriverConfig::new();

            cfg.insert("millis".to_owned(), toml::value::Value::Integer(500));
            cfg.insert(
                "disabled".to_owned(),
                toml::value::Value::Boolean(false),
            );

            assert!(Instance::create_instance(&cfg).await.is_err());
        }

        {
            let mut cfg = DriverConfig::new();

            cfg.insert("millis".to_owned(), toml::value::Value::Integer(500));
            cfg.insert(
                "disabled".to_owned(),
                toml::value::Value::Boolean(false),
            );
            cfg.insert(
                "enabled".to_owned(),
                toml::value::Value::Array(vec![toml::value::Value::Boolean(
                    true,
                )]),
            );

            assert!(Instance::create_instance(&cfg).await.is_err());
        }
    }

    #[test]
    fn test_state_changes() {
        let mut timer = Instance::new(
            false,
            time::Duration::from_millis(1000),
            device::Value::Bool(false),
            vec![device::Value::Bool(true), device::Value::Bool(false)],
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
            time::Duration::from_millis(1000),
            device::Value::Int(0),
            vec![
                device::Value::Int(1),
                device::Value::Int(2),
                device::Value::Int(3),
            ],
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
            time::Duration::from_millis(1000),
            device::Value::Bool(false),
            vec![
                device::Value::Bool(true),
                device::Value::Bool(false),
                device::Value::Bool(false),
            ],
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
            time::Duration::from_millis(1000),
            device::Value::Bool(false),
            vec![
                device::Value::Bool(false),
                device::Value::Bool(true),
                device::Value::Bool(false),
            ],
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

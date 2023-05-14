use drmem_api::{
    device,
    driver::{self, DriverConfig},
    Error, Result,
};
use std::{convert::Infallible, future::Future, pin::Pin, sync::Arc};
use tokio::{sync::Mutex, time};
use tokio_stream::StreamExt;
use tracing::{debug, info};

// This enum represents the four states in which the timer can
// be. They are a combination of the `enable` input and whether we're
// timing or not.

#[derive(Debug, PartialEq)]
enum TimerState {
    Armed,          // Not timing, input is false
    Timing,         // Timing, input is true
    TimingAndArmed, // Timing, input is false
    TimedOut,       // Not timing, input is true
}

pub struct Instance {
    state: TimerState,
    active_level: bool,
    millis: time::Duration,
}

pub struct Devices {
    d_output: driver::ReportReading<bool>,
    d_enable: driver::ReportReading<bool>,
    s_enable: driver::SettingStream<bool>,
}

impl Instance {
    pub const NAME: &'static str = "timer";

    pub const SUMMARY: &'static str =
        "Activates an output for a length of time.";

    pub const DESCRIPTION: &'static str = include_str!("drv_timer.md");

    /// Creates a new `Instance` instance. It is assumed the external
    /// input is `false` so the initial timer state is `Armed`.

    pub fn new(active_level: bool, millis: time::Duration) -> Instance {
        Instance {
            state: TimerState::Armed,
            active_level,
            millis,
        }
    }

    // Returns `true` if we're in a timing state.

    fn timing(&self) -> bool {
        self.state == TimerState::Timing
            || self.state == TimerState::TimingAndArmed
    }

    // Updates the state to a new one reflecting that we're no longer
    // timing.

    fn time_expired(&mut self) {
        if self.state == TimerState::Timing {
            self.state = TimerState::TimedOut;
        } else if self.state == TimerState::TimingAndArmed {
            self.state = TimerState::Armed;
        }
    }

    // Validates the time duration from the driver configuration.

    fn get_cfg_millis(cfg: &DriverConfig) -> Result<time::Duration> {
        match cfg.get("millis") {
            Some(toml::value::Value::Integer(millis)) => {
                if (50..=3_600_000).contains(millis) {
                    Ok(time::Duration::from_millis(*millis as u64))
                } else {
                    Err(Error::BadConfig(String::from("'millis' out of range")))
                }
            }
            Some(_) => Err(Error::BadConfig(String::from(
                "'millis' config parameter should be an integer",
            ))),
            None => Err(Error::BadConfig(String::from(
                "missing 'millis' parameter in config",
            ))),
        }
    }

    // Validates the logic level parameter.

    fn get_cfg_level(cfg: &DriverConfig) -> Result<bool> {
        match cfg.get("active_level") {
            Some(toml::value::Value::Boolean(level)) => Ok(*level),
            Some(_) => Err(Error::BadConfig(String::from(
                "'active_level' config parameter should be a boolean",
            ))),
            None => Err(Error::BadConfig(String::from(
                "missing 'active_level' parameter in config",
            ))),
        }
    }

    // Updates the state based on new `enable`. Returns an optional
    // instant of time with which the caller should use to start a new
    // timer.

    fn update_state(
        &mut self, val: bool,
    ) -> (Option<bool>, Option<time::Instant>) {
        match self.state {
            // Currently timing and the input was set to `false`.
            TimerState::TimingAndArmed => {
                // If the input is `true`, enter the Timing state and
                // return a new timeout value. A user has reset the
                // timer while is was in a previous timing cycle.

                (
                    None,
                    if val {
                        self.state = TimerState::Timing;
                        Some(time::Instant::now() + self.millis)
                    } else {
                        None
                    },
                )
            }

            // Not currently timing, but the input was `false`.
            TimerState::Armed => {
                // If the input is `true`, enter the Timing state, and
                // return a timeout value.

                if val {
                    self.state = TimerState::Timing;
                    (
                        Some(self.active_level),
                        Some(time::Instant::now() + self.millis),
                    )
                } else {
                    (None, None)
                }
            }

            // Currently timing and input is `true`.
            TimerState::Timing => {
                // If the input is `false`, continue with the current
                // timing cycle but enter `TimingAndArmed` because a
                // `true` can restart the timer.

                if !val {
                    self.state = TimerState::TimingAndArmed;
                }

                (None, None)
            }

            // Not timing, input is `true`.
            TimerState::TimedOut => {
                // If the input goes to `false`, the state is `Armed`.

                if !val {
                    self.state = TimerState::Armed;
                }

                (None, None)
            }
        }
    }
}

impl driver::API for Instance {
    type DeviceSet = Devices;

    fn register_devices(
        core: driver::RequestChan, _cfg: &DriverConfig,
        max_history: Option<usize>,
    ) -> Pin<Box<dyn Future<Output = Result<Self::DeviceSet>> + Send>> {
        let output_name = "output".parse::<device::Base>().unwrap();
        let enable_name = "enable".parse::<device::Base>().unwrap();

        Box::pin(async move {
            // Define the devices managed by this driver.
            //
            // This first device is the output of the timer. When
            // it's not timing, this device's value with be
            // `!level`. While it's timing, `level`.

            let (d_output, _) =
                core.add_ro_device(output_name, None, max_history).await?;

            // This device is settable. Any time it transitions
            // from `false` to `true`, the timer begins a timing
            // cycle.

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
        let level = Instance::get_cfg_level(cfg);

        let fut = async move {
            // Validate the configuration.

            let millis = millis?;
            let level = level?;

            // Build and return the future.

            Ok(Box::new(Instance::new(level, millis)))
        };

        Box::pin(fut)
    }

    fn run<'a>(
        &'a mut self, devices: Arc<Mutex<Self::DeviceSet>>,
    ) -> Pin<Box<dyn Future<Output = Infallible> + Send + 'a>> {
        let fut = async move {
            let mut timeout = time::Instant::now();
            let mut devices = devices.lock().await;

            (devices.d_enable)(false).await;
            (devices.d_output)(!self.active_level).await;

            loop {
                info!("state {:?} : waiting for event", &self.state);

                #[rustfmt::skip]
                tokio::select! {
                    // If the driver is in a timing cycle, add the
                    // sleep future to the list of futures to await.

                    _ = time::sleep_until(timeout), if self.timing() => {
			debug!("state {:?} : timeout occurred", &self.state);

			// If the timeout occurs, update the state and
			// set the output to the inactive value.

			self.time_expired();
			(devices.d_output)(!self.active_level).await;
                    }

                    // Always look for settings. We're pattern
                    // matching so, if all clients close their
                    // handles, this branch will forever be
                    // disabled. That should never happen since one
                    // handle is saved in the device look-up
                    // table. All other handles are cloned from it.

                    Some((b, reply)) = devices.s_enable.next() => {
                        let (out, tmo) = self.update_state(b);

                        reply(Ok(b));

                        debug!("state {:?} : new input -> {}", &self.state, b);

                        if let Some(tmo) = tmo {
			    timeout = tmo
                        }

                        (devices.d_enable)(b).await;

                        if let Some(out) = out {
			    (devices.d_output)(out).await;
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
    use tokio::time;

    #[test]
    fn test_state_changes() {
        let mut timer = Instance::new(true, time::Duration::from_millis(1000));

        assert_eq!(timer.state, TimerState::Armed);
        assert_eq!((None, None), timer.update_state(false));

        let (a, b) = timer.update_state(true);

        assert_eq!(timer.state, TimerState::Timing);
        assert_eq!(Some(true), a);
        assert!(b.is_some());

        assert_eq!((None, None), timer.update_state(true));
        assert_eq!(timer.state, TimerState::Timing);

        assert_eq!((None, None), timer.update_state(false));
        assert_eq!(timer.state, TimerState::TimingAndArmed);

        assert_eq!((None, None), timer.update_state(false));
        assert_eq!(timer.state, TimerState::TimingAndArmed);

        let (a, b) = timer.update_state(true);

        assert_eq!(timer.state, TimerState::Timing);
        assert!(a.is_none());
        assert!(b.is_some());

        timer.time_expired();
        assert_eq!(timer.state, TimerState::TimedOut);

        assert_eq!((None, None), timer.update_state(true));
        assert_eq!(timer.state, TimerState::TimedOut);

        assert_eq!((None, None), timer.update_state(false));
        assert_eq!(timer.state, TimerState::Armed);

        let (a, b) = timer.update_state(true);

        assert_eq!(timer.state, TimerState::Timing);
        assert_eq!(Some(true), a);
        assert!(b.is_some());

        assert_eq!((None, None), timer.update_state(false));
        assert_eq!(timer.state, TimerState::TimingAndArmed);

        timer.time_expired();
        assert_eq!(timer.state, TimerState::Armed);
    }
}

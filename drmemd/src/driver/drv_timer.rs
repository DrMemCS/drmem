use drmem_api::{
    device,
    driver::{self, ResettableState},
    Error, Result,
};
use std::convert::Infallible;
use tokio::time::{self, Duration};
use tracing::{debug, info};

#[derive(serde::Deserialize)]
pub struct InstanceConfig {
    millis: u64,
    disabled: device::Value,
    enabled: device::Value,
}

impl TryFrom<driver::DriverConfig> for InstanceConfig {
    type Error = Error;

    fn try_from(
        cfg: driver::DriverConfig,
    ) -> std::result::Result<Self, Self::Error> {
        cfg.parse_into()
    }
}

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
    active_value: device::Value,
    inactive_value: device::Value,
    millis: Duration,
}

impl Instance {
    pub const NAME: &'static str = "timer";

    pub const SUMMARY: &'static str =
        "Activates an output for a length of time.";

    pub const DESCRIPTION: &'static str = include_str!("drv_timer.md");

    /// Creates a new `Instance` instance. It is assumed the external
    /// input is `false` so the initial timer state is `Armed`.
    pub fn new(
        active_value: device::Value,
        inactive_value: device::Value,
        millis: Duration,
    ) -> Instance {
        Instance {
            state: TimerState::Armed,
            active_value,
            inactive_value,
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

    // Updates the state based on new `enable`. Returns an optional
    // instant of time with which the caller should use to start a new
    // timer.

    fn update_state(
        &mut self,
        val: bool,
    ) -> (Option<device::Value>, Option<time::Instant>) {
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
                        Some(self.active_value.clone()),
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

pub struct Devices {
    d_output: driver::ReadOnlyDevice<device::Value>,
    d_enable: driver::ReadWriteDevice<bool>,
}

impl driver::Registrator for Devices {
    type Config = InstanceConfig;

    async fn register_devices(
        core: &mut driver::RequestChan,
        _cfg: &Self::Config,
        max_history: Option<usize>,
    ) -> Result<Self> {
        // Define the devices managed by this driver.
        //
        // This first device is the output of the timer. When it's not
        // timing, this device's value with be `!level`. While it's
        // timing, `level`.

        let d_output = core.add_ro_device("output", None, max_history).await?;

        // This device is settable. Any time it transitions from
        // `false` to `true`, the timer begins a timing cycle.

        let d_enable = core.add_rw_device("enable", None, max_history).await?;

        Ok(Devices { d_output, d_enable })
    }
}

impl driver::API for Instance {
    type Config = InstanceConfig;
    type HardwareType = Devices;

    async fn create_instance(cfg: &Self::Config) -> Result<Box<Self>> {
        Ok(Box::new(Instance::new(
            cfg.enabled.clone(),
            cfg.disabled.clone(),
            Duration::from_millis(cfg.millis),
        )))
    }

    async fn run(&mut self, devices: &mut Self::HardwareType) -> Infallible {
        let mut timeout = time::Instant::now();

        // Initialize the reported state of the timer.

        devices.d_enable.report_update(false).await;
        devices
            .d_output
            .report_update(self.inactive_value.clone())
            .await;

        loop {
            info!("state {:?} : waiting for event", &self.state);

            #[rustfmt::skip]
            tokio::select! {
                // If the driver is in a timing cycle, add the sleep
                // future to the list of futures to await.

                _ = time::sleep_until(timeout), if self.timing() => {
		    debug!("state {:?} : timeout occurred", &self.state);

		    // If the timeout occurs, update the state and set
		    // the output to the inactive value.

		    self.time_expired();
		    devices
                        .d_output
                        .report_update(self.inactive_value.clone())
                        .await;
                }

                // Always look for settings. We're pattern matching
                // so, if all clients close their handles, this branch
                // will forever be disabled. That should never happen
                // since one handle is saved in the device look-up
                // table. All other handles are cloned from it.

                Some((b, reply)) = devices.d_enable.next_setting() => {
                    let (out, tmo) = self.update_state(b);

                    reply.ok(b);

                    debug!("state {:?} : new input -> {}", &self.state, b);

                    if let Some(tmo) = tmo {
			timeout = tmo
                    }

                    devices.d_enable.report_update(b).await;

                    if let Some(out) = out {
			devices.d_output.report_update(out).await;
                    }
                }
            }
        }
    }
}

impl ResettableState for Devices {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_state_changes() {
        let mut timer = Instance::new(
            device::Value::Bool(true),
            device::Value::Bool(false),
            Duration::from_millis(1000),
        );

        assert_eq!(timer.state, TimerState::Armed);
        assert_eq!((None, None), timer.update_state(false));

        let (a, b) = timer.update_state(true);

        assert_eq!(timer.state, TimerState::Timing);
        assert_eq!(Some(device::Value::Bool(true)), a);
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
        assert_eq!(Some(device::Value::Bool(true)), a);
        assert!(b.is_some());

        assert_eq!((None, None), timer.update_state(false));
        assert_eq!(timer.state, TimerState::TimingAndArmed);

        timer.time_expired();
        assert_eq!(timer.state, TimerState::Armed);
    }
}

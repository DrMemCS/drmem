use drmem_api::{
    driver::{self, DriverConfig},
    types::{
        device::{self, Base},
        Error,
    },
    Result,
};
use std::{convert::Infallible, future::Future, pin::Pin};
use tokio::time;
use tracing::{self, debug, error, info, warn};

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
    d_output: driver::ReportReading,
    d_enable: driver::ReportReading,
    s_enable: driver::RxDeviceSetting,
}

impl Instance {
    pub const NAME: &'static str = "timer";

    pub const SUMMARY: &'static str =
        "Activates an output for a length of time.";

    pub const DESCRIPTION: &'static str = include_str!("drv_timer.md");

    /// Creates a new `Instance` instance. It is assumed the external
    /// input is `false` so the initial timer state is `Armed`.

    pub fn new(
        active_level: bool, millis: time::Duration,
        d_output: driver::ReportReading, d_enable: driver::ReportReading,
        s_enable: driver::RxDeviceSetting,
    ) -> Instance {
        Instance {
            state: TimerState::Armed,
            active_level,
            millis,
            d_output,
            d_enable,
            s_enable,
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
                    return Ok(time::Duration::from_millis(*millis as u64));
                } else {
                    error!("'millis' out of range")
                }
            }
            Some(_) => error!("'millis' config parameter should be an integer"),
            None => error!("missing 'millis' parameter in config"),
        }

        Err(Error::BadConfig)
    }

    // Validates the logic level parameter.

    fn get_cfg_level(cfg: &DriverConfig) -> Result<bool> {
        match cfg.get("active_level") {
            Some(toml::value::Value::Boolean(level)) => return Ok(*level),
            Some(_) => {
                error!("'active_level' config parameter should be a boolean")
            }
            None => error!("missing 'active_level' parameter in config"),
        }

        Err(Error::BadConfig)
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
    fn create_instance(
        cfg: DriverConfig, core: driver::RequestChan,
        max_history: Option<usize>,
    ) -> Pin<
        Box<dyn Future<Output = Result<driver::DriverType>> + Send + 'static>,
    > {
        let output_name = "output".parse::<Base>().unwrap();
        let enable_name = "enable".parse::<Base>().unwrap();

        let fut = async move {
            // Validate the configuration.

            let millis = Instance::get_cfg_millis(&cfg)?;
            let level = Instance::get_cfg_level(&cfg)?;

            // Define the devices managed by this driver.
            //
            // This first device is the output of the timer. When it's
            // not timing, this device's value with be `!level`. While
            // it's timing, `level`.

            let (d_output, _) =
                core.add_ro_device(output_name, None, max_history).await?;

            // This device is settable. Any time it transitions from
            // `false` to `true`, the timer begins a timing cycle.

            let (d_enable, rx_set, _) =
                core.add_rw_device(enable_name, None, max_history).await?;

            // Build and return the future.

            Ok(Box::new(Instance::new(
                level, millis, d_output, d_enable, rx_set,
            )) as driver::DriverType)
        };

        Box::pin(fut)
    }

    fn run<'a>(
        &'a mut self,
    ) -> Pin<Box<dyn Future<Output = Infallible> + Send + 'a>> {
        let fut = async {
            let mut timeout = time::Instant::now();

            (self.d_enable)(false.into()).await;
            (self.d_output)((!self.active_level).into()).await;

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
			(self.d_output)((!self.active_level).into()).await;
                    }

                    // Always look for settings. We're pattern
                    // matching so, if all clients close their
                    // handles, this branch will forever be
                    // disabled. That should never happen since one
                    // handle is saved in the device look-up
                    // table. All other handles are cloned from it.

                    Some((v, tx)) = self.s_enable.recv() => {

			// If a client sends us something besides a
			// boolean, return an error and ignore the
			// setting. Otherwise, echo the value back to
			// the client and update the state with the
			// new value.

			if let device::Value::Bool(b) = v {
                            let (out, tmo) = self.update_state(b);
                            let _ = tx.send(Ok(v));

                            debug!("state {:?} : new input -> {}", &self.state, b);

                            if let Some(tmo) = tmo {
				timeout = tmo
                            }

                            (self.d_enable)(b.into()).await;

                            if let Some(out) = out {
				(self.d_output)(out.into()).await;
                            }
			} else {
                            let _ = tx.send(Err(Error::TypeError));

                            warn!("state {:?} : received bad value -> {:?}", &self.state, &v);
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
    use drmem_api::types::device;
    use tokio::{sync::mpsc, time};

    fn fake_report(
        _v: device::Value,
    ) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        Box::pin(async { () })
    }

    #[test]
    fn test_state_changes() {
        let (_tx, rx) = mpsc::channel(20);
        let mut timer = Instance::new(
            true,
            time::Duration::from_millis(1000),
            Box::new(fake_report),
            Box::new(fake_report),
            rx,
        );

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

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
use tracing::{self, debug, error, warn};

// This enum represents the three states in which the device can be.

#[derive(Debug, PartialEq)]
enum CycleState {
    Idle,
    CycleHigh,
    CycleLow,
}

// The state of a driver instance.

pub struct Instance {
    enabled_at_boot: bool,
    state: CycleState,
    millis: time::Duration,
    d_output: driver::ReportReading,
    d_enable: driver::ReportReading,
    s_enable: driver::RxDeviceSetting,
}

impl Instance {
    pub const NAME: &'static str = "cycle";

    pub const SUMMARY: &'static str =
        "Provides a cycling output that can be enableda nd disable.";

    pub const DESCRIPTION: &'static str = include_str!("../README.md");

    /// Creates a new, idle `Instance`.

    pub fn new(
	enabled: bool,
        millis: time::Duration,
        d_output: driver::ReportReading, d_enable: driver::ReportReading,
        s_enable: driver::RxDeviceSetting,
    ) -> Instance {
        Instance {
	    enabled_at_boot: enabled,
            state: CycleState::Idle,
            millis,
            d_output,
            d_enable,
            s_enable,
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

    // Validates the enable-at-boot parameter.

    fn get_cfg_enabled(cfg: &DriverConfig) -> Result<bool> {
        match cfg.get("enabled") {
            Some(toml::value::Value::Boolean(level)) => return Ok(*level),
            Some(_) => {
                error!("'enabled' config parameter should be a boolean")
            }
            None => return Ok(false)
        }

        Err(Error::BadConfig)
    }

    fn time_expired(&mut self) -> Option<bool> {
	match self.state {
            CycleState::Idle => None,

	    CycleState::CycleHigh => {
		self.state = CycleState::CycleLow;
		Some(false)
	    }

	    CycleState::CycleLow => {
		self.state = CycleState::CycleHigh;
		Some(true)
	    }
	}
    }

    // Updates the state based on new `enable`. Returns a 2-tuple
    // where the first element is a boolean which indicates whether
    // the interval timer should be reset. The second element is the
    // value with to set the output. If `None`, the output remains
    // unchanged.

    fn update_state(&mut self, val: bool) -> (bool, Option<bool>) {
        match self.state {
            CycleState::Idle =>
                if val {
                    self.state = CycleState::CycleHigh;
		    (true, Some(true))
		} else {
		    (false, None)
                },

            CycleState::CycleHigh
		| CycleState::CycleLow =>
		(
		    false,
                    if val {
			None
		    } else {
			self.state = CycleState::Idle;
			Some(false)
                    }
		),
        }
    }
}

impl driver::API for Instance {
    fn create_instance(
        cfg: DriverConfig, core: driver::RequestChan,
    ) -> Pin<
        Box<dyn Future<Output = Result<driver::DriverType>> + Send + 'static>,
    > {
        let fut = async move {
            // Validate the configuration.

            let millis = Instance::get_cfg_millis(&cfg)?;
            let enabled = Instance::get_cfg_enabled(&cfg)?;

            // Define the devices managed by this driver.

            let (d_output, _) =
                core.add_ro_device("output".parse::<Base>()?, None).await?;
            let (d_enable, rx_set, _) =
                core.add_rw_device("enable".parse::<Base>()?, None).await?;

            Ok(
                Box::new(Instance::new(enabled, millis, d_output, d_enable, rx_set))
                    as driver::DriverType,
            )
        };

        Box::pin(fut)
    }

    fn run<'a>(
        &'a mut self,
    ) -> Pin<Box<dyn Future<Output = Result<Infallible>> + Send + 'a>> {
        let fut = async {
	    let mut timer = time::interval(self.millis);

	    if self.enabled_at_boot {
		self.state = CycleState::CycleHigh;
		(self.d_enable)(true.into()).await?;
		(self.d_output)(true.into()).await?;
	    } else {
		(self.d_enable)(false.into()).await?;
		(self.d_output)(false.into()).await?;
	    }

            loop {
                debug!("state {:?} : waiting for event", &self.state);

                tokio::select! {
                    // If the driver is in a timing cycle, add the
                    // sleep future to the list of futures to await.

                    _ = timer.tick() => {

			// If the timeout occurs, update the state and
			// set the output to the inactive value.

			if let Some(v) = self.time_expired() {
			    debug!("state {:?} : timeout occurred -- output {}", &self.state, v);
			    (self.d_output)(v.into()).await?
			}
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
                            let (reset, out) = self.update_state(b);

                            if reset {
				timer.reset()
                            }

                            let _ = tx.send(Ok(v));

                            debug!("state {:?} : new input -> {}", &self.state, b);

                            (self.d_enable)(b.into()).await?;

                            if let Some(out) = out {
				(self.d_output)(out.into()).await?;
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
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send>> {
        let fut = async { Ok(()) };

        Box::pin(fut)
    }

    #[test]
    fn test_state_changes() {
        let (_tx, rx) = mpsc::channel(20);
        let mut timer = Instance::new(
	    false,
            time::Duration::from_millis(1000),
            Box::new(fake_report),
            Box::new(fake_report),
            rx,
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

	assert_eq!((true, Some(true)), timer.update_state(true));
        assert_eq!((false, None), timer.update_state(true));

	// Verify timeouts result in the toggling of the output.

	assert_eq!(Some(false), timer.time_expired());
	assert_eq!(Some(true), timer.time_expired());
	assert_eq!(Some(false), timer.time_expired());
	assert_eq!(Some(true), timer.time_expired());

	// Verify that, while cycling, a `false` input brings us back
	// to the Idle state.

        assert_eq!((false, Some(false)), timer.update_state(false));
        assert_eq!(timer.state, CycleState::Idle);
    }
}

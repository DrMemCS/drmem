use drmem_api::{
    driver::{self, classes, DriverConfig},
    Error, Result,
};
use std::convert::{Infallible, TryFrom};
use std::{future::Future, time::SystemTime};
use tokio::time::{interval_at, Duration, Instant};
use tracing::{debug, error, warn, Span};
use weather_underground as wu;

const DEFAULT_INTERVAL: u64 = 10;
const MIN_PUBLIC_INTERVAL: u64 = 10;

// This type defines a mini state machine to help us accumulate
// rainfall. Some weather stations reset their rainfall total at
// midnight -- even if it's still raining! This state machine tries to
// recognize those resets to properly maintain its local precip
// totals.

enum PrecipState {
    NoRain,
    Rain {
        prev: f64,
        running: f64,
        time: SystemTime,
    },
    Pause {
        prev: f64,
    },
}

impl PrecipState {
    fn new() -> Self {
        PrecipState::NoRain
    }

    // This method updates the state of the data based on new
    // readings. It returns values to be reported by the driver for
    // the three precip devices.

    fn update(
        &mut self,
        p_rate: f64,
        p_total: f64,
        now: SystemTime,
    ) -> (f64, f64, Option<f64>) {
        match self {
            // This state models when it isn't raining. It's the
            // initial state and it will be re-entered when the
            // weather station reports no rain.
            Self::NoRain => {
                // If there's a non-zero total, we need to switch to
                // the rain state.

                if p_total > 0.0 {
                    *self = Self::Rain {
                        prev: p_total,
                        running: p_total,
                        time: now,
                    };
                }
                (p_rate, p_total, None)
            }

            // This state is active after the 10 hour time between
            // rainfall has occurred, but the weather station is still
            // reporting a non-zero precip total.
            Self::Pause { prev } => {
                (
                    p_rate,
                    // If the weather station resets its total, we can
                    // go back to the `NoRain` state.
                    if p_total == 0.0 {
                        if p_rate == 0.0 {
                            *self = Self::NoRain;
                        }
                        0.0
                    }
                    // If more rain is reported, then a new system has
                    // rolled in. Go back to the `Rain` state, but set
                    // the currently reported total as the baseline
                    // with which to subtract future readings.
                    else if p_total > *prev {
                        let total = p_total - *prev;

                        *self = Self::Rain {
                            prev: p_total,
                            running: total,
                            time: now,
                        };
                        total
                    }
                    // The total is less than the previous, but not
                    // 0. This means we crossed midnight -- resetting
                    // the total -- but more rain occurred before we
                    // sampled the data. Go into the `Rain` state.
                    else if p_total < *prev {
                        *self = Self::Rain {
                            prev: p_total,
                            running: p_total,
                            time: now,
                        };
                        p_total
                    } else {
                        0.0
                    },
                    None,
                )
            }

            // This state is active while it is raining.
            Self::Rain {
                prev,
                running,
                time,
            } => {
                const TIMEOUT: Duration = Duration::from_secs(36_000);
                let delta = now
                    .duration_since(*time)
                    .unwrap_or_else(|_| Duration::from_secs(0));

                // If the weather station reports no rainfall and
                // reset its total, we emit the total as the value of
                // the last rainfall.

                if p_rate == 0.0 && delta >= TIMEOUT {
                    let last_total = *running;

                    *self = Self::Pause { prev: *prev };
                    (0.0, 0.0, Some(last_total))
                } else {
                    // If the total is less than the previous value,
                    // then we crossed midnight. Just use the reset
                    // value as the delta.

                    *running += if *prev > p_total {
                        p_total
                    } else {
                        p_total - *prev
                    };

                    // If the rainfall rate is 0, don't update the
                    // timeout value.

                    if p_rate > 0.0 {
                        *time = now;
                    }

                    // Update the totals in the state.

                    *prev = p_total;
                    (p_rate, *running, None)
                }
            }
        }
    }
}

pub struct Instance {
    con: reqwest::Client,
    api_key: String,
    interval: Duration,

    precip: PrecipState,
}

impl Instance {
    pub const NAME: &'static str = "weather-wu";

    pub const SUMMARY: &'static str =
        "obtains weather data from Weather Underground";

    pub const DESCRIPTION: &'static str = include_str!("../README.md");

    fn get_cfg_interval(cfg: &DriverConfig) -> Result<u64> {
        match cfg.get("interval") {
            Some(toml::value::Value::Integer(val)) => {
                Ok(std::cmp::max(*val as u64, 1))
            }
            Some(_) => Err(Error::ConfigError(String::from(
                "'interval' config parameter should be a positive integer",
            ))),
            None => Ok(DEFAULT_INTERVAL),
        }
    }

    fn get_cfg_key(cfg: &DriverConfig) -> Result<Option<String>> {
        match cfg.get("key") {
            Some(toml::value::Value::String(val)) => Ok(Some(val.to_string())),
            Some(_) => Err(Error::ConfigError(String::from(
                "'key' config parameter should be a string",
            ))),
            None => Ok(None),
        }
    }

    async fn get_cfg_key_and_interval(
        con: &mut reqwest::Client,
        key: Option<String>,
        interval: u64,
    ) -> Result<(String, Duration)> {
        match key {
            Some(val) => Ok((val, Duration::from_secs(interval * 60))),
            None => {
                if let Ok(api_key) = wu::fetch_api_key(con).await {
                    Ok((
                        api_key,
                        Duration::from_secs(
                            std::cmp::max(interval, MIN_PUBLIC_INTERVAL) * 60,
                        ),
                    ))
                } else {
                    error!("couldn't determine public API key");
                    Err(Error::NotFound)
                }
            }
        }
    }

    // Processes an observation by sending each parameter to the
    // correct device channel. It also does some sanity checks on the
    // values.

    async fn handle(
        &mut self,
        obs: &wu::Observation,
        devices: &mut <Self as driver::API>::HardwareType,
    ) {
        // Retreive all the parameters whose units can change between
        // English and Metric.

        let (dewpt, htidx, prate, ptotal, press, temp, wndchl, wndgst, wndspd) =
            if let classes::Units::Metric = devices.units {
                if let Some(params) = &obs.metric {
                    (
                        params.dewpt,
                        params.heat_index,
                        params.precip_rate,
                        params.precip_total,
                        params.pressure,
                        params.temp,
                        params.wind_chill,
                        params.wind_gust,
                        params.wind_speed,
                    )
                } else {
                    panic!("weather data didn't return any metric data")
                }
            } else if let Some(params) = &obs.imperial {
                (
                    params.dewpt,
                    params.heat_index,
                    params.precip_rate,
                    params.precip_total,
                    params.pressure,
                    params.temp,
                    params.wind_chill,
                    params.wind_gust,
                    params.wind_speed,
                )
            } else {
                panic!("weather data didn't return any imperial data")
            };

        if let Some(dewpt) = dewpt {
            if (0.0..=200.0).contains(&dewpt) {
                devices.dewpt.report_update(dewpt).await
            } else {
                warn!("ignoring bad dew point value: {:.1}", dewpt)
            }
        }

        if let Some(htidx) = htidx {
            if (0.0..=200.0).contains(&htidx) {
                devices.htidx.report_update(htidx).await
            } else {
                warn!("ignoring bad heat index value: {:.1}", htidx)
            }
        }

        if let (Some(prate), Some(ptotal)) = (prate, ptotal) {
            let (nrate, ntotal, nlast) =
                self.precip.update(prate, ptotal, SystemTime::now());

            devices.prec_rate.report_update(nrate).await;
            devices.prec_total.report_update(ntotal).await;

            if let Some(last) = nlast {
                devices.prec_last_total.report_update(last).await;
            }
        } else {
            warn!("need both precip fields to update precip calculations")
        }

        if let Some(press) = press {
            devices.pressure.report_update(press).await
        }

        if let Some(temp) = temp {
            devices.temp.report_update(temp).await
        }

        if let Some(wndchl) = wndchl {
            devices.wndchl.report_update(wndchl).await
        }

        if let Some(wndgst) = wndgst {
            devices.wndgst.report_update(wndgst).await
        }

        if let Some(wndspd) = wndspd {
            devices.wndspd.report_update(wndspd).await
        }

        // If solar radiation readings are provided, report them.

        if let Some(sol_rad) = obs.solar_radiation {
            // On Earth, solar radiation varies between 0 and 1361
            // W/m^2. (https://en.wikipedia.org/wiki/Solar_irradiance)
            // We'll round up to 1400 so weather stations with
            // slightly inaccurate sensors won't be ignored.

            if (0.0..=1400.0).contains(&sol_rad) {
                devices.solrad.report_update(sol_rad).await
            } else {
                warn!("ignoring bad solar radiation value: {:.1}", sol_rad)
            }
        }

        // If humidity readings are provided, report them.

        if let Some(humidity) = obs.humidity {
            // Technically the humidity could get to 0%, but it's
            // doubtful there's a place on earth that gets that low.

            if (0.0..=100.0).contains(&humidity) {
                devices.humidity.report_update(humidity).await
            } else {
                warn!("ignoring bad humidity value: {:.1}", humidity)
            }
        }

        // If UV readings are provided, report them.

        if let Some(uv) = obs.uv {
            devices.uv.report_update(uv).await
        }

        // If wind direction readings are provided, report them.

        if let Some(winddir) = obs.winddir {
            // Make sure the reading is in range.

            if (0.0..=360.0).contains(&winddir) {
                devices.wnddir.report_update(winddir).await
            } else {
                warn!("ignoring bad wind direction value: {:.1}", winddir)
            }
        }
    }
}

fn xlat_units(u: &classes::Units) -> wu::Unit {
    match u {
        classes::Units::English => wu::Unit::English,
        classes::Units::Metric => wu::Unit::Metric,
    }
}

impl driver::API for Instance {
    type HardwareType = classes::Weather;

    fn create_instance(
        cfg: &DriverConfig,
    ) -> impl Future<Output = Result<Box<Self>>> + Send {
        debug!("reading config parameters");

        let interval = Instance::get_cfg_interval(cfg);
        let key = Instance::get_cfg_key(cfg);

        async move {
            match wu::create_client(Duration::from_secs(5)) {
                Ok(mut con) => {
                    // Validate the driver parameters.

                    let (api_key, interval) =
                        Instance::get_cfg_key_and_interval(
                            &mut con, key?, interval?,
                        )
                        .await?;

                    // Assemble and return the state of the driver.

                    debug!("instance successfully created");

                    Ok(Box::new(Instance {
                        con,
                        api_key,
                        interval,
                        precip: PrecipState::new(),
                    }))
                }
                Err(e) => Err(Error::ConfigError(format!(
                    "couldn't build client connection -- {}",
                    &e
                ))),
            }
        }
    }

    async fn run(&mut self, devices: &mut Self::HardwareType) -> Infallible {
        Span::current().record("cfg", devices.station.as_str());

        devices.error.report_update(false).await;

        let mut timer = interval_at(Instant::now(), self.interval);

        // Loop forever.

        loop {
            debug!("waiting for next poll time");

            // Wait for the next sample time.

            timer.tick().await;

            debug!("fetching next observation");

            let result = wu::fetch_observation(
                &self.con,
                &self.api_key,
                &devices.station,
                &xlat_units(&devices.units),
            )
            .await;

            match result {
                Ok(Some(response)) => {
                    match wu::ObservationResponse::try_from(response) {
                        Ok(resp) => {
                            if let Some(obs) = resp.observations {
                                if !obs.is_empty() {
                                    // The API we're using should only
                                    // return 1 set of observations.
                                    // If it, for some reason, changes
                                    // and returns more, log it.

                                    if obs.len() > 1 {
                                        warn!("ignoring {} extra weather observations", obs.len() - 1);
                                    }
                                    devices.error.report_update(false).await;
                                    self.handle(&obs[0], devices).await;
                                    continue;
                                }
                            }
                            warn!("no weather data received")
                        }

                        Err(e) => {
                            devices.error.report_update(true).await;
                            panic!("error response from Weather Underground -- {:?}", &e)
                        }
                    }
                }

                Ok(None) => {
                    devices.error.report_update(true).await;
                    panic!("no response from Weather Underground")
                }

                Err(e) => {
                    devices.error.report_update(true).await;
                    panic!("error accessing Weather Underground -- {:?}", &e)
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::PrecipState;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    fn mk_time(secs: u64) -> SystemTime {
        UNIX_EPOCH.checked_add(Duration::from_secs(secs)).unwrap()
    }

    #[test]
    fn test_precip() {
        // This tests for normal rainfall. It also makes sure the
        // totals get adjusted after the long enough delay of no rain.

        {
            let mut s = PrecipState::new();

            // Should start as `NoRain` and, as long as we have no
            // precip, it should stay that way.

            assert_eq!(s.update(0.0, 0.0, mk_time(0)), (0.0, 0.0, None));
            assert_eq!(s.update(0.0, 0.0, mk_time(600)), (0.0, 0.0, None));

            // Even if the rainfall rate is non zero, we don't go into
            // the rain state until the total is nonzero.

            assert_eq!(s.update(0.1, 0.0, mk_time(1200)), (0.1, 0.0, None));

            // With both inputs 0.0, we shouldn't trigger a "last
            // rainfall" total.

            assert_eq!(s.update(0.0, 0.0, mk_time(1800)), (0.0, 0.0, None));

            // As rain occurs, we should track the total.

            assert_eq!(s.update(0.1, 0.125, mk_time(2400)), (0.1, 0.125, None));
            assert_eq!(s.update(0.05, 0.25, mk_time(3000)), (0.05, 0.25, None));
            assert_eq!(s.update(0.7, 0.375, mk_time(3600)), (0.7, 0.375, None));

            // Zero rate shouldn't reset by itself.

            assert_eq!(s.update(0.0, 0.375, mk_time(4200)), (0.0, 0.375, None));

            // Even if both are zeroed, we don't reset the count if
            // the time from the last rain was less than our timeout.

            assert_eq!(s.update(0.0, 0.0, mk_time(4800)), (0.0, 0.375, None));

            // Now add more and then simulate 10 hours of nothing.

            assert_eq!(s.update(0.1, 0.125, mk_time(5400)), (0.1, 0.5, None));
            assert_eq!(s.update(0.0, 0.125, mk_time(6000)), (0.0, 0.5, None));
            assert_eq!(s.update(0.0, 0.125, mk_time(41_399)), (0.0, 0.5, None));
            assert_eq!(
                s.update(0.0, 0.125, mk_time(41_400)),
                (0.0, 0.0, Some(0.5))
            );
            assert_eq!(s.update(0.0, 0.125, mk_time(42_001)), (0.0, 0.0, None));

            // Now any new rainfall start new accumulation.

            assert_eq!(s.update(0.1, 0.125, mk_time(40_800)), (0.1, 0.0, None));
            assert_eq!(
                s.update(0.1, 0.25, mk_time(41_400)),
                (0.1, 0.125, None)
            );
        }

        // This tests for a possible weird occurrance at midnight.

        {
            let mut s = PrecipState::new();

            // Reproduce the previous rain, but we'll add a midnight
            // crossing (which resets the total).

            assert_eq!(s.update(0.1, 0.125, mk_time(0)), (0.1, 0.125, None));
            assert_eq!(s.update(0.05, 0.25, mk_time(600)), (0.05, 0.25, None));
            assert_eq!(s.update(0.7, 0.375, mk_time(1200)), (0.7, 0.375, None));
            assert_eq!(s.update(0.3, 0.125, mk_time(1800)), (0.3, 0.5, None));
            assert_eq!(s.update(0.3, 0.25, mk_time(2400)), (0.3, 0.625, None));

            // Let's assume that the total got reset just before
            // reporting it to Weather Underground. In that case, the
            // total would be zero but the rate would be non-zero.

            assert_eq!(s.update(0.1, 0.0, mk_time(3000)), (0.1, 0.625, None));
            assert_eq!(s.update(0.0, 0.0, mk_time(3600)), (0.0, 0.625, None));
            assert_eq!(s.update(0.3, 0.125, mk_time(4200)), (0.3, 0.75, None));
        }
    }
}

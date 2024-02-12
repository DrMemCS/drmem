use drmem_api::{
    device,
    driver::{self, DriverConfig},
    Error, Result,
};
use std::convert::{Infallible, TryFrom};
use std::{future::Future, pin::Pin, sync::Arc};
use tokio::sync::Mutex;
use tokio::time::{interval_at, Duration, Instant};
use tracing::{debug, error, warn, Span};
use weather_underground as wu;

const DEFAULT_INTERVAL: u64 = 10;
const MIN_PUBLIC_INTERVAL: u64 = 10;

pub struct Instance {
    con: reqwest::Client,
    api_key: String,
    interval: Duration,
}

pub struct Devices {
    station: String,
    units: wu::Unit,

    d_dewpt: driver::ReportReading<f64>,
    d_htidx: driver::ReportReading<f64>,
    d_humidity: driver::ReportReading<f64>,
    d_prate: driver::ReportReading<f64>,
    d_ptotal: driver::ReportReading<f64>,
    d_pressure: driver::ReportReading<f64>,
    d_solrad: driver::ReportReading<f64>,
    d_state: driver::ReportReading<bool>,
    d_temp: driver::ReportReading<f64>,
    d_uv: driver::ReportReading<f64>,
    d_wndchl: driver::ReportReading<f64>,
    d_wnddir: driver::ReportReading<f64>,
    d_wndgst: driver::ReportReading<f64>,
    d_wndspd: driver::ReportReading<f64>,
}

impl Instance {
    pub const NAME: &'static str = "weather-wu";

    pub const SUMMARY: &'static str =
        "obtains weather data from Weather Underground";

    pub const DESCRIPTION: &'static str = include_str!("../README.md");

    fn get_cfg_station(cfg: &DriverConfig) -> Result<String> {
        match cfg.get("station") {
            Some(toml::value::Value::String(station)) => {
                Ok(station.to_string())
            }
            Some(_) => Err(Error::ConfigError(String::from(
                "'station' config parameter should be a string",
            ))),
            None => Err(Error::ConfigError(String::from(
                "missing 'station' parameter in config",
            ))),
        }
    }

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

    fn get_cfg_units(cfg: &DriverConfig) -> Result<wu::Unit> {
        match cfg.get("units") {
            Some(toml::value::Value::String(val)) => match val.as_str() {
                "metric" => Ok(wu::Unit::Metric),
                "imperial" => Ok(wu::Unit::English),
                _ => Err(Error::ConfigError(String::from(
                    "'units' parameter should be \"imperial\" or \"metric\"",
                ))),
            },
            Some(_) => Err(Error::ConfigError(String::from(
                "'units' parameter should be a string",
            ))),
            None => Ok(wu::Unit::Metric),
        }
    }

    // Processes an observation by sending each parameter to the
    // correct device channel. It also does some sanity checks on the
    // values.

    async fn handle(
        &mut self,
        obs: &wu::Observation,
        devices: &<Instance as driver::API>::DeviceSet,
    ) {
        // Retreive all the parameters whose units can change between
        // English and Metric.

        let (dewpt, htidx, prate, ptotal, press, temp, wndchl, wndgst, wndspd) =
            if let wu::Unit::Metric = devices.units {
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
                (devices.d_dewpt)(dewpt).await
            } else {
                warn!("ignoring bad dew point value: {:.1}", dewpt)
            }
        }

        if let Some(htidx) = htidx {
            if (0.0..=200.0).contains(&htidx) {
                (devices.d_htidx)(htidx).await
            } else {
                warn!("ignoring bad heat index value: {:.1}", htidx)
            }
        }

        if let (Some(prate), Some(ptotal)) = (prate, ptotal) {
            if (0.0..=24.0).contains(&prate) {
                (devices.d_prate)(prate).await
            } else {
                warn!("ignoring bad precip rate: {:.2}", prate)
            }

            (devices.d_ptotal)(ptotal).await
        } else {
            warn!("need both precip fields to update precip calculations")
        }

        if let Some(press) = press {
            (devices.d_pressure)(press).await
        }

        if let Some(temp) = temp {
            (devices.d_temp)(temp).await
        }

        if let Some(wndchl) = wndchl {
            (devices.d_wndchl)(wndchl).await
        }

        if let Some(wndgst) = wndgst {
            (devices.d_wndgst)(wndgst).await
        }

        if let Some(wndspd) = wndspd {
            (devices.d_wndspd)(wndspd).await
        }

        // If solar radiation readings are provided, report them.

        if let Some(sol_rad) = obs.solar_radiation {
            // On Earth, solar radiation varies between 0 and 1361
            // W/m^2. (https://en.wikipedia.org/wiki/Solar_irradiance)
            // We'll round up to 1400 so weather stations with
            // slightly inaccurate sensors won't be ignored.

            if (0.0..=1400.0).contains(&sol_rad) {
                (devices.d_solrad)(sol_rad).await
            } else {
                warn!("ignoring bad solar radiation value: {:.1}", sol_rad)
            }
        }

        // If humidity readings are provided, report them.

        if let Some(humidity) = obs.humidity {
            // Technically the humidity could get to 0%, but it's
            // doubtful there's a place on earth that gets that low.

            if (0.0..=100.0).contains(&humidity) {
                (devices.d_humidity)(humidity).await
            } else {
                warn!("ignoring bad humidity value: {:.1}", humidity)
            }
        }

        // If UV readings are provided, report them.

        if let Some(uv) = obs.uv {
            (devices.d_uv)(uv).await
        }

        // If wind direction readings are provided, report them.

        if let Some(winddir) = obs.winddir {
            // Make sure the reading is in range.

            if (0.0..=360.0).contains(&winddir) {
                (devices.d_wnddir)(winddir).await
            } else {
                warn!("ignoring bad wind direction value: {:.1}", winddir)
            }
        }
    }
}

impl driver::API for Instance {
    type DeviceSet = Devices;

    fn register_devices(
        core: driver::RequestChan,
        cfg: &DriverConfig,
        max_history: Option<usize>,
    ) -> Pin<Box<dyn Future<Output = Result<Self::DeviceSet>> + Send>> {
        let dewpoint_name = "dewpoint".parse::<device::Base>().unwrap();
        let heat_index_name = "heat-index".parse::<device::Base>().unwrap();
        let humidity_name = "humidity".parse::<device::Base>().unwrap();
        let precip_rate_name = "precip-rate".parse::<device::Base>().unwrap();
        let precip_total_name = "precip-total".parse::<device::Base>().unwrap();
        let pressure_name = "pressure".parse::<device::Base>().unwrap();
        let solar_rad_name = "solar-rad".parse::<device::Base>().unwrap();
        let state_name = "state".parse::<device::Base>().unwrap();
        let temperature_name = "temperature".parse::<device::Base>().unwrap();
        let uv_name = "uv".parse::<device::Base>().unwrap();
        let wind_chill_name = "wind-chill".parse::<device::Base>().unwrap();
        let wind_dir_name = "wind-dir".parse::<device::Base>().unwrap();
        let wind_gust_name = "wind-gust".parse::<device::Base>().unwrap();
        let wind_speed_name = "wind-speed".parse::<device::Base>().unwrap();

        let station = Instance::get_cfg_station(cfg);
        let units = Instance::get_cfg_units(cfg);

        Box::pin(async move {
            let station = station?;
            let units = units?;

            let temp_unit = Some(if let wu::Unit::English = units {
                "°F"
            } else {
                "°C"
            });
            let speed_unit = Some(if let wu::Unit::English = units {
                "mph"
            } else {
                "km/h"
            });

            let (d_dewpt, _) = core
                .add_ro_device(dewpoint_name, temp_unit, max_history)
                .await?;
            let (d_htidx, _) = core
                .add_ro_device(heat_index_name, temp_unit, max_history)
                .await?;
            let (d_humidity, _) = core
                .add_ro_device(humidity_name, Some("%"), max_history)
                .await?;
            let (d_prate, _) = core
                .add_ro_device(
                    precip_rate_name,
                    Some(if let wu::Unit::English = units {
                        "in/hr"
                    } else {
                        "mm/hr"
                    }),
                    max_history,
                )
                .await?;

            let (d_ptotal, _) = core
                .add_ro_device(
                    precip_total_name,
                    Some(if let wu::Unit::English = units {
                        "in"
                    } else {
                        "mm"
                    }),
                    max_history,
                )
                .await?;

            let (d_pressure, _) = core
                .add_ro_device(
                    pressure_name,
                    Some(if let wu::Unit::English = units {
                        "inHg"
                    } else {
                        "hPa"
                    }),
                    max_history,
                )
                .await?;

            let (d_solrad, _) = core
                .add_ro_device(solar_rad_name, Some("W/m²"), max_history)
                .await?;
            let (d_state, _) =
                core.add_ro_device(state_name, None, max_history).await?;
            let (d_temp, _) = core
                .add_ro_device(temperature_name, temp_unit, max_history)
                .await?;
            let (d_uv, _) =
                core.add_ro_device(uv_name, None, max_history).await?;
            let (d_wndchl, _) = core
                .add_ro_device(wind_chill_name, temp_unit, max_history)
                .await?;
            let (d_wnddir, _) = core
                .add_ro_device(wind_dir_name, Some("°"), max_history)
                .await?;
            let (d_wndgst, _) = core
                .add_ro_device(wind_gust_name, speed_unit, max_history)
                .await?;
            let (d_wndspd, _) = core
                .add_ro_device(wind_speed_name, speed_unit, max_history)
                .await?;

            Ok(Devices {
                station,
                units,
                d_dewpt,
                d_htidx,
                d_humidity,
                d_prate,
                d_ptotal,
                d_pressure,
                d_solrad,
                d_state,
                d_temp,
                d_uv,
                d_wndchl,
                d_wnddir,
                d_wndgst,
                d_wndspd,
            })
        })
    }

    fn create_instance(
        cfg: &DriverConfig,
    ) -> Pin<Box<dyn Future<Output = Result<Box<Self>>> + Send>> {
        debug!("reading config parameters");

        let interval = Instance::get_cfg_interval(cfg);
        let key = Instance::get_cfg_key(cfg);

        Span::current().record("cfg", Instance::get_cfg_station(cfg).unwrap());

        let fut = async move {
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
                    }))
                }
                Err(e) => Err(Error::ConfigError(format!(
                    "couldn't build client connection -- {}",
                    &e
                ))),
            }
        };

        Box::pin(fut)
    }

    fn run<'a>(
        &'a mut self,
        devices: Arc<Mutex<Self::DeviceSet>>,
    ) -> Pin<Box<dyn Future<Output = Infallible> + Send + 'a>> {
        let fut = async move {
            let devices = devices.lock().await;

            Span::current().record("cfg", devices.station.as_str());

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
                    &devices.units,
                )
                .await;

                match result {
                    Ok(Some(response)) => {
                        match wu::ObservationResponse::try_from(response) {
                            Ok(resp) => {
                                if let Some(obs) = resp.observations {
                                    if !obs.is_empty() {
                                        // The API we're using should
                                        // only return 1 set of
                                        // observations. If it, for
                                        // some reason, changes and
                                        // returns more, log it.

                                        if obs.len() > 1 {
                                            warn!("ignoring {} extra weather observations", obs.len() - 1);
                                        }
                                        (devices.d_state)(true).await;
                                        self.handle(&obs[0], &devices).await;
                                        continue;
                                    }
                                }
                                warn!("no weather data received")
                            }

                            Err(e) => {
                                (devices.d_state)(false).await;
                                panic!("error response from Weather Underground -- {:?}", &e)
                            }
                        }
                    }

                    Ok(None) => {
                        (devices.d_state)(false).await;
                        panic!("no response from Weather Underground")
                    }

                    Err(e) => {
                        (devices.d_state)(false).await;
                        panic!(
                            "error accessing Weather Underground -- {:?}",
                            &e
                        )
                    }
                }
            }
        };

        Box::pin(fut)
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn it_works() {
        let result = 2 + 2;
        assert_eq!(result, 4);
    }
}

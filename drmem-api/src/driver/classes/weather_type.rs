use crate::{
    device,
    driver::{self, DriverConfig, Registrator, RequestChan, Result},
    Error,
};
use std::future::Future;
use tokio::time::Duration;

pub enum Units {
    English,
    Metric,
}

pub struct Weather {
    pub station: String,
    pub units: Units,

    pub dewpt: driver::ReadOnlyDevice<f64>,
    pub htidx: driver::ReadOnlyDevice<f64>,
    pub humidity: driver::ReadOnlyDevice<f64>,
    pub prec_rate: driver::ReadOnlyDevice<f64>,
    pub prec_total: driver::ReadOnlyDevice<f64>,
    pub prec_last_total: driver::ReadOnlyDevice<f64>,
    pub pressure: driver::ReadOnlyDevice<f64>,
    pub solrad: driver::ReadOnlyDevice<f64>,
    pub error: driver::ReadOnlyDevice<bool>,
    pub temp: driver::ReadOnlyDevice<f64>,
    pub uv: driver::ReadOnlyDevice<f64>,
    pub wndchl: driver::ReadOnlyDevice<f64>,
    pub wnddir: driver::ReadOnlyDevice<f64>,
    pub wndgst: driver::ReadOnlyDevice<f64>,
    pub wndspd: driver::ReadOnlyDevice<f64>,
}

impl Weather {
    fn get_cfg_units(cfg: &DriverConfig) -> Result<Units> {
        match cfg.get("units") {
            Some(toml::value::Value::String(val)) => match val.as_str() {
                "metric" => Ok(Units::Metric),
                "imperial" => Ok(Units::English),
                _ => Err(Error::ConfigError(String::from(
                    "'units' parameter should be \"imperial\" or \"metric\"",
                ))),
            },
            Some(_) => Err(Error::ConfigError(String::from(
                "'units' parameter should be a string",
            ))),
            None => Ok(Units::Metric),
        }
    }

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
}

impl Registrator for Weather {
    fn register_devices<'a>(
        drc: &'a mut RequestChan,
        cfg: &DriverConfig,
        _override_timeout: Option<Duration>,
        max_history: Option<usize>,
    ) -> impl Future<Output = Result<Self>> + Send + 'a {
        let dewpoint_name = "dewpoint".parse::<device::Base>().unwrap();
        let heat_index_name = "heat-index".parse::<device::Base>().unwrap();
        let humidity_name = "humidity".parse::<device::Base>().unwrap();
        let precip_rate_name = "precip-rate".parse::<device::Base>().unwrap();
        let precip_total_name = "precip-total".parse::<device::Base>().unwrap();
        let precip_last_total_name =
            "precip-last-total".parse::<device::Base>().unwrap();
        let pressure_name = "pressure".parse::<device::Base>().unwrap();
        let solar_rad_name = "solar-rad".parse::<device::Base>().unwrap();
        let temperature_name = "temperature".parse::<device::Base>().unwrap();
        let uv_name = "uv".parse::<device::Base>().unwrap();
        let wind_chill_name = "wind-chill".parse::<device::Base>().unwrap();
        let wind_dir_name = "wind-dir".parse::<device::Base>().unwrap();
        let wind_gust_name = "wind-gust".parse::<device::Base>().unwrap();
        let wind_speed_name = "wind-speed".parse::<device::Base>().unwrap();
        let error_name = "error".parse::<device::Base>().unwrap();

        let station = Self::get_cfg_station(cfg);
        let units = Self::get_cfg_units(cfg);

        async move {
            let station = station?;
            let units = units?;

            let temp_unit = Some(if let Units::English = units {
                "°F"
            } else {
                "°C"
            });
            let speed_unit = Some(if let Units::English = units {
                "mph"
            } else {
                "km/h"
            });

            let dewpt = drc
                .add_ro_device(dewpoint_name, temp_unit, max_history)
                .await?;
            let htidx = drc
                .add_ro_device(heat_index_name, temp_unit, max_history)
                .await?;
            let humidity = drc
                .add_ro_device(humidity_name, Some("%"), max_history)
                .await?;
            let prec_rate = drc
                .add_ro_device(
                    precip_rate_name,
                    Some(if let Units::English = units {
                        "in/hr"
                    } else {
                        "mm/hr"
                    }),
                    max_history,
                )
                .await?;

            let prec_total = drc
                .add_ro_device(
                    precip_total_name,
                    Some(if let Units::English = units {
                        "in"
                    } else {
                        "mm"
                    }),
                    max_history,
                )
                .await?;

            let prec_last_total = drc
                .add_ro_device(
                    precip_last_total_name,
                    Some(if let Units::English = units {
                        "in"
                    } else {
                        "mm"
                    }),
                    max_history,
                )
                .await?;

            let pressure = drc
                .add_ro_device(
                    pressure_name,
                    Some(if let Units::English = units {
                        "inHg"
                    } else {
                        "hPa"
                    }),
                    max_history,
                )
                .await?;

            let solrad = drc
                .add_ro_device(solar_rad_name, Some("W/m²"), max_history)
                .await?;
            let error =
                drc.add_ro_device(error_name, None, max_history).await?;
            let temp = drc
                .add_ro_device(temperature_name, temp_unit, max_history)
                .await?;
            let uv = drc.add_ro_device(uv_name, None, max_history).await?;
            let wndchl = drc
                .add_ro_device(wind_chill_name, temp_unit, max_history)
                .await?;
            let wnddir = drc
                .add_ro_device(wind_dir_name, Some("°"), max_history)
                .await?;
            let wndgst = drc
                .add_ro_device(wind_gust_name, speed_unit, max_history)
                .await?;
            let wndspd = drc
                .add_ro_device(wind_speed_name, speed_unit, max_history)
                .await?;

            Ok(Weather {
                station,
                units,
                dewpt,
                htidx,
                humidity,
                prec_rate,
                prec_total,
                prec_last_total,
                pressure,
                solrad,
                error,
                temp,
                uv,
                wndchl,
                wnddir,
                wndgst,
                wndspd,
            })
        }
    }
}

impl crate::driver::ResettableState for Weather {}

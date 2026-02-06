use crate::driver::{self, Registrator, Reporter, RequestChan, Result};

#[derive(serde::Deserialize, Clone)]
pub enum WeatherUnits {
    English,
    Metric,
}

#[derive(serde::Deserialize)]
pub struct WeatherConfig {
    pub units: WeatherUnits,
}

pub struct Weather<R: Reporter> {
    pub units: WeatherUnits,

    pub dewpt: driver::ReadOnlyDevice<f64, R>,
    pub htidx: driver::ReadOnlyDevice<f64, R>,
    pub humidity: driver::ReadOnlyDevice<f64, R>,
    pub prec_rate: driver::ReadOnlyDevice<f64, R>,
    pub prec_total: driver::ReadOnlyDevice<f64, R>,
    pub prec_last_total: driver::ReadOnlyDevice<f64, R>,
    pub pressure: driver::ReadOnlyDevice<f64, R>,
    pub solrad: driver::ReadOnlyDevice<f64, R>,
    pub error: driver::ReadOnlyDevice<bool, R>,
    pub temp: driver::ReadOnlyDevice<f64, R>,
    pub uv: driver::ReadOnlyDevice<f64, R>,
    pub wndchl: driver::ReadOnlyDevice<f64, R>,
    pub wnddir: driver::ReadOnlyDevice<f64, R>,
    pub wndgst: driver::ReadOnlyDevice<f64, R>,
    pub wndspd: driver::ReadOnlyDevice<f64, R>,
}

impl<R: Reporter> Registrator<R> for Weather<R> {
    type Config = WeatherConfig;

    async fn register_devices(
        drc: &mut RequestChan<R>,
        cfg: &Self::Config,
        max_history: Option<usize>,
    ) -> Result<Self> {
        let temp_unit = Some(if let WeatherUnits::English = cfg.units {
            "°F"
        } else {
            "°C"
        });
        let speed_unit = Some(if let WeatherUnits::English = cfg.units {
            "mph"
        } else {
            "km/h"
        });

        let dewpt = drc
            .add_ro_device("dewpoint", temp_unit, max_history)
            .await?;
        let htidx = drc
            .add_ro_device("heat-index", temp_unit, max_history)
            .await?;
        let humidity = drc
            .add_ro_device("humidity", Some("%"), max_history)
            .await?;
        let prec_rate = drc
            .add_ro_device(
                "precip-rate",
                Some(if let WeatherUnits::English = cfg.units {
                    "in/hr"
                } else {
                    "mm/hr"
                }),
                max_history,
            )
            .await?;

        let prec_total = drc
            .add_ro_device(
                "precip-total",
                Some(if let WeatherUnits::English = cfg.units {
                    "in"
                } else {
                    "mm"
                }),
                max_history,
            )
            .await?;

        let prec_last_total = drc
            .add_ro_device(
                "precip-last-total",
                Some(if let WeatherUnits::English = cfg.units {
                    "in"
                } else {
                    "mm"
                }),
                max_history,
            )
            .await?;

        let pressure = drc
            .add_ro_device(
                "pressure",
                Some(if let WeatherUnits::English = cfg.units {
                    "inHg"
                } else {
                    "hPa"
                }),
                max_history,
            )
            .await?;

        let solrad = drc
            .add_ro_device("solar-rad", Some("W/m²"), max_history)
            .await?;
        let error = drc.add_ro_device("error", None, max_history).await?;
        let temp = drc
            .add_ro_device("temperature", temp_unit, max_history)
            .await?;
        let uv = drc.add_ro_device("uv", None, max_history).await?;
        let wndchl = drc
            .add_ro_device("wind-chill", temp_unit, max_history)
            .await?;
        let wnddir = drc
            .add_ro_device("wind-dir", Some("°"), max_history)
            .await?;
        let wndgst = drc
            .add_ro_device("wind-gust", speed_unit, max_history)
            .await?;
        let wndspd = drc
            .add_ro_device("wind-speed", speed_unit, max_history)
            .await?;

        Ok(Weather {
            units: cfg.units.clone(),
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

impl<R: Reporter> crate::driver::ResettableState for Weather<R> {}

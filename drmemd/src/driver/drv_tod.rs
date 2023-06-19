use chrono::{self, Datelike, Timelike};
use drmem_api::{
    device,
    driver::{self, DriverConfig},
    Error, Result,
};
use std::{convert::Infallible, future::Future, pin::Pin, sync::Arc};
use tokio::{sync::Mutex, time};
use tracing::info;

pub struct Instance;

pub struct Devices {
    utc: bool,

    d_year: driver::ReportReading<u16>,
    d_month: driver::ReportReading<u16>,
    d_day: driver::ReportReading<u16>,
    d_hour: driver::ReportReading<u16>,
    d_min: driver::ReportReading<u16>,
    d_second: driver::ReportReading<u16>,
    d_dow: driver::ReportReading<u16>,
}

impl Instance {
    pub const NAME: &'static str = "tod";

    pub const SUMMARY: &'static str =
        "Provides devices that represent time-of-day.";

    pub const DESCRIPTION: &'static str = include_str!("drv_tod.md");

    pub fn initial_delay() -> u32 {
        let now = chrono::Utc::now();
        let extra = now.timestamp_subsec_millis();

        (10020 - extra) % 1000
    }

    fn get_utc_flag(cfg: &DriverConfig) -> Result<bool> {
        match cfg.get("utc") {
            Some(toml::value::Value::Boolean(level)) => Ok(*level),
            Some(_) => Err(Error::BadConfig(String::from(
                "'utc' config parameter should be a boolean",
            ))),
            None => Ok(false),
        }
    }

    fn get_time(utc: bool) -> (u16, u16, u16, u16, u16, u16, u16) {
        if utc {
            let now = chrono::Utc::now();

            (
                now.year() as u16,
                now.month0() as u16,
                now.day0() as u16,
                now.weekday().num_days_from_monday() as u16,
                now.hour() as u16,
                now.minute() as u16,
                now.second() as u16,
            )
        } else {
            let now = chrono::Local::now();

            (
                now.year() as u16,
                now.month0() as u16,
                now.day0() as u16,
                now.weekday().num_days_from_monday() as u16,
                now.hour() as u16,
                now.minute() as u16,
                now.second() as u16,
            )
        }
    }
}

impl driver::API for Instance {
    type DeviceSet = Devices;

    fn register_devices(
        core: driver::RequestChan,
        cfg: &DriverConfig,
        _max_history: Option<usize>,
    ) -> Pin<Box<dyn Future<Output = Result<Self::DeviceSet>> + Send>> {
        let year_name = "year".parse::<device::Base>().unwrap();
        let month_name = "month".parse::<device::Base>().unwrap();
        let day_name = "day".parse::<device::Base>().unwrap();
        let hour_name = "hour".parse::<device::Base>().unwrap();
        let min_name = "minute".parse::<device::Base>().unwrap();
        let second_name = "second".parse::<device::Base>().unwrap();
        let dow_name = "day-of-week".parse::<device::Base>().unwrap();

        let utc = Instance::get_utc_flag(cfg);

        Box::pin(async move {
            let utc = utc?;

            let (d_year, _) =
                core.add_ro_device(year_name, None, Some(1)).await?;
            let (d_month, _) =
                core.add_ro_device(month_name, None, Some(1)).await?;
            let (d_day, _) =
                core.add_ro_device(day_name, None, Some(1)).await?;
            let (d_hour, _) =
                core.add_ro_device(hour_name, None, Some(1)).await?;
            let (d_min, _) =
                core.add_ro_device(min_name, None, Some(1)).await?;
            let (d_second, _) =
                core.add_ro_device(second_name, None, Some(1)).await?;
            let (d_dow, _) =
                core.add_ro_device(dow_name, None, Some(1)).await?;

            Ok(Devices {
                utc,
                d_year,
                d_month,
                d_day,
                d_hour,
                d_min,
                d_second,
                d_dow,
            })
        })
    }

    fn create_instance(
        _cfg: &DriverConfig,
    ) -> Pin<Box<dyn Future<Output = Result<Box<Self>>> + Send>> {
        Box::pin(async move { Ok(Box::new(Instance)) })
    }

    fn run<'a>(
        &'a mut self,
        devices: Arc<Mutex<Self::DeviceSet>>,
    ) -> Pin<Box<dyn Future<Output = Infallible> + Send + 'a>> {
        let fut = async move {
            let mut year: Option<u16> = None;
            let mut month: Option<u16> = None;
            let mut day: Option<u16> = None;
            let mut hour: Option<u16> = None;
            let mut minute: Option<u16> = None;
            let mut second: Option<u16> = None;
            let mut dow: Option<u16> = None;

            let mut interval = time::interval_at(
                time::Instant::now()
                    + time::Duration::from_millis(
                        Instance::initial_delay() as u64
                    ),
                time::Duration::from_secs(1),
            );

            let devices = devices.lock().await;

            loop {
                let _inst = interval.tick().await;

                let (
                    now_year,
                    now_month,
                    now_day,
                    now_dow,
                    now_hour,
                    now_min,
                    now_sec,
                ) = Instance::get_time(devices.utc);

                if year != Some(now_year) {
                    year = Some(now_year);
                    (devices.d_year)(now_year).await
                }

                if month != Some(now_month) {
                    month = Some(now_month);
                    (devices.d_month)(now_month).await
                }

                if day != Some(now_day) {
                    day = Some(now_day);
                    (devices.d_day)(now_day).await
                }

                if hour != Some(now_hour) {
                    hour = Some(now_hour);
                    (devices.d_hour)(now_hour).await
                }

                if minute != Some(now_min) {
                    minute = Some(now_min);
                    (devices.d_min)(now_min).await
                }

                if second != Some(now_sec) {
                    second = Some(now_sec);
                    (devices.d_second)(now_sec).await
                }

                if dow != Some(now_dow) {
                    dow = Some(now_dow);
                    (devices.d_dow)(now_dow).await
                }
            }
        };

        Box::pin(fut)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_tod() {}
}

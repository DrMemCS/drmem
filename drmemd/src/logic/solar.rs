// The fomulas found in this module were obtained from
//
//	https://www.sciencedirect.com/science/article/pii/S0960148121004031
//
// in late Feb of 2024.

use chrono::{Datelike, Timelike};
use std::sync::Arc;
use tokio::{sync::broadcast, time};
use tracing::{info, info_span, warn};
use tracing_futures::Instrument;

pub struct SolarInfo {
    pub elevation: f64,
    pub azimuth: f64,
    pub right_ascension: f64,
    pub declination: f64,
}

pub type Info = Arc<SolarInfo>;

// Compute the sun's position information based on the latitude
// (degrees), longitude (degrees), and time-of-day. The formulas used
// in this function were obtained from a paper link to from the
// Wikipedia page. The paper included FORTRAN code to perform the
// calculations. That code was used as a reference to built this
// function.

fn get_solar_position(
    lat: f64,
    long: f64,
    time: &chrono::DateTime<chrono::Utc>,
) -> Arc<SolarInfo> {
    // Convert time-of-day to a floating point value in the range 0.0
    // through 23.999.

    let gmtime: f64 = time.hour() as f64
        + ((time.minute() * 60 + time.second()) as f64 / 3600.0);

    // Calculate the number of days since the "base date" used by
    // these formulas (Jan 1st, 2000 UTC). The number of leap years
    // will be correct until 2100.

    let n_ly: f64 = ((time.year() - 2000) / 4 + 1) as f64;
    let n: f64 = n_ly
        + (time.year() - 2000) as f64 * 365.0
        + time.ordinal0() as f64
        + gmtime / 24.0
        - 1.5;

    // Calculate "mean longitude" for the sun.

    let l: f64 = (280.466 + 0.9856474 * n).rem_euclid(360.0);

    // Calculate "mean anomaly".

    let g: f64 = (357.528 + 0.9856003 * n).rem_euclid(360.0).to_radians();

    // Calculate "eliptic longitude".

    let lambda: (f64, f64) =
        (l + 1.915 * f64::sin(g) + 0.020 * f64::sin(2.0 * g))
            .rem_euclid(360.0)
            .to_radians()
            .sin_cos();

    // Calculate the "obliquity of ecliptic".

    let epsilon: (f64, f64) = (23.440 - 0.0000004 * n).to_radians().sin_cos();

    // Compute the right ascension.

    let alpha: f64 = f64::atan2(epsilon.1 * lambda.0, lambda.1)
        .to_degrees()
        .rem_euclid(360.0);

    // Compute the declination.

    let sunlat: f64 = f64::asin(epsilon.0 * lambda.0);
    let delta: f64 = sunlat.to_degrees();
    let sunlat_sc: (f64, f64) = sunlat.sin_cos();

    // Compute the "equation of time".

    let eot: f64 = (l - alpha + 180.0).rem_euclid(360.0) - 180.0;

    let sunlon: f64 = -15.0 * (gmtime - 12.0 + eot / 15.0);

    let lon_delta: (f64, f64) = (sunlon - long).to_radians().sin_cos();
    let lat_sc: (f64, f64) = lat.to_radians().sin_cos();

    let sx: f64 = sunlat_sc.1 * lon_delta.0;
    let sy: f64 = lat_sc.1 * sunlat_sc.0 - lat_sc.0 * sunlat_sc.1 * lon_delta.1;
    let sz: f64 = lat_sc.0 * sunlat_sc.0 + lat_sc.1 * sunlat_sc.1 * lon_delta.1;

    let elevation: f64 = f64::asin(sz).to_degrees();
    let azimuth: f64 =
        (f64::atan2(-sx, -sy).to_degrees() + 180.0).rem_euclid(360.0);

    info!(
        "alt: {:.2}, az: {:.2}, ra: {:.2}, dec: {:.2}",
        round(elevation, 0.02),
        round(azimuth, 0.1),
        round(alpha, 0.1),
        round(delta, 0.1)
    );

    Arc::new(SolarInfo {
        elevation: round(elevation, 0.02),
        azimuth: round(azimuth, 0.1),
        right_ascension: round(alpha, 0.1),
        declination: round(delta, 0.1),
    })
}

fn round(v: f64, prec: f64) -> f64 {
    (v / prec).round() * prec
}

async fn run(tx: broadcast::Sender<Info>, lat: f64, long: f64) {
    let mut interval = time::interval(time::Duration::from_secs(15));

    info!("starting task");

    while tx
        .send(get_solar_position(lat, long, &chrono::Utc::now()))
        .is_ok()
    {
        let _ = interval.tick().await;
    }
    warn!("no remaining clients ... terminating");
}

pub fn create_task(
    lat: f64,
    long: f64,
) -> (broadcast::Sender<Info>, broadcast::Receiver<Info>) {
    let (tx, rx) = broadcast::channel(1);
    let tx_copy = tx.clone();

    tokio::spawn(
        async move { run(tx_copy, lat, long).await }
            .instrument(info_span!("solar")),
    );

    (tx, rx)
}

#[cfg(test)]
mod tests {
    use super::get_solar_position;
    use chrono::TimeZone;

    fn close_enough(a: f64, b: f64, delta: f64) -> bool {
        (a - b).abs() <= delta
    }

    struct TestData {
        year: i32,
        month: u32,
        day: u32,
        hour: u32,
        minute: u32,
        second: u32,
        lat: f64,
        long: f64,
        elev: f64,
        az: f64,
        decl: f64,
    }

    #[test]
    fn test_solar() {
        // The calculated values were obtained from
        // https://gml.noaa.gov/grad/solcalc/

        const TEST_DATA: &[TestData] = &[
            // This first group verifies noon, Jan 1st, 2000 at
            // latitudes 45, 0, and -45 with longtitudes 0, 90, 180,
            // and -90.
            TestData {
                year: 2000,
                month: 1,
                day: 1,
                hour: 12,
                minute: 0,
                second: 0,
                lat: 45.0,
                long: 0.0,
                elev: 22.0,
                az: 179.18,
                decl: -23.03,
            },
            TestData {
                year: 2000,
                month: 1,
                day: 1,
                hour: 12,
                minute: 0,
                second: 0,
                lat: 0.0,
                long: 0.0,
                elev: 66.96,
                az: 178.06,
                decl: -23.03,
            },
            TestData {
                year: 2000,
                month: 1,
                day: 1,
                hour: 12,
                minute: 0,
                second: 0,
                lat: -45.0,
                long: 0.0,
                elev: 68.03,
                az: 2.03,
                decl: -23.03,
            },
            TestData {
                year: 2000,
                month: 1,
                day: 1,
                hour: 18,
                minute: 0,
                second: 0,
                lat: 45.0,
                long: -90.0,
                elev: 22.02,
                az: 179.15,
                decl: -23.01,
            },
            TestData {
                year: 2000,
                month: 1,
                day: 1,
                hour: 18,
                minute: 0,
                second: 0,
                lat: 0.0,
                long: -90.0,
                elev: 66.98,
                az: 177.99,
                decl: -23.01,
            },
            TestData {
                year: 2000,
                month: 1,
                day: 1,
                hour: 18,
                minute: 0,
                second: 0,
                lat: -45.0,
                long: -90.0,
                elev: 68.01,
                az: 2.1,
                decl: -23.01,
            },
            TestData {
                year: 2000,
                month: 1,
                day: 1,
                hour: 0,
                minute: 0,
                second: 0,
                lat: 45.0,
                long: -180.0,
                elev: 21.96,
                az: 179.24,
                decl: -23.07,
            },
            TestData {
                year: 2000,
                month: 1,
                day: 1,
                hour: 0,
                minute: 0,
                second: 0,
                lat: 0.0,
                long: -180.0,
                elev: 66.92,
                az: 178.2,
                decl: -23.07,
            },
            TestData {
                year: 2000,
                month: 1,
                day: 1,
                hour: 0,
                minute: 0,
                second: 0,
                lat: -45.0,
                long: -180.0,
                elev: 68.07,
                az: 1.89,
                decl: -23.07,
            },
            TestData {
                year: 2000,
                month: 1,
                day: 1,
                hour: 6,
                minute: 0,
                second: 0,
                lat: 45.0,
                long: 90.0,
                elev: 21.98,
                az: 179.21,
                decl: -23.05,
            },
            TestData {
                year: 2000,
                month: 1,
                day: 1,
                hour: 6,
                minute: 0,
                second: 0,
                lat: 0.0,
                long: 90.0,
                elev: 66.94,
                az: 178.13,
                decl: -23.05,
            },
            TestData {
                year: 2000,
                month: 1,
                day: 1,
                hour: 6,
                minute: 0,
                second: 0,
                lat: -45.0,
                long: 90.0,
                elev: 68.05,
                az: 1.96,
                decl: -23.05,
            },
            // This next set tests two hours before and after noon for
            // longitude 0 and latitudes 45, 0, and -45.
            TestData {
                year: 2010,
                month: 7,
                day: 1,
                hour: 10,
                minute: 0,
                second: 0,
                lat: 45.0,
                long: 0.0,
                elev: 56.65,
                az: 120.65,
                decl: 23.1,
            },
            TestData {
                year: 2010,
                month: 7,
                day: 1,
                hour: 10,
                minute: 0,
                second: 0,
                lat: 0.0,
                long: 0.0,
                elev: 52.09,
                az: 50.33,
                decl: 23.1,
            },
            TestData {
                year: 2010,
                month: 7,
                day: 1,
                hour: 10,
                minute: 0,
                second: 0,
                lat: -45.0,
                long: 0.0,
                elev: 16.34,
                az: 29.53,
                decl: 23.1,
            },
            TestData {
                year: 2010,
                month: 7,
                day: 1,
                hour: 14,
                minute: 0,
                second: 0,
                lat: 45.0,
                long: 0.0,
                elev: 57.79,
                az: 236.87,
                decl: 23.09,
            },
            TestData {
                year: 2010,
                month: 7,
                day: 1,
                hour: 14,
                minute: 0,
                second: 0,
                lat: 0.0,
                long: 0.0,
                elev: 53.55,
                az: 311.29,
                decl: 23.09,
            },
            TestData {
                year: 2010,
                month: 7,
                day: 1,
                hour: 14,
                minute: 0,
                second: 0,
                lat: -45.0,
                long: 0.0,
                elev: 17.0,
                az: 332.18,
                decl: 23.09,
            },
        ];

        for data in TEST_DATA {
            let time = chrono::Utc
                .with_ymd_and_hms(
                    data.year,
                    data.month,
                    data.day,
                    data.hour,
                    data.minute,
                    data.second,
                )
                .single()
                .unwrap();
            let pos = get_solar_position(data.lat, data.long, &time);

            assert!(
                close_enough(pos.elevation, data.elev, 0.2),
                "elevation: {} <> {}",
                pos.elevation,
                data.elev
            );
            assert!(
                close_enough(pos.azimuth, data.az, 0.2),
                "azimuth: {} <> {}",
                pos.azimuth,
                data.az
            );
            assert!(
                close_enough(pos.declination, data.decl, 0.2),
                "declination: {} <> {}",
                pos.declination,
                data.decl
            );
        }
    }
}

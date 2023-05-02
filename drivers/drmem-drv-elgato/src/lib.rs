use std::{convert::Infallible, pin::Pin};
use std::{future::Future, net::Ipv4Addr};
use tracing::{error};

use mdns_sd::{ServiceDaemon, ServiceEvent};
use serde::{Deserialize, Serialize};
use serde_repr::{Deserialize_repr, Serialize_repr};
use tokio::time::{interval_at, Duration, Instant};

use drmem_api::{
    driver::{self, DriverConfig},
    types::{device::Base, Error},
    Result,
};

#[derive(Debug, PartialEq)]
enum DriverState {
    Unknown,  // Initialized, but no state reported
    Ok, // Data received
    Error,    // Data received, but haven't updated since last cycle
}

#[derive(Debug, Serialize_repr, Deserialize_repr, Clone, Copy)]
#[repr(u8)]
enum Power {
    Off = 0,
    On = 1,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy)]
struct Settings {
    on: Power,
    brightness: u8,
    temperature: u16,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct LightState {
    number_of_lights: u8,
    lights: Vec<Settings>,
}

pub struct Instance {
    state: DriverState,
    addr: Ipv4Addr,
    // light_state: LightState,
    d_on: driver::ReportReading<i16>,
    // rx_set_on: driver::RxDeviceSetting,
    // d_brightness: u8,
    // rx_set_brightness: driver::RxDeviceSetting,
    // d_temperature: u16,
    // rx_set_temperature: driver::RxDeviceSetting,
}

impl Instance {
    pub const NAME: &'static str = "Elgato";

    pub const SUMMARY: &'static str = "Monitors and controls Elgato key lights";

    pub const DESCRIPTION: &'static str = include_str!("../README.md");

    // Attempts to pull the gal-per-min parameter from the driver's
    // configuration. The value can be specified as an integer or
    // floating point. It gets returned only as an `f64`.

    // fn get_cfg_name(cfg: &DriverConfig) -> Result<String> {
    //     match cfg.get("name") {
    //         Some(toml::value::Value::String(name)) => return Ok(*name),
    //         Some(_) => error!("'name' config parameter should be a string"),
    //         None => error!("missing 'name' parameter in config"),
    //     }

    //     Err(Error::BadConfig)
    // }

    fn _gen_url(address: Ipv4Addr) -> String {
        format!("http://{}:9123/elgato/lights", address)
    }

    // Attempts to pull the hostname/port for the remote process.

    fn get_cfg_address(cfg: &DriverConfig) -> Result<Ipv4Addr> {
        match cfg.get("addr") {
            Some(toml::value::Value::String(addr)) => {
                if let Ok(addr) = addr.parse::<Ipv4Addr>() {
                    return Ok(addr);
                } else {
                    error!("'addr' not in hostname:port format")
                }
            }
            Some(_) => error!("'addr' config parameter should be a string"),
            None => error!("missing 'addr' parameter in config"),
        }

        Err(Error::BadConfig)
    }

    async fn connect(address: Ipv4Addr) -> Result<LightState> {
        // Get the current status of the light
        // this allows us to fill the struct with the current values
        // The API requires that we send the entire struct back
        let url = Self::_gen_url(address);

        match reqwest::get(url).await?.json::<LightState>().await {
            Err(error) => panic!("Problem getting light status: {:?}", error),
            Ok(status) => Ok(status),
        }
    }

    async fn get_light_status(&mut self) -> Result<LightState> {
        // Get the current status of the light
        // this allows us to fill the struct with the current values
        // The API requires that we send the entire struct back
        let url = Self::_gen_url(self.addr);

        match reqwest::get(url).await?.json::<LightState>().await {
            Err(error) => panic!("Problem getting light status: {:?}", error),
            Ok(status) => Ok(status),
        }
    }
}

impl driver::API for Instance {
    fn create_instance(
        cfg: &DriverConfig, core: driver::RequestChan,
        max_history: Option<usize>,
    ) -> Pin<
        Box<dyn Future<Output = Result<driver::DriverType>> + Send + 'static>,
    > {
        let addr = Instance::get_cfg_address(cfg);

        let fut = async move {
            let addr = addr?;

            let light_state = Instance::connect(addr).await?;

            // Define the devices managed by this driver.

            // let (d_on, rx_set_on, _) =
            //     core.add_rw_device("on".parse::<Base>()?, None, None).await?;
            let (d_on, _) = core
                .add_ro_device("on".parse::<Base>()?, None, None)
                .await?;
            // let (d_brightness, rx_set_brightness, _) = core
            //     .add_rw_device("brightness".parse::<Base>()?, Some("%"), None)
            //     .await?;
            // let (d_temperature, rx_set_temperature, _) = core
            //     .add_rw_device("temperature".parse::<Base>()?, Some("K"), None)
            //     .await?;

            Ok(Box::new(Instance {
                state: DriverState::Unknown,
                addr,
                // light_state,
                d_on,
                // rx_set_on,
                // d_brightness,
                // rx_set_brightness,
                // d_temperature,
                // rx_set_temperature
            }) as driver::DriverType)
        };

        Box::pin(fut)
    }

    fn run<'a>(
        &'a mut self,
    ) -> Pin<Box<dyn Future<Output = Infallible> + Send + 'a>> {
        let fut = async {
            let mut timer = interval_at(Instant::now(), Duration::from_millis(1000));

            loop {
                // Wait for the next sample time.

                timer.tick().await;

                match self.get_light_status().await {
                    Ok(status) => {
                        // (self.light_state)(status).await;
                        (self.d_on)(status.lights[0].on as i16).await;
                        self.state = DriverState::Ok;
                    }

                    Err(e) => {
                        self.state = DriverState::Error;
                        // (self.d_state)(false.into()).await;
                        panic!("couldn't read light state -- {:?}", e);
                    }
                }
            }
        };

        Box::pin(fut)
    }
}

// #[cfg(test)]
// mod tests {
//     use super::*;

//     #[test]
//     fn test_states() {
//         let mut state = State::Unknown;

//         assert_eq!(state.on_event(0), false);
//         assert_eq!(state, State::Unknown);

//         state = State::Off { off_time: 100 };

//         assert_eq!(state.on_event(0), false);
//         assert_eq!(state, State::Off { off_time: 100 });
//         assert_eq!(state.on_event(200), true);
//         assert_eq!(
//             state,
//             State::On {
//                 off_time: 100,
//                 on_time: 200
//             }
//         );

//         assert_eq!(state.on_event(200), false);
//         assert_eq!(
//             state,
//             State::On {
//                 off_time: 100,
//                 on_time: 200
//             }
//         );

//         state = State::Unknown;

//         assert_eq!(state.off_event(1000, 50.0), None);
//         assert_eq!(state, State::Off { off_time: 1000 });
//         assert_eq!(state.off_event(1100, 50.0), None);
//         assert_eq!(state, State::Off { off_time: 1000 });

//         state = State::On {
//             off_time: 1000,
//             on_time: 101000,
//         };

//         assert_eq!(state.off_event(1000, 50.0), None);
//         assert_eq!(state, State::Off { off_time: 1000 });

//         state = State::On {
//             off_time: 1000,
//             on_time: 101000,
//         };

//         assert_eq!(state.off_event(101500, 50.0), None);
//         assert_eq!(
//             state,
//             State::On {
//                 off_time: 1000,
//                 on_time: 101000
//             }
//         );

//         assert!(state.off_event(101501, 50.0).is_some());
//         assert_eq!(state, State::Off { off_time: 101501 });

//         state = State::On {
//             off_time: 0,
//             on_time: 540000,
//         };

//         assert_eq!(state.off_event(600000, 50.0), Some((600000, 10.0, 5.0)));
//         assert_eq!(state, State::Off { off_time: 600000 });

//         state = State::On {
//             off_time: 0,
//             on_time: 54000,
//         };

//         assert_eq!(state.off_event(60000, 60.0), Some((60000, 10.0, 6.0)));
//         assert_eq!(state, State::Off { off_time: 60000 });
//     }
// }

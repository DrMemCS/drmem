use drmem_api::{
    driver::{self, DriverConfig},
    types::{device::Base, Error},
    Result,
};
use elgato_keylight::{
    keylight::{ElgatoError, Status},
    KeyLight,
};
use std::future::Future;
use std::{convert::Infallible, pin::Pin};
use std::time::Duration;
use tracing::{error, info};

fn xlat_err(v: ElgatoError) -> Error {
    match v {
        ElgatoError => Error::OperationError,
    }
}

#[derive(Debug, PartialEq)]
enum DriverState {
    Unknown, // Initialized, but no state reported
    UpToDate, // Data received
    Stale, // Data received, but haven't updated since last cycle
    TimedOut, // No response in the given time
}

pub struct Instance {
    state: DriverState,
    name: String,
    d_on: bool,
    rx_set_on: driver::RxDeviceSetting,
    d_brightness: u8,
    rx_set_brightness: driver::RxDeviceSetting,
    d_temperature: u16,
    rx_set_temperature: driver::RxDeviceSetting,
}

impl Instance {
    pub const NAME: &'static str = "Elgato";

    pub const SUMMARY: &'static str = "monitors and controls Elgato key lights";

    pub const DESCRIPTION: &'static str = include_str!("../README.md");

    /// Creates a new `Instance` instance. It is assumed the external
    /// input is `false` so the initial state is `Unknown`.

    pub fn new(
        name: String,
        d_on: driver::ReportReading,
        rx_set_on: driver::RxDeviceSetting,
        d_brightness: driver::ReportReading,
        rx_set_brightness: driver::RxDeviceSetting,
        d_temperature: driver::ReportReading,
        rx_set_temperature: driver::RxDeviceSetting,
    ) -> Instance {
        Instance {
            state: DriverState::Unknown,
            name,
            d_on,
            rx_set_on,
            d_brightness,
            rx_set_brightness,
            d_temperature,
            rx_set_temperature,
        }
    }

    // Attempts to pull the gal-per-min parameter from the driver's
    // configuration. The value can be specified as an integer or
    // floating point. It gets returned only as an `f64`.

    fn get_cfg_name(cfg: &DriverConfig) -> Result<String> {
        match cfg.get("name") {
            Some(toml::value::Value::String(name)) => return Ok(*name),
            Some(_) => error!("'name' config parameter should be a string"),
            None => error!("missing 'name' parameter in config"),
        }

        Err(Error::BadConfig)
    }

    async fn connect(name: &String) -> Result<Instance> {
        info!("connecting to {}", name);

        //Lookup lamp by name (using zeroconf)
        let kl =
            KeyLight::new_from_name(name, Some(Duration::from_millis(2_000)))
                .await
                .map_err(xlat_err)?;
        let status = kl.get().await.map_err(xlat_err)?;
        let firstLight = status.lights.first();

        match firstLight {
            Some(v) => Ok(Instance::new(
                *name,
                d_on: v.on,
                rx_set_on: driver::RxDeviceSetting,
                d_brightness: v.brightness,
                rx_set_brightness: driver::RxDeviceSetting,
                d_temperature: v.temperature,
                rx_set_temperature: driver::RxDeviceSetting,
            )),
            None => Err(Error::NotFound),
        }
    }

    async fn get_status(device: KeyLight) -> Result<Status> {
        // info!("getting status for {}", device.name());

        //Get the lamp status
        let status = device.get().await.map_err(xlat_err)?;
        // println!("{:?}", status);

        Ok(status)
    }
}

impl driver::API for Instance {
    fn create_instance(
        cfg: DriverConfig, core: driver::RequestChan,
    ) -> Pin<
        Box<dyn Future<Output = Result<driver::DriverType>> + Send + 'static>,
    > {
        let fut = async move {
            //Lookup lamp by name (using zeroconf)
            let name = Instance::get_cfg_name(&cfg)?;

            let lightInstance = Instance::connect(&name).await?;

            // Define the devices managed by this driver.

            let (d_on, rx_set_on, _) =
                core.add_rw_device("on".parse::<Base>()?, None).await?;
            let (d_brightness, rx_set_brightness, _) = core
                .add_rw_device("brightness".parse::<Base>()?, Some("%"))
                .await?;
            let (d_temperature, rx_set_temperature, _) = core
                .add_rw_device("temperature".parse::<Base>()?, Some("K"))
                .await?;

            Ok(Box::new(Instance::new(
                name,
                d_on,
                rx_set_on,
                d_brightness,
                rx_set_brightness,
                d_temperature,
                rx_set_temperature
            )) as driver::DriverType)
        };

        Box::pin(fut)
    }

    fn run<'a>(
        &'a mut self,
    ) -> Pin<Box<dyn Future<Output = Infallible> + Send + 'a>> {
        let fut = async {
            (self.d_service)(true.into()).await;

            loop {
                match self.get_reading().await {
                    Ok((stamp, true)) => {
                        if self.state.on_event(stamp) {
                            (self.d_state)(true.into()).await;
                        }
                    }

                    Ok((stamp, false)) => {
                        let gpm = self.gpm;

                        if let Some((cycle, duty, in_flow)) =
                            self.state.off_event(stamp, gpm)
                        {
                            info!(
                                "cycle: {}, duty: {:.1}%, inflow: {:.2} gpm",
                                Instance::elapsed(cycle),
                                duty,
                                in_flow
                            );

                            (self.d_state)(false.into()).await;
                            (self.d_duty)(duty.into()).await;
                            (self.d_inflow)(in_flow.into()).await;
                        }
                    }

                    Err(e) => {
                        (self.d_state)(false.into()).await;
                        (self.d_service)(false.into()).await;
                        panic!("couldn't read sump state -- {:?}", e);
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

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
    d_on: driver::ReportReading<u16>,
    d_brightness: driver::ReportReading<u16>,
    d_temperature: driver::ReportReading<u16>,
}

impl Instance {
    pub const NAME: &'static str = "Elgato";

    pub const SUMMARY: &'static str = "Monitors and controls Elgato key lights";

    pub const DESCRIPTION: &'static str = include_str!("../README.md");

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

            // Define the devices managed by this driver.
            let (d_on, _) = core
                .add_ro_device("on".parse::<Base>()?, None, max_history)
                .await?;
            let (d_brightness, _) = core
                .add_ro_device("brightness".parse::<Base>()?, None, max_history)
                .await?;
            let (d_temperature, _) = core
                .add_ro_device("temperature".parse::<Base>()?, None, max_history)
                .await?;

            Ok(Box::new(Instance {
                state: DriverState::Unknown,
                addr,
                d_on,
                d_brightness,
                d_temperature,
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
                        (self.d_on)(status.lights[0].on as u16).await;
                        (self.d_brightness)(status.lights[0].brightness as u16).await;
                        (self.d_temperature)(status.lights[0].temperature as u16).await;
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

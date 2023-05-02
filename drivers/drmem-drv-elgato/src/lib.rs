use std::{convert::Infallible, future::Future, net::Ipv4Addr, pin::Pin};
use tracing::error;

use serde::{Deserialize, Serialize};
use serde_repr::{Deserialize_repr, Serialize_repr};
use tokio::time::{interval_at, Duration, Instant};
use tokio_stream::StreamExt;

use drmem_api::{
    driver::{self, DriverConfig},
    types::{device::Base, Error},
    Result,
};

#[derive(Debug, PartialEq)]
enum DriverState {
    Unknown, // Initialized, but no state reported
    Ok,      // Data received
    Error,   // Data received, but haven't updated since last cycle
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
    brightness: u16,
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
    d_on: driver::ReportReading<bool>,
    s_on: driver::SettingStream<bool>,
    d_brightness: driver::ReportReading<u16>,
    s_brightness: driver::SettingStream<u16>,
    d_temperature: driver::ReportReading<u16>,
    s_temperature: driver::SettingStream<u16>,
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
        let url = Instance::_gen_url(self.addr);

        match reqwest::get(url).await?.json::<LightState>().await {
            Err(error) => panic!("Problem getting light status: {:?}", error),
            Ok(status) => Ok(status),
        }
    }

    async fn set_light_state(&mut self, status: LightState) {
        // Make a PUT request to toggle the light power
        reqwest::Client::new()
            .put(&Instance::_gen_url(self.addr))
            .json(&status)
            .send()
            .await
            .ok();
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
            let (d_on, s_on, _) = core
                .add_rw_device("on".parse::<Base>()?, None, max_history)
                .await?;
            let (d_brightness, s_brightness, _) = core
                .add_rw_device("brightness".parse::<Base>()?, None, max_history)
                .await?;
            let (d_temperature, s_temperature, _) = core
                .add_rw_device(
                    "temperature".parse::<Base>()?,
                    None,
                    max_history,
                )
                .await?;

            Ok(Box::new(Instance {
                state: DriverState::Unknown,
                addr,
                d_on,
                s_on,
                d_brightness,
                s_brightness,
                d_temperature,
                s_temperature,
            }) as driver::DriverType)
        };

        Box::pin(fut)
    }

    fn run<'a>(
        &'a mut self,
    ) -> Pin<Box<dyn Future<Output = Infallible> + Send + 'a>> {
        let fut = async {
            let mut timer =
                interval_at(Instant::now(), Duration::from_millis(1000));

            loop {
                // Wait for the next sample time.
                timer.tick().await;

                match self.get_light_status().await {
                    Ok(status) => {
                        // (self.light_state)(status).await;
                        (self.d_on)(match status.lights[0].on {
                            Power::Off => false,
                            Power::On => true,
                        }).await;
                        (self.d_brightness)(status.lights[0].brightness as u16)
                            .await;
                        (self.d_temperature)(
                            status.lights[0].temperature as u16,
                        )
                        .await;
                        self.state = DriverState::Ok;
                        status
                    }

                    Err(e) => {
                        self.state = DriverState::Error;
                        // (self.d_state)(false.into()).await;
                        panic!("couldn't read light state -- {:?}", e);
                    }
                };

                if let Some((v, reply)) = self.s_on.next().await {
                    reply(Ok(v.clone()));
                    let mut status = match self.get_light_status().await {
                        Ok(status) => status,
                        Err(e) => panic!("couldn't read light state -- {:?}", e),
                    };
                    match v {
                        false => status.lights[0].on = Power::Off,
                        true => status.lights[0].on = Power::On,
                    };
                    self.set_light_state(status).await
                } else {
                    panic!("can no longer receive settings");
                }

                if let Some((v, reply)) = self.s_brightness.next().await {
                    reply(Ok(v.clone()));
                    let mut status = match self.get_light_status().await {
                        Ok(status) => status,
                        Err(e) => panic!("couldn't read light state -- {:?}", e),
                    };
                    status.lights[0].brightness = v;
                    self.set_light_state(status).await
                } else {
                    panic!("can no longer receive settings");
                }

                if let Some((v, reply)) = self.s_temperature.next().await {
                    reply(Ok(v.clone()));
                    let mut status = match self.get_light_status().await {
                        Ok(status) => status,
                        Err(e) => panic!("couldn't read light state -- {:?}", e),
                    };
                    status.lights[0].temperature = v;
                    self.set_light_state(status).await
                } else {
                    panic!("can no longer receive settings");
                }
            }
        };

        Box::pin(fut)
    }
}

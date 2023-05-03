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
    poll_interval: Duration,
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

    // Attempts to pull the hostname for the light.
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

    // Attempts to pull the poll interval for the light.
    fn get_cfg_poll_interval(cfg: &DriverConfig) -> Result<Duration> {
        match cfg.get("poll_interval") {
            Some(toml::value::Value::Integer(interval)) => {
                return Ok(Duration::from_millis(*interval as u64));
            }
            Some(_) => {
                error!("'poll_interval' config parameter should be an integer")
            }
            None => return Ok(Duration::from_millis(3000)),
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
        let poll_interval = Instance::get_cfg_poll_interval(cfg);

        let fut = async move {
            let addr = addr?;
            let poll_interval = poll_interval?;

            // Define the devices managed by this driver.
            let (d_on, s_on, _) = core
                .add_rw_device("on".parse::<Base>()?, None, max_history)
                .await?;
            let (d_brightness, s_brightness, _) = core
                .add_rw_device(
                    "brightness".parse::<Base>()?,
                    Some("%"),
                    max_history,
                )
                .await?;
            let (d_temperature, s_temperature, _) = core
                .add_rw_device(
                    "temperature".parse::<Base>()?,
                    Some("K"),
                    max_history,
                )
                .await?;

            Ok(Box::new(Instance {
                state: DriverState::Unknown,
                addr,
                poll_interval,
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
            let mut timer = interval_at(Instant::now(), self.poll_interval);
            let mut on: Option<bool> = None;
            let mut brightness: Option<u16> = None;
            let mut temperature: Option<u16> = None;

            // https://github.com/adamesch/elgato-key-light-api/tree/master/resources/lights
            // This doc was useful for figuring out the API
            loop {
                tokio::select! {
                    // Wait for the next sample time.
                    _ = timer.tick() => {
                        match self.get_light_status().await {
                            Ok(status) => {
                                let new_on = Some(match status.lights[0].on {
                                    Power::Off => false,
                                    Power::On => true,
                                });
                                let new_brightness = Some(status.lights[0].brightness as u16);
                                let new_temperature = Some(status.lights[0].temperature as u16);

                                if on != new_on {
                                    (self.d_on)(new_on.unwrap()).await;
                                    on = new_on;
                                }

                                if brightness != new_brightness {
                                    (self.d_brightness)(new_brightness.unwrap()).await;
                                    brightness = new_brightness;
                                }

                                if temperature != new_temperature {
                                    (self.d_temperature)(new_temperature.unwrap()).await;
                                    temperature = new_temperature;
                                }

                                self.state = DriverState::Ok;
                                status
                            }

                            Err(e) => {
                                self.state = DriverState::Error;
                                panic!("couldn't read light state -- {:?}", e);
                            }
                        };
                    }

                    Some((v, reply)) = self.s_on.next() => {
                        let mut status = match self.get_light_status().await {
                            Ok(status) => status,
                            Err(e) => {
                                panic!("couldn't read light state -- {:?}", e)
                            }
                        };
                        match v {
                            false => status.lights[0].on = Power::Off,
                            true => status.lights[0].on = Power::On,
                        };
                        reply(Ok(v.clone()));
                        self.set_light_state(status).await;
                    }

                    Some((v, reply)) = self.s_brightness.next() => {
                        // Clamp the brightness to the range 3-100
                        let clamped_brightness = match v {
                            v if v < 3 => 3,
                            v if v > 100 => 100,
                            v => v,
                        };
                        let mut status = match self.get_light_status().await {
                            Ok(status) => status,
                            Err(e) => {
                                panic!("couldn't read light state -- {:?}", e)
                            }
                        };
                        status.lights[0].brightness = clamped_brightness;
                        reply(Ok(clamped_brightness.clone()));
                        self.set_light_state(status).await;
                    }

                    Some((v, reply)) = self.s_temperature.next() => {
                        // Clamp the temperature to the range 2900-7000
                        // This can only be adjusted in 50K increments
                        // The light API uses scaled values, so we need to
                        // divide 1_000_000 by the temperature
                        // and then round to the nearest natural number
                        let clamped_temperature = match v {
                            v if v < 2900 => 2900,
                            v if v > 7000 => 7000,
                            v => v,
                        };
                        // This is wrong because 2900K results in 345
                        // which is out of the bounds of the scaled values
                        // 143-344, but it will work for all other clamped
                        // Kelvin values
                        // The light clamps this value as well, so we don't
                        // need to worry about it
                        let v = (1_000_000.0 / clamped_temperature as f32).round() as u16;

                        let mut status = match self.get_light_status().await {
                            Ok(status) => status,
                            Err(e) => {
                                panic!("couldn't read light state -- {:?}", e)
                            }
                        };
                        status.lights[0].temperature = v;
                        reply(Ok(v.clone()));
                        self.set_light_state(status).await;
                    }
                }
            }
        };

        Box::pin(fut)
    }
}

use super::{config, device};
use drmem_api::{
    Error, Result,
    driver::{API, Reporter},
};
use palette::{IntoColor, LinSrgb, LinSrgba, Yxy};
use reqwest::{
    Client,
    header::{HeaderMap, HeaderValue},
};
use std::{convert::Infallible, sync::Arc};
use tokio::{sync::mpsc, task::JoinHandle, time::Duration};
use tracing::{Span, info};

mod hue_streamer;
mod payload;

pub struct Instance {
    client: Client,
    host: Arc<str>,
    updates: mpsc::Receiver<payload::ResourceData>,
    update_task: JoinHandle<Result<Infallible>>,
}

impl Instance {
    pub const NAME: &'static str = "hue";

    pub const SUMMARY: &'static str =
        "controls devices registered with a Philips Hue bridge";

    pub const DESCRIPTION: &'static str = include_str!("../../README.md");

    // Creates a new instance of the driver state.

    pub fn new<R: Reporter>(cfg: &<Self as API<R>>::Config) -> Result<Self> {
        // Every request needs to have the App ID so this section of code
        // makes it one of the default headers.

        let mut hdr_map: HeaderMap = HeaderMap::new();

        hdr_map.insert(
            "hue-application-key",
            HeaderValue::from_str(&cfg.app_id).map_err(|e| {
                Error::ConfigError(format!(
                    "config error with app key -- {}",
                    e
                ))
            })?,
        );

        static APP_USER_AGENT: &str =
            concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"),);

        // Build the client with our desired defaults.

        let client = Client::builder()
            .danger_accept_invalid_certs(true)
            .user_agent(APP_USER_AGENT)
            .default_headers(hdr_map)
            .use_rustls_tls()
            .tcp_keepalive_interval(Duration::from_secs(30))
            .connect_timeout(Duration::from_millis(500))
            .build()
            .map_err(|e| {
                Error::OperationError(format!(
                    "can't create connection -- {}",
                    e
                ))
            })?;

        let (tx, rx) = mpsc::channel(100);
        let update_task =
            hue_streamer::start(cfg.host.clone(), client.clone(), tx);

        Ok(Instance {
            host: cfg.host.clone(),
            client,
            updates: rx,
            update_task,
        })
    }
}

impl<R: Reporter> API<R> for Instance {
    type Config = config::Params;
    type HardwareType = device::Set<R>;

    async fn create_instance(cfg: &Self::Config) -> Result<Box<Self>> {
        Span::current().record("cfg", &*cfg.host);
        Self::new::<R>(cfg).map(Box::new)
    }

    // Main run loop for the Hue driver.

    async fn run<'a>(
        &'a mut self,
        devices: &'a mut Self::HardwareType,
    ) -> Infallible {
        loop {
            tokio::select! {
                Some(update) = self.updates.recv() => {
                    info!("hue update: {:?}", &update);

                    // Look up the device ID in the hardware map. If it doesn't
                    // exist, ignore the update. If it does, report the new
                    // state to the appropriate setting(s).

                    if let Some(dev_set) = devices.map.get_mut(update.id.as_str()) {
                        info!("found {} for update", &update.id);
                        match dev_set {
                            device::DeviceSet::Switch(switch) => {
                                if let Some(on) = update.on {
                                    info!("reporting switch update: {}", on.on);
                                    switch.state.report_update(on.on).await;
                                }
                            }

                            device::DeviceSet::Bulb(dimmer) => {
                                if let Some(on) = update.on {
                                    if !on.on {
                                        info!("turning off bulb");
                                        dimmer.brightness.report_update(0.0).await;
                                    } else if let Some(dim) = update.dimming {
                                        info!("reporting dimmer update: {}", dim.brightness);
                                        dimmer.brightness.report_update(dim.brightness as f64).await;
                                    } else {
                                        info!("turning on bulb");
                                        dimmer.brightness.report_update(100.0).await;
                                    }
                                } else if let Some(dim) = update.dimming {
                                    info!("reporting dimmer update: {}", dim.brightness);
                                    dimmer.brightness.report_update(dim.brightness as f64).await;
                                }
                            }

                            device::DeviceSet::ColorBulb(color_bulb) | device::DeviceSet::Group(color_bulb) => {
                                // Handle brightness updates
                                if let Some(on) = &update.on {
                                    if !on.on {
                                        info!("turning off color bulb");
                                        color_bulb.brightness.report_update(0.0).await;
                                    } else if let Some(dim) = &update.dimming {
                                        info!("reporting color bulb brightness update: {}", dim.brightness);
                                        color_bulb.brightness.report_update(dim.brightness as f64).await;
                                    }
                                } else if let Some(dim) = &update.dimming {
                                    info!("reporting color bulb brightness update: {}", dim.brightness);
                                    color_bulb.brightness.report_update(dim.brightness as f64).await;
                                }

                                // Handle color updates using palette's CIE XY conversion
                                if let Some(color) = &update.color {
                                    let yxy = Yxy::new(color.xy.x, color.xy.y, 1.0);
                                    let rgb: LinSrgb = yxy.into_color();

                                    let rgba = LinSrgba::new(
                                        (rgb.red.clamp(0.0, 1.0) * 255.0) as u8,
                                        (rgb.green.clamp(0.0, 1.0) * 255.0) as u8,
                                        (rgb.blue.clamp(0.0, 1.0) * 255.0) as u8,
                                        255,
                                    );
                                    info!("reporting color bulb color update: {:?}", rgba);
                                    color_bulb.color.report_update(rgba).await;
                                }
                            }
                        }
                    }
                }

                Err(e) = &mut self.update_task => {
                    panic!("Hue stream task failed -- {}", e)
                }
            }
        }
    }
}

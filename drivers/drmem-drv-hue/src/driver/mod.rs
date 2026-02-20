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
use tracing::{Level, Span, error, instrument};

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

    async fn sync_initial_state<R: Reporter>(
        &self,
        devices: &mut device::Set<R>,
    ) -> Result<()> {
        for rtype in &["light", "grouped_light"] {
            let url =
                format!("https://{}/clip/v2/resource/{}", self.host, rtype);

            let resp = self.client
                .get(url)
                .send()
                .await
                .map_err(|e| Error::OperationError(format!("request failed: {e}")))?
                .error_for_status() // Catch 401, 403, 404, etc.
                .map_err(|e| Error::OperationError(format!("HTTP error: {e}")))?;

            // Fetch as text first to allow for debug logging on failure

            let body = resp.text().await.map_err(|e| {
                Error::OperationError(format!("failed to read body: {e}"))
            })?;

            let payload: payload::HueResponse = serde_json::from_str(&body)
                .map_err(|e| {
                    error!("Hue JSON mismatch: {}. Body: {}", e, body);
                    Error::OperationError(format!("JSON decode error: {e}"))
                })?;

            for update in payload.data {
                if let Some(dev_set) = devices.map.get_mut(update.id.as_ref()) {
                    Self::report_update(dev_set, update).await;
                }
            }
        }
        Ok(())
    }

    #[instrument(level = Level::INFO, name = "report", skip(dev_set, update), fields(id = update.id.as_ref()))]
    async fn report_update<R: Reporter>(
        dev_set: &mut device::DeviceSet<R>,
        update: payload::ResourceData,
    ) {
        match dev_set {
            device::DeviceSet::Switch(switch) => {
                if let Some(on) = update.on {
                    switch.state.report_update(on.on).await;
                }
            }

            device::DeviceSet::Bulb(dimmer) => {
                if let Some(on) = update.on {
                    if !on.on {
                        dimmer.brightness.report_update(0.0).await;
                    } else if let Some(dim) = update.dimming {
                        dimmer
                            .brightness
                            .report_update(dim.brightness as f64)
                            .await;
                    } else {
                        dimmer.brightness.report_update(100.0).await;
                    }
                } else if let Some(dim) = update.dimming {
                    dimmer
                        .brightness
                        .report_update(dim.brightness as f64)
                        .await;
                }
            }

            device::DeviceSet::ColorBulb(color_bulb)
            | device::DeviceSet::Group(color_bulb) => {
                // Handle brightness updates
                if let Some(on) = &update.on {
                    if !on.on {
                        color_bulb.brightness.report_update(0.0).await;
                    } else if let Some(dim) = &update.dimming {
                        color_bulb
                            .brightness
                            .report_update(dim.brightness as f64)
                            .await;
                    }
                } else if let Some(dim) = &update.dimming {
                    color_bulb
                        .brightness
                        .report_update(dim.brightness as f64)
                        .await;
                }

                // Handle color updates using palette's CIE XY conversion

                if let Some(color) =
                    &update.color.as_ref().and_then(|c| c.xy.as_ref())
                {
                    let yxy = Yxy::new(color.x, color.y, 1.0);
                    let rgb: LinSrgb = yxy.into_color();

                    let rgba = LinSrgba::new(
                        (rgb.red.clamp(0.0, 1.0) * 255.0) as u8,
                        (rgb.green.clamp(0.0, 1.0) * 255.0) as u8,
                        (rgb.blue.clamp(0.0, 1.0) * 255.0) as u8,
                        255,
                    );
                    color_bulb.color.report_update(rgba).await;
                }
            }
        }
    }
}

impl<R: Reporter> API<R> for Instance {
    type Config = config::Params;
    type HardwareType = device::Set<R>;

    async fn create_instance(cfg: &Self::Config) -> Result<Box<Self>> {
        Span::current().record("cfg", cfg.host.as_ref());
        Self::new::<R>(cfg).map(Box::new)
    }

    // Main run loop for the Hue driver.

    async fn run<'a>(
        &'a mut self,
        devices: &'a mut Self::HardwareType,
    ) -> Infallible {
        if let Err(e) = self.sync_initial_state(devices).await {
            panic!("failed to sync initial hue state: {e}");
        }

        loop {
            tokio::select! {
                Some(update) = self.updates.recv() => {

                    // Look up the device ID in the hardware map. If it doesn't
                    // exist, ignore the update. If it does, report the new
                    // state to the appropriate setting(s).

                    if let Some(dev_set) = devices.map.get_mut(update.id.as_ref()) {
                        Self::report_update(dev_set, update).await;
                    }
                }

                Err(e) = &mut self.update_task => {
                    panic!("Hue stream task failed -- {}", e)
                }
            }
        }
    }
}

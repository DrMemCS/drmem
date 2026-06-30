use super::{config, device};
use drmem_api::{
    Error, Result,
    driver::{API, Reporter},
};
use reqwest::{
    Client,
    header::{HeaderMap, HeaderValue},
};
use std::{convert::Infallible, future::pending, sync::Arc};
use tokio::{sync::Mutex, time::Duration};
use tracing::{Level, Span, error, instrument};

pub(crate) mod color;
pub(crate) mod constants;
pub(crate) mod device_traits;
pub(crate) mod payload;

use constants::{GROUPED_LIGHT_RESOURCE, LIGHT_RESOURCE};

pub struct Instance {
    client: Arc<Mutex<Client>>,
    host: Arc<str>,
    poll_interval: Duration,
}

impl Instance {
    pub const NAME: &'static str = "hue";

    pub const SUMMARY: &'static str =
        "controls devices registered with a Philips Hue bridge";

    pub const DESCRIPTION: &'static str = include_str!("../../README.md");

    // Creates a new instance of the driver state.

    pub fn new<R: Reporter>(cfg: &<Self as API<R>>::Config) -> Result<Self> {
        let _ = rustls::crypto::ring::default_provider().install_default();

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

        hdr_map.insert(
            "content-type",
            HeaderValue::from_str("application/json").unwrap(),
        );

        static APP_USER_AGENT: &str =
            concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"),);

        // Build the client with our desired defaults.

        let client = Client::builder()
            .danger_accept_invalid_certs(true)
            .user_agent(APP_USER_AGENT)
            .default_headers(hdr_map)
            .use_rustls_tls()
            .tcp_keepalive(Duration::from_secs(30))
            .connect_timeout(Duration::from_millis(500))
            .build()
            .map_err(|e| {
                Error::OperationError(format!(
                    "can't create connection -- {}",
                    e
                ))
            })?;

        Ok(Instance {
            host: cfg.host.clone(),
            client: Arc::new(Mutex::new(client)),
            poll_interval: Duration::from_secs(cfg.poll_interval_secs),
        })
    }

    // Poll all devices and update their states from the bridge
    async fn poll_all_devices<R: Reporter>(
        &self,
        devices: &mut device::Set<R>,
    ) -> Result<()> {
        for rtype in &[LIGHT_RESOURCE, GROUPED_LIGHT_RESOURCE] {
            let url = constants::resource_url(&self.host, rtype);

            let client = self.client.lock().await;
            let resp = client
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
                    dev_set.apply_update(&update).await;
                }
            }
        }
        Ok(())
    }
}

impl<R: Reporter> API<R> for Instance {
    type Config = config::Params;
    type HardwareType = device::Set<R>;

    async fn create_instance(cfg: &Self::Config) -> Result<Box<Self>> {
        Span::current().record("cfg", cfg.host.as_ref());
        Self::new::<R>(cfg).map(Box::new)
    }

    // Main run loop for the Hue driver - spawns per-device loops

    async fn run<'a>(
        &'a mut self,
        devices: &'a mut Self::HardwareType,
    ) -> Infallible {
        // Initial state sync
        if let Err(e) = self.poll_all_devices(devices).await {
            error!("failed to sync initial hue state: {}", e);
            // Continue anyway - devices will sync on first poll
        }

        // Spawn a task for each device that manages its own timeouts
        let mut tasks = Vec::new();

        // Take ownership of devices map and iterate
        let devices_map = std::mem::take(&mut devices.map);

        for (id, mut dev) in devices_map {
            let client = Arc::clone(&self.client);
            let host = self.host.clone();
            let poll_interval = self.poll_interval;
            let rtype = dev.resource_type();

            let task = tokio::spawn(async move {
                Self::device_loop(
                    client,
                    host,
                    poll_interval,
                    id,
                    rtype,
                    &mut dev,
                )
                .await
            });

            tasks.push(task);
        }

        // Wait for all device loops (they run forever)
        for task in tasks {
            let _ = task.await;
        }

        // This should never return, but if all tasks die, return Infallible

        pending::<()>().await;
        unreachable!()
    }
}

impl Instance {
    // Per-device loop that manages its own timeouts
    async fn device_loop<R: Reporter>(
        client: Arc<Mutex<Client>>,
        host: Arc<str>,
        poll_interval: Duration,
        id: Arc<str>,
        rtype: &'static str,
        dev: &mut device_traits::DeviceWrapper<R>,
    ) {
        let mut poll_timer = tokio::time::interval(poll_interval);

        loop {
            tokio::select! {
                _ = poll_timer.tick() => {
                }

                opt_cmd = dev.next_setting() => {
                    if let Some(cmd) = opt_cmd {
                        Self::send_command_direct(&client, &host, &id, rtype, cmd).await;

                        // Immediately poll this device to get its new state
                        poll_timer.reset();
                    } else {
                        continue;
                    }
                }
            }
            Self::poll_device_direct(&client, &host, &id, rtype, dev).await;
        }
    }

    // Direct device poll (doesn't need the devices map)
    async fn poll_device_direct<R: Reporter>(
        client: &Arc<Mutex<Client>>,
        host: &str,
        id: &str,
        rtype: &str,
        dev: &mut device_traits::DeviceWrapper<R>,
    ) {
        let url = constants::device_url(host, rtype, id);

        let client_guard = client.lock().await;
        match client_guard.get(&url).send().await {
            Ok(resp) => match resp.error_for_status() {
                Ok(resp) => match resp.text().await {
                    Ok(body) => {
                        match serde_json::from_str::<payload::HueResponse>(
                            &body,
                        ) {
                            Ok(payload) => {
                                for update in payload.data {
                                    Self::report_update(dev, &update).await;
                                }
                            }
                            Err(e) => error!(
                                "Failed to parse poll response for {}: {}",
                                id, e
                            ),
                        }
                    }
                    Err(e) => {
                        error!("Failed to read poll response for {}: {}", id, e)
                    }
                },
                Err(e) => error!("Hue bridge error polling {}: {}", id, e),
            },
            Err(e) => error!("Failed to poll device {}: {}", id, e),
        }
    }

    // Direct command send (doesn't need the devices map)
    #[instrument(level = Level::INFO, name = "control", skip(client), fields(id = id, r#type = rtype))]
    async fn send_command_direct(
        client: &Arc<Mutex<Client>>,
        host: &str,
        id: &str,
        rtype: &str,
        cmd: payload::LightCommand,
    ) {
        let url = constants::device_url(host, rtype, id);
        let body_str = serde_json::to_string(&cmd).unwrap();
        let client_guard = client.lock().await;

        match client_guard.put(&url).body(body_str).send().await {
            Ok(resp) => {
                if let Err(e) = resp.error_for_status() {
                    error!("Hue bridge rejected setting for {}: {}", id, e);
                }
            }
            Err(e) => error!("Failed to communicate with Hue bridge: {}", e),
        }
    }

    #[instrument(level = Level::INFO, name = "report", skip(dev_wrapper, update), fields(id = update.id.as_ref()))]
    async fn report_update<R: Reporter>(
        dev_wrapper: &mut device_traits::DeviceWrapper<R>,
        update: &payload::ResourceData,
    ) {
        dev_wrapper.apply_update(update).await;
    }
}

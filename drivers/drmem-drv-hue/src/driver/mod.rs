use super::{config, device};
use drmem_api::{
    Error, Result,
    driver::{API, Reporter},
};
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

    async fn run<'a>(
        &'a mut self,
        _devices: &'a mut Self::HardwareType,
    ) -> Infallible {
        loop {
            tokio::select! {
                Some(update) = self.updates.recv() => {
                    info!("hue update: {:?}", update)
                }

                Err(e) = &mut self.update_task => {
                    panic!("Hue stream task failed -- {}", e)
                }
            }
        }
    }
}

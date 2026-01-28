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
use tokio::time::Duration;

mod hue_streamer;
mod payload;

pub struct Instance {
    client: Client,
    host: Arc<str>,
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
            .user_agent(APP_USER_AGENT)
            .default_headers(hdr_map)
            .danger_accept_invalid_certs(true)
            .timeout(Duration::from_millis(500))
            .http2_prior_knowledge()
            .tcp_keepalive_interval(Duration::from_secs(30))
            .build()
            .map_err(|e| {
                Error::OperationError(format!(
                    "can't create connection -- {}",
                    e
                ))
            })?;

        Ok(Instance {
            host: cfg.host.clone(),
            client,
        })
    }
}

impl<R: Reporter> API<R> for Instance {
    type Config = config::Params;
    type HardwareType = device::Set<R>;

    async fn create_instance(cfg: &Self::Config) -> Result<Box<Self>> {
        Self::new::<R>(cfg).map(Box::new)
    }

    async fn run<'a>(
        &'a mut self,
        devices: &'a mut Self::HardwareType,
    ) -> Infallible {
        todo!()
    }
}

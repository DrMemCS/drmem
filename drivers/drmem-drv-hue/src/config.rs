use drmem_api::{Error, device::Path, driver::DriverConfig};
use std::sync::Arc;

#[derive(serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DevCfgType {
    Switch,
    Dimmer,
    Bulb,
    ColorBulb,
    Group,
}

#[derive(serde::Deserialize)]
pub struct DeviceConfig {
    pub subpath: Arc<Path>,
    pub id: Arc<str>,
    pub r#type: DevCfgType,
    /// Override timeout in seconds
    pub override_timeout: Option<u64>,
    #[serde(default = "default_poll_on_change")]
    pub poll_on_change: bool,
}

fn default_poll_on_change() -> bool {
    true
}

#[derive(serde::Deserialize)]
pub struct Params {
    pub host: Arc<str>,
    pub app_id: Arc<str>,
    pub devices: Vec<DeviceConfig>,
    #[serde(default = "default_poll_interval")]
    pub poll_interval_secs: u64,
}

fn default_poll_interval() -> u64 {
    5
}

impl TryFrom<DriverConfig> for Params {
    type Error = Error;

    fn try_from(cfg: DriverConfig) -> std::result::Result<Self, Self::Error> {
        cfg.parse_into()
    }
}

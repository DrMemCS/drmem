use drmem_api::{driver::DriverConfig, Error};
use std::net::SocketAddrV4;

#[derive(serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DevCfgType {
    #[serde(alias = "Switch", alias = "SWITCH")]
    Switch,
    #[serde(alias = "Outlet", alias = "OUTLET")]
    Outlet,
    #[serde(alias = "Dimmer", alias = "DIMMER")]
    Dimmer,
}

#[derive(serde::Deserialize)]
pub struct Params {
    pub addr: SocketAddrV4,
    pub r#type: DevCfgType,
    pub override_timeout: Option<u64>,
}

impl TryFrom<DriverConfig> for Params {
    type Error = Error;

    fn try_from(cfg: DriverConfig) -> std::result::Result<Self, Self::Error> {
        cfg.parse_into()
    }
}

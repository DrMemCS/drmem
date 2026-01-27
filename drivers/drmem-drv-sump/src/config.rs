use drmem_api::{driver::DriverConfig, Error};
use std::net::SocketAddrV4;

#[derive(serde::Deserialize)]
pub struct Params {
    pub addr: SocketAddrV4,
    pub gpm: f64,
}

impl TryFrom<DriverConfig> for Params {
    type Error = Error;

    fn try_from(cfg: DriverConfig) -> std::result::Result<Self, Self::Error> {
        cfg.parse_into()
    }
}

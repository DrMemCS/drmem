use drmem_api::{
    driver::{classes, DriverConfig},
    Error,
};
use std::convert::TryFrom;

#[derive(serde::Deserialize)]
pub struct Params {
    pub api_key: Option<String>,
    pub station: String,
    pub interval: Option<u64>,
    pub units: classes::WeatherUnits,
}

impl TryFrom<DriverConfig> for Params {
    type Error = Error;

    fn try_from(cfg: DriverConfig) -> std::result::Result<Self, Self::Error> {
        cfg.parse_into()
    }
}

use drmem_api::{device, driver::DriverConfig, Error};

#[derive(serde::Deserialize)]
pub struct Params {
    pub disabled: device::Value,
    pub enabled: device::Value,
}

impl TryFrom<DriverConfig> for Params {
    type Error = Error;

    fn try_from(cfg: DriverConfig) -> std::result::Result<Self, Self::Error> {
        cfg.parse_into()
    }
}

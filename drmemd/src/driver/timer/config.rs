use drmem_api::{device, driver, Error};

#[derive(serde::Deserialize)]
pub struct Params {
    pub millis: u64,
    pub disabled: device::Value,
    pub enabled: device::Value,
}

impl TryFrom<driver::DriverConfig> for Params {
    type Error = Error;

    fn try_from(
        cfg: driver::DriverConfig,
    ) -> std::result::Result<Self, Self::Error> {
        cfg.parse_into()
    }
}

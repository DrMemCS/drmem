use drmem_api::{driver::DriverConfig, Error};

#[derive(serde::Deserialize)]
pub struct Params;

impl TryFrom<DriverConfig> for Params {
    type Error = Error;

    fn try_from(_cfg: DriverConfig) -> std::result::Result<Self, Self::Error> {
        Ok(Params)
    }
}

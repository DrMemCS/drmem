use drmem_api::{device, driver::DriverConfig, Error};

#[derive(serde::Deserialize)]
pub struct Params {
    pub millis: u64,
    #[serde(default = "cfg_eab_default")]
    pub enabled_at_boot: bool,
    pub disabled: device::Value,
    pub enabled: Vec<device::Value>,
}

impl TryFrom<DriverConfig> for Params {
    type Error = Error;

    fn try_from(cfg: DriverConfig) -> std::result::Result<Self, Self::Error> {
        cfg.parse_into()
    }
}

fn cfg_eab_default() -> bool {
    false
}

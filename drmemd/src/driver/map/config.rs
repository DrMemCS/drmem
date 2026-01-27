use drmem_api::{device::Value, driver::DriverConfig, Error, Result};

#[derive(PartialEq, serde::Deserialize, Debug)]
pub struct Entry {
    pub start: i32,
    pub end: Option<i32>,
    pub value: Value,
}

#[derive(serde::Deserialize)]
pub struct Params {
    pub initial: Option<i32>,
    pub default: Value,
    pub values: Vec<Entry>,
}

impl TryFrom<DriverConfig> for Params {
    type Error = Error;

    fn try_from(cfg: DriverConfig) -> std::result::Result<Self, Self::Error> {
        parse(&cfg)
    }
}

// Convert the TOML Table into `InstanceConfig`.

pub fn parse(cfg: &DriverConfig) -> Result<Params> {
    let mut cfg: Params = cfg.parse_into()?;

    // Sort the entries by the value of the start index.

    cfg.values.sort_by(|a, b| a.start.cmp(&b.start));

    // Now check to see if any ranges overlap. If so, that's an
    // error.

    if cfg
        .values
        .windows(2)
        .any(|e| e[0].end.unwrap_or(e[0].start) >= e[1].start)
    {
        return Err(Error::ConfigError(
            "`values` array contains overlapping ranges".into(),
        ));
    }
    Ok(cfg)
}

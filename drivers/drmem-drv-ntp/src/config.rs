use drmem_api::{driver::DriverConfig, Error};
use std::net::SocketAddrV4;

#[derive(serde::Deserialize)]
pub struct Params {
    pub addr: SocketAddrV4,
}

impl TryFrom<DriverConfig> for Params {
    type Error = Error;

    fn try_from(cfg: DriverConfig) -> std::result::Result<Self, Self::Error> {
        cfg.parse_into()
    }
}

#[cfg(test)]
mod tests {
    use super::Params;
    use drmem_api::{driver::DriverConfig, Error, Result};
    use std::net::SocketAddrV4;

    // Helper function to build a config from a string view.

    fn mk_cfg(text: &str) -> Result<Params> {
        Into::<DriverConfig>::into(
            toml::from_str::<toml::value::Table>(text)
                .map_err(|e| Error::ConfigError(format!("{}", e)))?,
        )
        .parse_into()
    }

    #[test]
    fn test_config() {
        assert!(mk_cfg("addr = 5").is_err());
        assert!(mk_cfg("addr = true").is_err());
        assert!(mk_cfg("addr = \"hello\"").is_err());

        let cfg = mk_cfg("addr = \"192.168.1.100:50\"").unwrap();

        assert_eq!(cfg.addr, SocketAddrV4::new([192, 168, 1, 100].into(), 50));
    }
}

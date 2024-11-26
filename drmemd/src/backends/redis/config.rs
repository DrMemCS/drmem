use serde_derive::Deserialize;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

#[derive(Deserialize, Clone)]
pub struct Config {
    pub addr: Option<SocketAddr>,
    pub dbn: Option<i64>,
}

impl Config {
    pub const fn new() -> Config {
        Config {
            addr: None,
            dbn: None,
        }
    }

    pub fn get_addr(&self) -> SocketAddr {
        self.addr.unwrap_or_else(|| {
            SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 6379)
        })
    }

    #[cfg(debug_assertions)]
    pub fn get_dbn(&self) -> i64 {
        self.dbn.unwrap_or(1)
    }
    #[cfg(not(debug_assertions))]
    pub fn get_dbn(&self) -> i64 {
        self.dbn.unwrap_or(0)
    }
}

pub static DEF: Config = Config::new();

impl Default for Config {
    fn default() -> Self {
	Self::new()
    }
}

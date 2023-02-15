use serde_derive::Deserialize;
use std::net::{Ipv4Addr, SocketAddr};

fn def_name() -> String {
    String::from("unknown name")
}

fn def_location() -> String {
    String::from("unknown location")
}

fn def_address() -> SocketAddr {
    (Ipv4Addr::new(0, 0, 0, 0), 3000).into()
}

#[derive(Deserialize)]
pub struct Config {
    #[serde(default = "def_name")]
    pub name: String,
    #[serde(default = "def_location")]
    pub location: String,
    #[serde(default = "def_address")]
    pub addr: SocketAddr,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            name: def_name(),
            location: def_location(),
            addr: def_address(),
        }
    }
}

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

fn def_pref_port() -> u16 {
    3000
}

#[derive(Deserialize)]
pub struct Config {
    #[serde(default = "def_name")]
    pub name: String,
    #[serde(default = "def_location")]
    pub location: String,
    #[serde(default = "def_address")]
    pub addr: SocketAddr,
    pub pref_host: Option<String>,
    #[serde(default = "def_pref_port")]
    pub pref_port: u16,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            name: def_name(),
            location: def_location(),
            addr: def_address(),
            pref_host: None,
            pref_port: def_pref_port(),
        }
    }
}

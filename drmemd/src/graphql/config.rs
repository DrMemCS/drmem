use serde::Deserialize;
use std::{
    net::{Ipv4Addr, SocketAddr},
    path::Path,
    sync::Arc,
};

fn def_name() -> String {
    "unknown name".into()
}

fn def_location() -> Arc<str> {
    "unknown location".into()
}

fn def_address() -> SocketAddr {
    (Ipv4Addr::new(0, 0, 0, 0), 3000).into()
}

fn def_pref_port() -> u16 {
    3000
}

#[derive(Deserialize)]
pub struct Security {
    pub clients: Arc<[String]>,
    pub cert_file: Arc<Path>,
    pub key_file: Arc<Path>,
}

#[derive(Deserialize)]
pub struct Config {
    #[serde(default = "def_name")]
    pub name: String,
    #[serde(default = "def_location")]
    pub location: Arc<str>,
    #[serde(default = "def_address")]
    pub addr: SocketAddr,
    pub pref_host: Option<Arc<str>>,
    #[serde(default = "def_pref_port")]
    pub pref_port: u16,
    pub security: Option<Security>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            name: def_name(),
            location: def_location(),
            addr: def_address(),
            pref_host: None,
            pref_port: def_pref_port(),
            security: None,
        }
    }
}

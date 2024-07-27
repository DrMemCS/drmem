use serde_derive::Deserialize;

#[derive(Deserialize, Clone)]
pub struct Config {}

impl Config {
    pub const fn new() -> Config {
        Config {}
    }
}

pub static DEF: Config = Config::new();

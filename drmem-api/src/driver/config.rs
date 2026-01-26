/// Represents how configuration information is given to a driver.
/// Since each driver can have vastly different requirements, the
/// config structure needs to be as general as possible. A
/// `DriverConfig` type is a map with `String` keys and `toml::Value`
/// values.
use crate::{types::Error, Result};
use serde::de::DeserializeOwned;
use std::ops::Deref;
use toml::value::{Table, Value};

#[derive(Clone, Debug, Default)]
pub struct DriverConfig(Table);

impl DriverConfig {
    /// Return a reference to the underlying toml::Value for a key.
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.0.get(key)
    }

    pub fn get_str(&self, key: &str) -> Result<String> {
        match self.0.get(key) {
            Some(Value::String(s)) => Ok(s.clone()),
            Some(_) => {
                Err(Error::ConfigError(format!("'{}' must be a string", key)))
            }
            None => Err(Error::ConfigError(format!(
                "missing {} config paramater",
                key
            ))),
        }
    }

    pub fn parse_into<T>(&self) -> Result<T>
    where
        T: DeserializeOwned,
    {
        Value::Table(self.0.clone()).try_into().map_err(|e| {
            Error::ConfigError(format!("config parse error: {}", e))
        })
    }
}

impl From<Table> for DriverConfig {
    fn from(t: Table) -> Self {
        DriverConfig(t)
    }
}

impl From<DriverConfig> for Table {
    fn from(dc: DriverConfig) -> Self {
        dc.0
    }
}

impl std::convert::TryFrom<DriverConfig> for () {
    type Error = std::convert::Infallible;

    fn try_from(_cfg: DriverConfig) -> std::result::Result<Self, Self::Error> {
        Ok(())
    }
}

impl Deref for DriverConfig {
    type Target = Table;

    fn deref(&self) -> &Table {
        &self.0
    }
}

use crate::Result;
use drmem_types::{device::Value, Error};
use std::{collections::HashMap, marker::PhantomData};

/// A `Device` type provides a view into the database for a single
/// device. It caches meta information and standardizes fields for
/// devices, as well.

type DeviceInfo = HashMap<&'static str, Value>;

pub struct Device<T>(String, DeviceInfo, PhantomData<T>);

impl<T: Into<Value> + Send> Device<T> {
    // Define constant string slices that will be shared by every device's
    // HashMap.

    const KEY_SUMMARY: &'static str = "summary";
    const KEY_UNITS: &'static str = "units";

    /// Creates a new instance of a `Device`. `summary` is a one-line
    /// summary of the device. If the value returned by the device is
    /// in engineering units, then they can be specified with `units`.

    pub fn create(name: &str, summary: String, units: Option<String>) -> Self {
        let mut map = HashMap::new();

        map.insert(Device::<T>::KEY_SUMMARY, Value::Str(summary));

        if let Some(u) = units {
            map.insert(Device::<T>::KEY_UNITS, Value::Str(u));
        }
        Device(String::from(name), map, PhantomData)
    }

    /// Creates a `Device` type from a hash map of key/values
    /// (presumably obtained from redis.) The fields are checked for
    /// proper structure.

    pub fn create_from_map(
        name: &str, map: HashMap<String, Value>,
    ) -> Result<Self> {
        let mut result = DeviceInfo::new();

        // Verify a 'summary' field exists and is a string. The
        // summary field is recommended to be a single line of text,
        // but this code doesn't enforce it.

        match map.get(Device::<T>::KEY_SUMMARY) {
            Some(Value::Str(val)) => {
                let _ = result
                    .insert(Device::<T>::KEY_SUMMARY, Value::Str(val.clone()));
            }
            Some(_) => return Err(Error::TypeError),
            None => return Err(Error::NotFound),
        }

        // Verify there is no "units" field or, if it exists, it's a
        // string value.

        match map.get(Device::<T>::KEY_UNITS) {
            Some(Value::Str(val)) => {
                let _ = result
                    .insert(Device::<T>::KEY_UNITS, Value::Str(val.clone()));
            }
            Some(_) => return Err(Error::TypeError),
            None => (),
        }

        Ok(Device(String::from(name), result, PhantomData))
    }

    /// Returns a vector of pairs where each pair consists of a key
    /// and its associated value in the map.

    pub fn to_vec(&self) -> Vec<(&'static str, Value)> {
        self.1.iter().map(|(k, v)| (*k, v.clone())).collect()
    }

    pub fn set(&self, v: T) -> (String, Value) {
        (self.0.clone(), v.into())
    }
}

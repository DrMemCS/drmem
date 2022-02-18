// Copyright (c) 2022, Richard M Neswold, Jr.
// All rights reserved.
//
// Redistribution and use in source and binary forms, with or without
// modification, are permitted provided that the following conditions are
// met:
//
// 1. Redistributions of source code must retain the above copyright
//    notice, this list of conditions and the following disclaimer.
//
// 2. Redistributions in binary form must reproduce the above copyright
//    notice, this list of conditions and the following disclaimer in the
//    documentation and/or other materials provided with the distribution.
//
// 3. Neither the name of the copyright holder nor the names of its
//    contributors may be used to endorse or promote products derived
//    from this software without specific prior written permission.
//
// THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS
// "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT
// LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR
// A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT
// HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
// SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT
// LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE,
// DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY
// THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
// (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
// OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use crate::{
    types::{DeviceValue, DrMemError},
    Result,
};
use std::{collections::HashMap, marker::PhantomData};

/// A `Device` type provides a view into the database for a single
/// device. It caches meta information and standardizes fields for
/// devices, as well.

type DeviceInfo = HashMap<&'static str, DeviceValue>;

pub struct Device<T>(String, DeviceInfo, PhantomData<T>);

impl<T: Into<DeviceValue> + Send> Device<T> {
    // Define constant string slices that will be shared by every device's
    // HashMap.

    const KEY_SUMMARY: &'static str = "summary";
    const KEY_UNITS: &'static str = "units";

    /// Creates a new instance of a `Device`. `summary` is a one-line
    /// summary of the device. If the value returned by the device is
    /// in engineering units, then they can be specified with `units`.

    pub fn create(name: &str, summary: String, units: Option<String>) -> Self {
        let mut map = HashMap::new();

        map.insert(Device::<T>::KEY_SUMMARY, DeviceValue::Str(summary));

        if let Some(u) = units {
            map.insert(Device::<T>::KEY_UNITS, DeviceValue::Str(u));
        }
        Device(String::from(name), map, PhantomData)
    }

    /// Creates a `Device` type from a hash map of key/values
    /// (presumably obtained from redis.) The fields are checked for
    /// proper structure.

    pub fn create_from_map(
        name: &str, map: HashMap<String, DeviceValue>,
    ) -> Result<Self> {
        let mut result = DeviceInfo::new();

        // Verify a 'summary' field exists and is a string. The
        // summary field is recommended to be a single line of text,
        // but this code doesn't enforce it.

        match map.get(Device::<T>::KEY_SUMMARY) {
            Some(DeviceValue::Str(val)) => {
                let _ = result.insert(
                    Device::<T>::KEY_SUMMARY,
                    DeviceValue::Str(val.clone()),
                );
            }
            Some(_) => return Err(DrMemError::TypeError),
            None => return Err(DrMemError::NotFound),
        }

        // Verify there is no "units" field or, if it exists, it's a
        // string value.

        match map.get(Device::<T>::KEY_UNITS) {
            Some(DeviceValue::Str(val)) => {
                let _ = result.insert(
                    Device::<T>::KEY_UNITS,
                    DeviceValue::Str(val.clone()),
                );
            }
            Some(_) => return Err(DrMemError::TypeError),
            None => (),
        }

        Ok(Device(String::from(name), result, PhantomData))
    }

    /// Returns a vector of pairs where each pair consists of a key
    /// and its associated value in the map.

    pub fn to_vec(&self) -> Vec<(&'static str, DeviceValue)> {
        self.1.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
    }

    pub fn set(&self, v: T) -> (String, DeviceValue) {
        (self.0.clone(), v.into())
    }
}

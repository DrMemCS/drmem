// Copyright (c) 2021, Richard M Neswold, Jr.
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

use std::collections::HashMap;
use redis::*;

pub mod data;
pub mod db;

use data::Type;

// Define constant string slices that will be (hopefully) shared by
// every device's HashMap.

const KEY_SUMMARY: &'static str = "summary";
const KEY_UNITS: &'static str = "units";

/// A `Device` type provides a view into the database for a single
/// device. It caches meta information and standardizes fields for
/// devices, as well.

type DeviceInfo = HashMap<String, Type>;

pub struct Device(DeviceInfo);

impl Device {
    /// Creates a new instance of a `Device`. `summary` is a one-line
    /// summary of the device. If the value returned by the device is
    /// in engineering units, then they can be specified with `units`.

    pub fn create(summary: String, units: Option<String>) -> Device {
	let mut map = HashMap::new();

	map.insert(String::from(KEY_SUMMARY), Type::Str(summary));

	if let Some(u) = units {
	    map.insert(String::from(KEY_UNITS), Type::Str(u));
	}
	Device(map)
    }

    /// Creates a `Device` type from a hash map of key/values
    /// (presumably obtained from redis.) The fields are checked for
    /// proper structure.

    pub fn create_from_map(map: HashMap<String, Type>)
			   -> redis::RedisResult<Device> {
	let mut result = DeviceInfo::new();

	// Verify a 'summary' field exists and is a string. The
	// summary field is recommended to be a single line of text,
	// but this code doesn't enforce it.

	match map.get(KEY_SUMMARY) {
	    Some(Type::Str(val)) => {
		let _ =
		    result.insert(String::from(KEY_SUMMARY),
				  Type::Str(val.clone()));
	    }
	    Some(_) =>
		return Err(RedisError::from((ErrorKind::TypeError,
					     "'summary' field isn't a string"))),
	    None =>
		return Err(RedisError::from((ErrorKind::TypeError,
					     "'summary' is missing")))
	}

	// Verify there is no "units" field or, if it exists, it's a
	// string value.

	match map.get(KEY_UNITS) {
	    Some(Type::Str(val)) => {
		let _ =
		    result.insert(String::from(KEY_UNITS),
				  Type::Str(val.clone()));
	    }
	    Some(_) =>
		return Err(RedisError::from((ErrorKind::TypeError,
					     "'units' field isn't a string"))),
	    None => ()
	}

	Ok(Device(result))
    }

    /// Returns a vector of pairs where each pair consists of a key
    /// and its associated value in the map.

    pub fn to_vec(&self) -> Vec<(String, Type)> {
	let mut result: Vec<(String, Type)> = vec![];

	for (k, v) in self.0.iter() {
	    result.push((String::from(k), v.clone()))
	}
	result
    }

}

pub struct DeviceContext(String);

impl DeviceContext {

    pub fn new(name: String) -> DeviceContext {
	DeviceContext(name)
    }

    pub fn set(&self, v: Type) -> (String, Type) {
	(self.0.clone(), v)
    }

}

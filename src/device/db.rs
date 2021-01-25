// Copyright (c) 2020-2021, Richard M Neswold, Jr.
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
use tracing::{ debug, info, warn };
use redis::*;

use super::{ Device, DeviceContext, Type };
use crate::config::Config;

type DevMap = HashMap<String, Device>;

/// Defines a driver "context" which is used to communicate with the
/// `redis` database.
pub struct Context {

    /// The base name used by the instance of the driver. Defining
    /// `Device` instances will add the last segment to the name.
    base: String,

    /// This connection is used for interacting with the database.
    db_con: redis::aio::Connection,

    /// A map which maps keys to devices. The key to the map becomes
    /// the last segment of the device name. It recommended that the
    /// key only contains alphanumeric characters and dashes
    /// (specifically, adding a colon will be confusing since the
    /// final segment should refer to a specific device in a driver
    /// and the path refers to an instance of a driver.)
    devices: DevMap
}

impl<'a> Context {

    // Creates a connection to redis.

    async fn make_connection(cfg: &Config,
			     name: Option<String>,
			     pword: Option<String>)
			     -> redis::RedisResult<redis::aio::Connection> {
	// Create connection information needed to access `redis`.

	let addr = redis::ConnectionAddr::Tcp(cfg.redis.addr.clone(),
					      cfg.redis.port);
	let info = redis::ConnectionInfo { addr: Box::new(addr),
					   db: cfg.redis.dbn,
					   username: name,
					   passwd: pword };

	// Connect to redis and return the Connection.

	debug!("connecting to redis -- addr: {:?}, db#: {}, and account: {:?}",
	       &info.addr, &info.db, &info.username);
	redis::aio::connect_tokio(&info).await
    }

    /// Builds a new driver context which can be used to interact with
    /// `redis`. The parameters in `cfg` will be used to locate the
    /// `redis` instance. If `name` and `pword` are not `None`, they
    /// will be used for credentials when connecting to `redis`.

    pub async fn create(base_name: &str,
			cfg: &Config,
			name: Option<String>,
			pword: Option<String>) -> redis::RedisResult<Self> {
	let db_con = Context::make_connection(cfg, name, pword).await?;

	Ok(Context { base: String::from(base_name),
		     db_con,
		     devices: DevMap::new() })
    }

    // Returns the key that returns meta information for the device.

    fn info_key(&self, name: &str) -> String {
	format!("{}:{}#info", &self.base, &name)
    }

    // Returns the key that returns time-series information for the
    // device.

    fn history_key(&self, name: &str) -> String {
	format!("{}:{}#hist", &self.base, &name)
    }

    // Returns the key that is used to communicate settings to a
    // device.

    fn setting_key(&self, name: &str) -> String {
	format!("{}:{}#set", &self.base, &name)
    }

    // Does some sanity checks on a device to see if it appears to be
    // valid.

    async fn get_device(&mut self, info_key: &str)
			-> redis::RedisResult<Device> {
	let data_type: String = redis::cmd("TYPE").arg(info_key)
	    .query_async(&mut self.db_con).await?;

	// If the info key is a "hash" type, we assume the device has
	// been created and maintained properly.

	match data_type.as_str() {
	    "hash" => {
		let result: HashMap<String, Type> =
		    redis::Cmd::hgetall(info_key)
		    .query_async(&mut self.db_con)
		    .await?;

		Device::create_from_map(result)
	    },
	    "none" =>
		Err(RedisError::from((ErrorKind::TypeError,
				      "device doesn't exist"))),
	    _ =>
		Err(RedisError::from((ErrorKind::TypeError,
				      "wrong type associated with key"))),
	}
    }

    /// Used by a driver to define a readable device. `name` specifies
    /// the final segment of the device name (the prefix is determined
    /// by the driver's name.) `summary` should be a one-line
    /// description of the device. `units` is an optional units
    /// field. Some devices (like boolean or string devices) don't
    /// require engineering units.
    pub async fn define_device(&'a mut self,
			       name: &'a str,
			       summary: &str,
			       units: Option<String>)
			       -> redis::RedisResult<DeviceContext> {
	let dev_name = format!("{}:{}", &self.base, &name);
	let info_key = self.info_key(&name);
	let hist_key = self.history_key(&name);

	debug!("defining '{}'", &dev_name);

	let result = match self.get_device(&info_key).await {
	    Ok(v) => v,
	    Err(e) => {
		warn!("'{}' isn't defined properly -- {:?}", &dev_name, e);

		let dev = Device::create(String::from(summary), units);

		// Create a command pipeline that deletes the two keys
		// and then creates them properly with default values.

		let _: () = redis::pipe()
		    .atomic()
		    .del(&hist_key)
		    .xadd(&hist_key, 1, &[("value", 0)])
		    .xdel(&hist_key, &[1])
		    .del(&info_key)
		    .hset_multiple(&info_key, &dev.to_vec())
		    .query_async(&mut self.db_con).await?;

		info!("'{}' has been successfully created", &dev_name);
		dev
	    }
	};

	let _ = self.devices.insert(dev_name.clone(), result);

	Ok(DeviceContext::new(String::from(name)))
    }

    fn to_stamp(val: Option<u64>) -> String {
	match val {
	    Some(v) => format!("{}", v),
	    None => String::from("*")
	}
    }

    /// Allows a driver to write values, associated with devices, to
    /// the database. `stamp` is the timestamp associated with every
    /// entry in the `values` array. With each call to this method,
    /// the timestamp must be always increasing. Trying to insert
    /// values with a timestamp earlier than the last timestamp of the
    /// device in the database will result in an error. The `values`
    /// array indicate which devices should be updated.
    ///
    /// If multiple devices change simultaneously (e.g. a device's
    /// value is computed from other devices), a driver is strongly
    /// recommended to make a single call with all the affected
    /// devices. Each call to this function makes an atomic change to
    /// the database so if all devices are changed in a single call,
    /// clients will see a consistent change.
    pub async fn write_values(&mut self,
			      stamp: Option<u64>,
			      values: &[(String, Type)])
			      -> redis::RedisResult<()> {
	let stamp = Context::to_stamp(stamp);
	let mut pipe = redis::pipe();
	let mut cmd = pipe.atomic();

	for (dev, val) in values {
	    let key = self.history_key(&dev);

	    cmd = cmd.xadd(key, &stamp, &[("value", val.to_redis_args())]);

	    // TODO: need to check alarm limits -- and add the command
	    // to announce it -- as the command is built-up.
	}

	let _: () = cmd.query_async(&mut self.db_con).await?;

	Ok(())
    }
}

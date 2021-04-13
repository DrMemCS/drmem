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
use std::convert::TryInto;
use async_trait::async_trait;
use tracing::{ debug, info, warn };
use drmem_api::{ DbContext, Result, device::Device,
		 types::{ Compat, DeviceValue, Error, ErrorKind } };

// Translates a Redis error into a DrMem error.

fn xlat_result<T>(res: redis::RedisResult<T>) -> Result<T> {
    match res {
	Ok(res) => Ok(res),
	Err(err) => {
	    let msg = String::from(err
				   .detail()
				   .unwrap_or("no further information"));

	    match err.kind() {
		redis::ErrorKind::ResponseError =>
		    Err(Error(ErrorKind::DbCommunicationError, msg)),
		redis::ErrorKind::AuthenticationFailed =>
		    Err(Error(ErrorKind::AuthenticationError, msg)),
		redis::ErrorKind::TypeError =>
		    Err(Error(ErrorKind::TypeError, msg)),
		redis::ErrorKind::ExecAbortError =>
		    Err(Error(ErrorKind::OperationError, msg)),
		redis::ErrorKind::BusyLoadingError =>
		    Err(Error(ErrorKind::OperationError, msg)),
		redis::ErrorKind::NoScriptError =>
		    Err(Error(ErrorKind::NotFound, msg)),
		redis::ErrorKind::InvalidClientConfig =>
		    Err(Error(ErrorKind::AuthenticationError, msg)),
		redis::ErrorKind::Moved =>
		    Err(Error(ErrorKind::NotFound, msg)),
		redis::ErrorKind::Ask =>
		    Err(Error(ErrorKind::NotFound, msg)),
		redis::ErrorKind::TryAgain =>
		    Err(Error(ErrorKind::OperationError, msg)),
		redis::ErrorKind::ClusterDown =>
		    Err(Error(ErrorKind::DbCommunicationError, msg)),
		redis::ErrorKind::CrossSlot =>
		    Err(Error(ErrorKind::DbCommunicationError, msg)),
		redis::ErrorKind::MasterDown =>
		    Err(Error(ErrorKind::DbCommunicationError, msg)),
		redis::ErrorKind::IoError =>
		    Err(Error(ErrorKind::DbCommunicationError, msg)),
		redis::ErrorKind::ClientError =>
		    Err(Error(ErrorKind::OperationError, msg)),
		redis::ErrorKind::ExtensionError =>
		    Err(Error(ErrorKind::OperationError, msg)),
	    }
	}
    }
}

// Encodes a `DeviceValue` into a binary which gets stored in
// redis. This encoding lets us store type information in redis so
// there's no rounding errors or misinterpretation of the data.

fn to_redis<T: Compat>(val: T) -> Vec<u8> {
    match val.to_type() {
	DeviceValue::Nil => vec![],
	DeviceValue::Bool(false) => vec!['F' as u8],
	DeviceValue::Bool(true) => vec!['T' as u8],

	DeviceValue::Int(v) => {
	    let mut buf: Vec<u8> = Vec::with_capacity(9);

	    buf.push('I' as u8);
	    buf.extend_from_slice(&v.to_be_bytes());
	    buf
	},

	DeviceValue::Flt(v) => {
	    let mut buf: Vec<u8> = Vec::with_capacity(9);

	    buf.push('D' as u8);
	    buf.extend_from_slice(&v.to_be_bytes());
	    buf
	},

	DeviceValue::Str(s) => {
	    let s = s.as_bytes();
	    let mut buf: Vec<u8> = Vec::with_capacity(5 + s.len());

	    buf.push('S' as u8);
	    buf.extend_from_slice(&(s.len() as u32).to_be_bytes());
	    buf.extend_from_slice(&s);
	    buf
	}
    }
}

fn decode_integer(buf: &[u8]) -> Result<DeviceValue> {
    if buf.len() >= 8 {
	let buf = buf[..8].try_into().unwrap();

	return Ok(DeviceValue::Int(i64::from_be_bytes(buf)))
    }
    Err(Error(ErrorKind::TypeError,
	      String::from("buffer too short for integer data")))
}

fn decode_float(buf: &[u8]) -> Result<DeviceValue> {
    if buf.len() >= 8 {
	let buf = buf[..8].try_into().unwrap();

	return Ok(DeviceValue::Flt(f64::from_be_bytes(buf)))
    }
    Err(Error(ErrorKind::TypeError,
	      String::from("buffer too short for floating point data")))
}

fn decode_string(buf: &[u8]) -> Result<DeviceValue> {
    if buf.len() >= 4 {
	let len_buf = buf[..4].try_into().unwrap();
	let len = u32::from_be_bytes(len_buf) as usize;

	if buf.len() >= (4 + len) as usize {
	    let str_vec = buf[4..4 + len].to_vec();

	    return match String::from_utf8(str_vec) {
		Ok(s) => Ok(DeviceValue::Str(s)),
		Err(_) => Err(Error(ErrorKind::TypeError,
				    String::from("string not UTF-8")))
	    }
	}
    }
    Err(Error(ErrorKind::TypeError,
	      String::from("buffer too short for string data")))
}

fn from_value(v: &redis::Value) -> Result<DeviceValue>
{
    if let redis::Value::Data(buf) = v {

	// The buffer has to have at least one character in order to
	// be decoded.

	if buf.len() > 0 {
	    match buf[0] as char {
		'F' => Ok(DeviceValue::Bool(false)),
		'T' => Ok(DeviceValue::Bool(true)),
		'I' => decode_integer(&buf[1..]),
		'D' => decode_float(&buf[1..]),
		'S' => decode_string(&buf[1..]),

		// Any other character in the tag field is unknown and
		// can't be decoded as a `DeviceValue`.

		_ =>
		    Err(Error(ErrorKind::TypeError,
			      String::from("unknown tag")))
	    }
	} else {
	    Ok(DeviceValue::Nil)
	}
    } else {
	Err(Error(ErrorKind::TypeError, String::from("bad redis::Value")))
    }
}

/// Defines a context that uses redis for the back-end storage.
pub struct RedisContext {
    /// The base name used by the instance of the driver. Defining
    /// `Device` instances will add the last segment to the name.
    base: String,

    /// This connection is used for interacting with the database.
    db_con: redis::aio::Connection,
}

impl RedisContext {

    // Creates a connection to redis.

    async fn make_connection(cfg: &drmem_config::backend::Config,
			     name: Option<String>,
			     pword: Option<String>)
			     -> Result<redis::aio::Connection> {
	// Create connection information needed to access `redis`.

	let addr = redis::ConnectionAddr::Tcp(String::from(cfg.get_addr()),
					      cfg.get_port());
	let info = redis::ConnectionInfo { addr: Box::new(addr),
					   db: cfg.get_dbn(),
					   username: name,
					   passwd: pword };

	// Connect to redis and return the Connection.

	debug!("connecting to redis -- addr: {:?}, db#: {}, and account: {:?}",
	       &info.addr, &info.db, &info.username);
	xlat_result(redis::aio::connect_tokio(&info).await)
    }

    /// Builds a new backend context which can interacts with `redis`.
    /// The parameters in `cfg` will be used to locate the `redis`
    /// instance. If `name` and `pword` are not `None`, they will be used
    /// for credentials when connecting to `redis`.

    pub async fn new(base_name: &str, cfg: &drmem_config::backend::Config,
		     name: Option<String>, pword: Option<String>) -> Result<Self> {
	let db_con = RedisContext::make_connection(cfg, name, pword).await?;

	Ok(RedisContext { base: String::from(base_name), db_con })
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

    // Does some sanity checks on a device to see if it appears to be
    // valid.

    async fn get_device<T: Compat + Send>(&mut self, name: &str)
					  -> Result<Device<T>> {
	let info_key = self.info_key(name);
	let data_type: String =
	    xlat_result(redis::cmd("TYPE")
			.arg(&info_key)
			.query_async(&mut self.db_con)
			.await)?;

	// If the info key is a "hash" type, we assume the device has
	// been created and maintained properly.

	match data_type.as_str() {
	    "hash" => {
		let mut result: HashMap<String, redis::Value> =
		    xlat_result(redis::Cmd::hgetall(&info_key)
				.query_async(&mut self.db_con)
				.await)?;

		// Convert the HaspMap<String, redis::Value> into a
		// HashMap<String, DeviceValue>. As it converts each
		// entry, it checks to see if the associated
		// redis::Value can be translated. If not, it is
		// ignored.

		let fields = result
		    .drain()
		    .filter_map(|(k, v)|
				if let Ok(v) = from_value(&v) {
				    Some((k, v))
				} else {
				    None
				})
		    .collect();

		Device::create_from_map(name, fields)
	    },
	    "none" =>
		Err(Error(ErrorKind::NotFound,
			  String::from("device doesn't exist"))),
	    _ =>
		Err(Error(ErrorKind::NotFound,
			  String::from("wrong type associated with key"))),
	}
    }
}

#[async_trait]
impl DbContext for RedisContext {
    async fn define_device<T: Compat + Send>(&mut self,
					     name: &str,
					     summary: &str,
					     units: Option<String>) ->
	Result<Device<T>>
    {
	let dev_name = format!("{}:{}", &self.base, &name);

	debug!("defining '{}'", &dev_name);

	match self.get_device::<T>(&name).await {
	    Ok(v) =>
		Ok(v),
	    Err(e) => {
		warn!("'{}' isn't defined properly -- {:?}", &dev_name, e);

		let hist_key = self.history_key(&name);
		let info_key = self.info_key(&name);
		let dev = Device::create(name, String::from(summary), units);

		let temp = dev.to_vec();
		let fields: Vec<(&String, Vec<u8>)> =
		    temp
		    .iter()
		    .map(|(k, v)| (k, to_redis(v)))
		    .collect();

		// Create a command pipeline that deletes the two keys
		// and then creates them properly with default values.

		let _: () = xlat_result(redis::pipe()
					.atomic()
					.del(&hist_key)
					.xadd(&hist_key, "1",
					      &[("value", to_redis(0))])
					.xdel(&hist_key, &["1"])
					.del(&info_key)
					.hset_multiple(&info_key, &fields)
					.query_async(&mut self.db_con).await)?;

		info!("'{}' has been successfully created", &dev_name);
		Ok(dev)
	    }
	}
    }

    async fn write_values(&mut self, values: &[(String, DeviceValue)])
			  -> Result<()> {
	let mut pipe = redis::pipe();
	let mut cmd = pipe.atomic();

	for (dev, val) in values {
	    let key = self.history_key(&dev);

	    cmd = cmd.xadd(key, "*", &[("value", to_redis(val))]);

	    // TODO: need to check alarm limits -- and add the command
	    // to announce it -- as the command is built-up.
	}

	xlat_result(cmd.query_async(&mut self.db_con).await)
    }
}

// This section holds code used for testing the module. The
// "#[cfg(test)]" attribute means the module will only be compiled and
// included in the test executable; debug and release versions won't
// have the code.

#[cfg(test)]
mod tests {
    use redis::{ Value };
    use super::*;

    // We only want to convert Value::Data() forms. These tests make
    // sure the other variants don't translate.

    #[tokio::test]
    async fn test_reject_invalid_forms() {
	if let Ok(v) = from_value(&Value::Nil) {
	    panic!("Value::Nil incorrectly translated to {:?}", v);
	}
	if let Ok(v) = from_value(&Value::Int(0)) {
	    panic!("Value::Int incorrectly translated to {:?}", v);
	}
	if let Ok(v) = from_value(&Value::Bulk(vec![])) {
	    panic!("Value::Bulk incorrectly translated to {:?}", v);
	}
	if let Ok(v) = from_value(&Value::Status(String::from(""))) {
	    panic!("Value::Status incorrectly translated to {:?}", v);
	}
	if let Ok(v) = from_value(&Value::Okay) {
	    panic!("Value::Okay incorrectly translated to {:?}", v);
	}
    }

    // Test correct DeviceValue::Nil decoding.

    #[tokio::test]
    async fn test_nil_decoder() {
	assert_eq!(Ok(DeviceValue::Nil), from_value(&Value::Data(vec![])));
    }

    // Test correct DeviceValue::Bool decoding.

    #[tokio::test]
    async fn test_bool_decoder() {
	assert_eq!(Ok(DeviceValue::Bool(false)),
		   from_value(&Value::Data(vec!['F' as u8])));
	assert_eq!(Ok(DeviceValue::Bool(true)),
		   from_value(&Value::Data(vec!['T' as u8])));
    }

    // Test correct DeviceValue::Int decoding.

    #[tokio::test]
    async fn test_int_decoder() {
	let values: Vec<(i64, Vec<u8>)> = vec![
	    (0, vec!['I' as u8,
		     0x00u8, 0x00u8, 0x00u8, 0x00u8,
		     0x00u8, 0x00u8, 0x00u8, 0x00u8]),
	    (1, vec!['I' as u8,
		     0x00u8, 0x00u8, 0x00u8, 0x00u8,
		     0x00u8, 0x00u8, 0x00u8, 0x01u8]),
	    (-1, vec!['I' as u8,
		      0xffu8, 0xffu8, 0xffu8, 0xffu8,
		      0xffu8, 0xffu8, 0xffu8, 0xffu8]),
	    (0x7fffffffffffffff, vec!['I' as u8,
				      0x7fu8, 0xffu8, 0xffu8, 0xffu8,
				      0xffu8, 0xffu8, 0xffu8, 0xffu8]),
	    (-0x8000000000000000, vec!['I' as u8,
				       0x80u8, 0x00u8, 0x00u8, 0x00u8,
				       0x00u8, 0x00u8, 0x00u8, 0x00u8]),
	    (0x0123456789abcdef, vec!['I' as u8,
				      0x01u8, 0x23u8, 0x45u8, 0x67u8,
				      0x89u8, 0xabu8, 0xcdu8, 0xefu8]),
	];

	for (v, rv) in values.iter() {
	    let data = Value::Data(rv.to_vec());

	    assert_eq!(Ok(DeviceValue::Int(*v)), from_value(&data));
	}
    }

    #[tokio::test]
    async fn test_nil_encoder() {
	assert_eq!(Vec::<u8>::new(), to_redis(DeviceValue::Nil));
    }

    #[tokio::test]
    async fn test_bool_encoder() {
	assert_eq!(vec![ 'F' as u8], to_redis(DeviceValue::Bool(false)));
	assert_eq!(vec![ 'T' as u8], to_redis(DeviceValue::Bool(true)));
    }

    #[tokio::test]
    async fn test_int_encoder() {
	let values: Vec<(i64, Vec<u8>)> = vec![
	    (0, vec![ 'I' as u8, 0x00, 0x00, 0x00, 0x00,
		       0x00, 0x00, 0x00, 0x00 ]),
	    (1, vec![ 'I' as u8, 0x00, 0x00, 0x00, 0x00,
		       0x00, 0x00, 0x00, 0x01 ]),
	    (-1, vec![ 'I' as u8, 0xff, 0xff, 0xff, 0xff,
			0xff, 0xff, 0xff, 0xff ]),
	    (0x7fffffffffffffff,
	     vec![ 'I' as u8, 0x7f, 0xff, 0xff, 0xff,
		    0xff, 0xff, 0xff, 0xff ]),
	    (-0x8000000000000000,
	     vec![ 'I' as u8, 0x80, 0x00, 0x00, 0x00,
		    0x00, 0x00, 0x00, 0x00 ]),
	    (0x0123456789abcdef,
	     vec![ 'I' as u8, 0x01, 0x23, 0x45, 0x67,
		    0x89, 0xab, 0xcd, 0xef ]),
	];

	for (v, rv) in values.iter() {
	    assert_eq!(*rv, to_redis(DeviceValue::Int(*v)));
	}
    }
}

use std::collections::HashMap;
use tracing::{debug, info, warn};
use redis::*;
use crate::data;
use crate::config::Config;

// Define constant string slices that will be (hopefully) shared by
// every device HashMap.

const KEY_SUMMARY: &'static str = "summary";
const KEY_UNITS: &'static str = "units";

/// A `Device` type provides a view into the database for a single
/// device. It caches meta information and standardizes fields for
/// devices, as well.

pub struct Device (HashMap<&'static str, data::Type>);

impl Device {
    /// Creates a new instance of a `Device`. `summary` is a one-line
    /// summary of the device. If the value returned by the device is
    /// in engineering units, then they can be specified with `units`.

    pub fn create(summary: String, units: Option<String>) -> Device {
	let mut map = HashMap::new();

	map.insert(KEY_SUMMARY, data::Type::Str(summary));

	if let Some(u) = units {
	    map.insert(KEY_UNITS, data::Type::Str(u));
	}
	Device(map)
    }

    /// Creates a `Device` type from a hash map of key/values
    /// (presumably obtained from redis.) The fields are checked for
    /// proper structure.

    pub fn create_from_map(map: HashMap<String, data::Type>)
			   -> redis::RedisResult<Device> {
	let mut result = HashMap::<&'static str, data::Type>::new();

	// Verify a 'summary' field exists and is a string. The
	// summary field is recommended to be a single line of text,
	// but this code doesn't enforce it.

	match map.get(KEY_SUMMARY) {
	    Some(data::Type::Str(val)) => {
		let _ =
		    result.insert(KEY_SUMMARY, data::Type::Str(val.clone()));
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
	    Some(data::Type::Str(val)) => {
		let _ =
		    result.insert(KEY_UNITS, data::Type::Str(val.clone()));
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

    pub fn to_vec(&self) -> Vec<(String, data::Type)> {
	let mut result: Vec<(String, data::Type)> = vec![];

	for (k, v) in self.0.iter() {
	    result.push((k.to_string(), v.clone()))
	}
	result
    }
}

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

impl Context {

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

	Ok(Context { base: base_name.to_string(),
		     db_con,
		     devices: DevMap::new() })
    }

    // Generates the keys used to access meta info and historical
    // data. Given a device "foo.bar", the convention is its meta
    // information is stored using the key "foo.bar#info" and its
    // historical data uses "foo.bar#hist".

    fn get_keys(&self, name: &str) -> (String, String) {
	(format!("{}:{}#info", &self.base, &name),
	 format!("{}:{}#hist", &self.base, &name))
    }

    // Does some sanity checks on a device to see if it appears to be
    // valid.

    async fn get_device(&mut self, info_key: &str)
			-> redis::RedisResult<Device> {
	let data_type: String = redis::cmd("TYPE").arg(info_key)
	    .query_async(&mut self.db_con).await?;

	// If the info key is a "hash" type, we assume the device has
	// been created and maintained properly.

	match &data_type[..] {
	    "hash" => {
		let result: HashMap<String, data::Type> =
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
    pub async fn def_device(&mut self,
			    name: &str,
			    summary: &str,
			    units: Option<String>) -> redis::RedisResult<()> {
	// Create the device name and the names of the keys associated
	// with this device.

	let dev_name = format!("{}:{}", &self.base, &name);
	let (info_key, hist_key) = self.get_keys(&name);

	debug!("defining '{}'", &dev_name);

	let result = match self.get_device(&info_key).await {
	    Ok(v) => v,
	    Err(e) => {
		warn!("'{}' isn't defined properly -- {:?}", &dev_name, e);

		let dev = Device::create(summary.to_string(), units);

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

	let _ = self.devices.insert(dev_name, result);

	Ok(())
    }

    fn to_stamp(val: Option<u64>) -> String {
	match val {
	    Some(v) => format!("{}", v),
	    None => "*".to_string()
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
			      values: &[(&str, data::Type)])
			      -> redis::RedisResult<()> {
	let stamp = Context::to_stamp(stamp);
	let mut pipe = redis::pipe();
	let mut cmd = pipe.atomic();

	for (dev, val) in values {
	    let (_, key) = self.get_keys(&dev);

	    cmd = cmd.xadd(key, &stamp, &[("value", val.to_redis_args())]);

	    // TODO: need to check alarm limits -- and add the command
	    // to announce it -- as the command is built-up.
	}

	let _: () = cmd.query_async(&mut self.db_con).await?;

	Ok(())
    }
}

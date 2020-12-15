use std::collections::HashMap;
use tracing::{debug, info, warn};
use redis::*;
use crate::data;
use crate::config::Config;

/// A `Device` provides a narrow interface to the database which
/// allows a driver to interact with a single device.
pub struct Device (HashMap<String, data::Type>);

impl Device {

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
    pub async fn create(base_name: String,
			cfg: &Config,
			name: Option<String>,
			pword: Option<String>) -> redis::RedisResult<Self> {
	let db_con = Context::make_connection(cfg, name, pword).await?;

	Ok(Context { base: base_name, db_con, devices: DevMap::new() })
    }

    // Generates the keys used to access meta info and historical data.

    fn get_keys(&self, name: &str) -> (String, String) {
	(format!("{}#info", &name), format!("{}#hist", &name))
    }

    // Does some sanity checks on a device to see if it appears to be
    // valid.

    async fn get_device(&mut self, info_key: &str)
			-> redis::RedisResult<Option<HashMap<String, data::Type>>> {
	let data_type: String = redis::cmd("TYPE").arg(info_key)
	    .query_async(&mut self.db_con).await?;

	// If the info key is a "hash" type, we assume the device has
	// been created and maintained properly.

	if "hash" == data_type {
	    let result: HashMap<String, data::Type> =
		redis::Cmd::hgetall(info_key)
		.query_async(&mut self.db_con).await?;

	    // Verify 'summary' field exists and is a string.

	    match result.get("summary") {
		Some(data::Type::Str(_)) => (),
		Some(_) => {
		    warn!("'summary' field isn't a string");
		    return Ok(None)
		}
		None => {
		    warn!("'summary' field is missing");
		    return Ok(None)
		}
	    }

	    // Verify there is no "units" field or, if it exists, it's
	    // a string value.

	    match result.get("units") {
		Some(data::Type::Str(_)) => (),
		Some(_) => {
		    warn!("'units' field isn't a string");
		    return Ok(None)
		}
		None => ()
	    }
	    Ok(Some(result))
	} else {
	    Ok(None)
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
			    summary: String,
			    units: Option<String>) -> redis::RedisResult<()> {
	// Create the device name and the names of the keys associated
	// with this device.

	let dev_name = format!("{}:{}", &self.base, &name);
	let (info_key, hist_key) = self.get_keys(&dev_name);

	debug!("defining '{}'", &dev_name);

	let result = match self.get_device(&info_key).await? {
	    Some(v) => v,
	    None => {
		info!("'{}' isn't defined properly ... initializing",
		      &dev_name);

		// 'defaults' holds an array of default field values
		// for the newly created device.

		let mut defaults = vec![("summary", data::Type::Str(summary))];

		if let Some(u) = units {
		    defaults.push(("units", data::Type::Str(u)))
		}

		// Create a command pipeline that deletes the two keys
		// and then creates them properly with default values.

		let _: () = redis::pipe()
		    .atomic()
		    .del(&hist_key)
		    .xadd(&hist_key, 1, &[("value", 0)])
		    .xdel(&hist_key, &[1])
		    .del(&info_key)
		    .hset_multiple(&info_key, &defaults)
		    .query_async(&mut self.db_con).await?;

		let mut map = HashMap::new();

		for (k, v) in defaults {
		    let _ = map.insert(k.to_string(), v);
		}

		map
	    }
	};

	let _ = self.devices.insert(dev_name, Device(result));

	Ok(())
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
	let stamp = if let Some(ts) = stamp {
	    format!("{}", ts)
	} else {
	    "*".to_string()
	};

	let mut pipe = redis::pipe();
	let mut cmd = pipe.atomic();

	for (dev, val) in values {
	    let dev = format!("{}:{}", &self.base, &dev);
	    let (_, key) = self.get_keys(&dev);

	    cmd = cmd.xadd(key, &stamp, &[("value", val.to_redis_args())]);

	    // TODO: need to check alarm limits -- and add the command
	    // to announce it -- as the command is built-up.
	}

	let _: () = cmd.query_async(&mut self.db_con).await?;

	Ok(())
    }
}

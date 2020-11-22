use std::collections::HashMap;
use tracing::{debug, info};
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

    /// This connection is used for pubsub notifications to monitor
    /// key changes. If a key associated with this driver is modified,
    /// it'll get reported on this Connection.
    pubsub_con: redis::aio::Connection,

    /// A map which maps keys to devices.
    devices: DevMap
}

impl Context {

    // This is a utility function to make a connection to redis. It is
    // not part of the public API.

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

	debug!("connecting to redis using {:?}", &info);
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
	let pubsub_con = Context::make_connection(cfg, None, None).await?;

	Ok(Context { base: base_name, db_con, pubsub_con,
		     devices: DevMap::new() })
    }

    fn get_keys(&self, name: &str) -> (String, String) {
	(format!("{}#info", &name), format!("{}#hist", &name))
    }

    pub async fn def_device(&mut self,
			    name: &str,
			    summary: String,
			    units: Option<String>) -> redis::RedisResult<()> {
	// Create the device name and the names of the keys associated
	// with this device.

	let dev_name = format!("{}:{}", &self.base, &name);
	let (info_key, hist_key) = self.get_keys(&dev_name);

	debug!("defining '{}'", &dev_name);

	let data_type: String = redis::cmd("TYPE")
	    .arg(&info_key)
	    .query_async(&mut self.db_con).await?;

	// If the info key is a "hash" type, we assume the device has
	// been created and maintained properly. If it isn't a "hash"
	// type, we need to correct it.

	if "hash" != data_type {
	    info!("'{}' isn't defined ... initializing", &dev_name);

	    // 'defaults' holds an array of default field values for
	    // the newly created device.

	    let mut defaults = vec![("summary", data::Type::Str(summary))];

	    if let Some(u) = units {
		defaults.push(("units", data::Type::Str(u)))
	    }

	    // Create a command pipeline that deletes the two keys and
	    // then creates them properly with default values.

	    let _: () = redis::pipe()
		.atomic()
		.del(&hist_key)
		.xadd(&hist_key, 1, &[("value", 0)])
		.xdel(&hist_key, &[1])
		.del(&info_key)
		.hset_multiple(&info_key, &defaults)
		.query_async(&mut self.db_con).await?;
	}

	// If we reached this point, either the device entries have
	// been created or we found the '#info" key and we're assuming
	// the device exists.

	let result: HashMap<String, data::Type> =
	    redis::Cmd::hgetall(&info_key)
	    .query_async(&mut self.db_con).await?;

	let _ = self.devices.insert(dev_name, Device(result));
	Ok(())
    }
}

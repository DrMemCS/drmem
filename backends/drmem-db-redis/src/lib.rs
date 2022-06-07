use async_trait::async_trait;
use drmem_api::{
    driver::{ReportReading, RxDeviceSetting, TxDeviceSetting},
    types::{device::{Value, Name}, Error},
    Result, Store,
};
use drmem_config::backend;
use futures_util::FutureExt;
use std::collections::HashMap;
use std::convert::TryInto;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

// Translates a Redis error into a DrMem error. The translation is
// slightly lossy in that we lose the exact Redis error that occurred
// and, instead map it into a more general "backend" error. We
// propagate the associated message so, hopefully, that's enough to
// rebuild the context of the error.
//
// This is a job for `impl From<RedisError> for Error`, but it won't
// work here because neither of those types are defined in this
// module. We'd have to put the trait implementation in the
// `drmem-api` crate which, then, requires all projects to build the
// `redis` crate. Since we only need to do the translationin this
// module, this function will be the translater.

fn xlat_err(e: redis::RedisError) -> Error {
    match e.kind() {
        redis::ErrorKind::ResponseError
        | redis::ErrorKind::ClusterDown
        | redis::ErrorKind::CrossSlot
        | redis::ErrorKind::MasterDown
        | redis::ErrorKind::IoError => Error::DbCommunicationError,

        redis::ErrorKind::AuthenticationFailed
        | redis::ErrorKind::InvalidClientConfig => Error::AuthenticationError,

        redis::ErrorKind::TypeError => Error::TypeError,

        redis::ErrorKind::ExecAbortError
        | redis::ErrorKind::BusyLoadingError
        | redis::ErrorKind::TryAgain
        | redis::ErrorKind::ClientError
        | redis::ErrorKind::ExtensionError
        | redis::ErrorKind::ReadOnly => Error::OperationError,

        redis::ErrorKind::NoScriptError
        | redis::ErrorKind::Moved
        | redis::ErrorKind::Ask => Error::NotFound,

        _ => Error::UnknownError,
    }
}

fn xlat_result<T>(res: redis::RedisResult<T>) -> Result<T> {
    res.map_err(xlat_err)
}

// Encodes a `Value` into a binary which gets stored in redis. This
// encoding lets us store type information in redis so there's no
// rounding errors or misinterpretation of the data.

fn to_redis(val: &Value) -> Vec<u8> {
    match val {
        Value::Bool(false) => vec![b'F'],
        Value::Bool(true) => vec![b'T'],

        Value::Int(v) => {
            let mut buf: Vec<u8> = Vec::with_capacity(9);

            buf.push(b'I');
            buf.extend_from_slice(&v.to_be_bytes());
            buf
        }

        Value::Flt(v) => {
            let mut buf: Vec<u8> = Vec::with_capacity(9);

            buf.push(b'D');
            buf.extend_from_slice(&v.to_be_bytes());
            buf
        }

        Value::Str(s) => {
            let s = s.as_bytes();
            let mut buf: Vec<u8> = Vec::with_capacity(5 + s.len());

            buf.push(b'S');
            buf.extend_from_slice(&(s.len() as u32).to_be_bytes());
            buf.extend_from_slice(s);
            buf
        }
    }
}

// Decodes an `i64` from an 8-byte buffer.

fn decode_integer(buf: &[u8]) -> Result<Value> {
    if buf.len() >= 8 {
        let buf = buf[..8].try_into().unwrap();

        return Ok(Value::Int(i64::from_be_bytes(buf)));
    }
    Err(Error::TypeError)
}

// Decodes an `f64` from an 8-byte buffer.

fn decode_float(buf: &[u8]) -> Result<Value> {
    if buf.len() >= 8 {
        let buf = buf[..8].try_into().unwrap();

        return Ok(Value::Flt(f64::from_be_bytes(buf)));
    }
    Err(Error::TypeError)
}

// Decodes a UTF-8 encoded string from a raw, u8 buffer.

fn decode_string(buf: &[u8]) -> Result<Value> {
    if buf.len() >= 4 {
        let len_buf = buf[..4].try_into().unwrap();
        let len = u32::from_be_bytes(len_buf) as usize;

        if buf.len() >= (4 + len) as usize {
            let str_vec = buf[4..4 + len].to_vec();

            return match String::from_utf8(str_vec) {
                Ok(s) => Ok(Value::Str(s)),
                Err(_) => Err(Error::TypeError),
            };
        }
    }
    Err(Error::TypeError)
}

// Returns a `Value` from a `redis::Value`. The only enumeration we
// support is the `Value::Data` form since that's the one used to
// return redis data.

fn from_value(v: &redis::Value) -> Result<Value> {
    if let redis::Value::Data(buf) = v {
        // The buffer has to have at least one character in order to
        // be decoded.

        if !buf.is_empty() {
            match buf[0] as char {
                'F' => Ok(Value::Bool(false)),
                'T' => Ok(Value::Bool(true)),
                'I' => decode_integer(&buf[1..]),
                'D' => decode_float(&buf[1..]),
                'S' => decode_string(&buf[1..]),

                // Any other character in the tag field is unknown and
                // can't be decoded as a `Value`.
                _ => Err(Error::TypeError),
            }
        } else {
            Err(Error::TypeError)
        }
    } else {
        Err(Error::TypeError)
    }
}

/// Defines a context that uses redis for the back-end storage.
pub struct RedisStore {
    /// This connection is used for interacting with the database.
    db_con: redis::aio::MultiplexedConnection,
    table: HashMap<Name, TxDeviceSetting>,
}

impl RedisStore {
    // Creates a connection to redis.

    async fn make_connection(
        cfg: &backend::Config, name: Option<String>, pword: Option<String>,
    ) -> Result<redis::aio::MultiplexedConnection> {
        use redis::{ConnectionAddr, ConnectionInfo, RedisConnectionInfo};

        let addr = cfg.get_addr();

        let ci = ConnectionInfo {
            addr: ConnectionAddr::Tcp(addr.ip().to_string(), addr.port()),
            redis: RedisConnectionInfo {
                db: cfg.get_dbn(),
                username: name,
                password: pword,
            },
        };

        let client = redis::Client::open(ci).unwrap();

        xlat_result(client.get_multiplexed_tokio_connection().await)
    }

    /// Builds a new backend context which interacts with `redis`.
    /// The parameters in `cfg` will be used to locate the `redis`
    /// instance. If `name` and `pword` are not `None`, they will be
    /// used for credentials when connecting to `redis`.

    pub async fn new(
        cfg: &backend::Config, name: Option<String>, pword: Option<String>,
    ) -> Result<Self> {
        let db_con = RedisStore::make_connection(cfg, name, pword).await?;

        Ok(RedisStore {
            db_con,
            table: HashMap::new(),
        })
    }

    // Returns the key that returns meta information for the device.

    fn info_key(&self, name: &str) -> String {
        format!("{}#info", name)
    }

    // Returns the key that returns time-series information for the
    // device.

    fn history_key(&self, name: &str) -> String {
        format!("{}#hist", name)
    }

    async fn last_value(&mut self, name: &str) -> Option<Value> {
        let result: Result<HashMap<String, HashMap<String, redis::Value>>> =
            xlat_result(
                redis::pipe()
                    .xrevrange_count(name, "-", "+", 1usize)
                    .query_async(&mut self.db_con)
                    .await,
            );

        if let Ok(v) = result {
            if let Some((_k, m)) = v.iter().next() {
                if let Some(val) = m.get("value") {
                    return from_value(&val).ok();
                } else {
                    debug!("no 'value' field for {}", name);
                }
            } else {
                debug!("empty results for {}", name);
            }
        } else {
            debug!("no previous value for {}", name);
        }
        None
    }

    // Does some sanity checks on a device to see if it appears to be
    // valid.

    async fn validate_device(&mut self, name: &str) -> Result<()> {
        // This section verifies the device has a NAME#info key that
        // is a hash map.

        {
            let info_key = self.info_key(name);
            let result: Result<String> = xlat_result(
                redis::cmd("TYPE")
                    .arg(&info_key)
                    .query_async(&mut self.db_con)
                    .await,
            );

            match result {
                Ok(data_type) if data_type.as_str() == "hash" => (),
                Ok(_) => {
                    warn!("{} is of the wrong key type", &info_key);
                    return Err(Error::TypeError);
                }
                Err(_) => {
                    warn!("{} doesn't exist", &info_key);
                    return Err(Error::NotFound);
                }
            }
        }

        // This section verifies the device has a NAME#hist key that
        // is a time-series stream.

        {
            let hist_key = self.history_key(name);
            let result: Result<String> = xlat_result(
                redis::cmd("TYPE")
                    .arg(&hist_key)
                    .query_async(&mut self.db_con)
                    .await,
            );

            match result {
                Ok(data_type) if data_type.as_str() == "stream" => Ok(()),
                Ok(_) => {
                    warn!("{} is of the wrong key type", &hist_key);
                    Err(Error::TypeError)
                }
                Err(_) => {
                    warn!("{} doesn't exist", &hist_key);
                    Err(Error::NotFound)
                }
            }
        }
    }

    // Initializes the state of a DrMem device in the REDIS database.
    // It creates two keys: one key is appended with "#info" and
    // addresses a hash table which will contain device meta
    // information; the other key is appended with "#hist" and is a
    // time-series stream which holds recent history of a device's
    // values.

    async fn init_device(
        &mut self, name: &str, units: &Option<String>,
    ) -> Result<()> {
        debug!("initializing {}", name);

        let hist_key = self.history_key(name);
        let info_key = self.info_key(name);

        // Create a command pipeline that deletes the two keys and
        // then creates them properly with default values.

        let fields: Vec<(&str, String)> = if let Some(units) = units {
            vec![("units", units.clone())]
        } else {
            vec![]
        };

        xlat_result(
            redis::pipe()
                .atomic()
                .del(&hist_key)
                .xadd(&hist_key, "1", &[("value", &[1u8])])
                .xdel(&hist_key, &["1"])
                .del(&info_key)
                .hset_multiple(&info_key, &fields)
                .query_async(&mut self.db_con)
                .await,
        )
    }

    fn mk_report_func(&self, name: &str) -> ReportReading {
        let hist_key = self.history_key(name);
        let db_con = self.db_con.clone();

        Box::new(move |v| {
            let mut db_con = db_con.clone();
            let hist_key = hist_key.clone();
            let data = [("value", to_redis(&v))];

            Box::pin(async move {
                redis::pipe()
                    .xadd(&hist_key, "*", &data)
                    .query_async(&mut db_con)
                    .map(xlat_result)
                    .await
            })
        })
    }
}

#[async_trait]
impl Store for RedisStore {
    /// Registers a device in the redis backend.

    async fn register_read_only_device(
        &mut self, _driver_name: &str, name: &Name, units: &Option<String>,
    ) -> Result<(ReportReading, Option<Value>)> {
	let name = name.to_string();

        debug!("registering '{}' as read-only", &name);

        if self.validate_device(&name).await.is_err() {
            self.init_device(&name, units).await?;

            info!("'{}' has been successfully created", &name);
        }
        Ok((self.mk_report_func(&name), self.last_value(&name).await))
    }

    async fn register_read_write_device(
        &mut self, _driver_name: &str, name: &Name, units: &Option<String>,
    ) -> Result<(ReportReading, RxDeviceSetting, Option<Value>)> {
	let sname = name.to_string();

        debug!("registering '{}' as read-write", &sname);

        if self.validate_device(&sname).await.is_err() {
            self.init_device(&sname, units).await?;

            info!("'{}' has been successfully created", &sname);
        }

        let (tx, rx) = mpsc::channel(20);
        let _ = self.table.insert(name.clone(), tx);

        Ok((self.mk_report_func(&sname), rx, self.last_value(&sname).await))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use drmem_api::types::device;
    use redis::Value;

    // We only want to convert Value::Data() forms. These tests make
    // sure the other variants don't translate.

    #[tokio::test]
    async fn test_reject_invalid_forms() {
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

    // Test correct decoding of Value::Bool values.

    #[tokio::test]
    async fn test_bool_decoder() {
        assert_eq!(
            Ok(device::Value::Bool(false)),
            from_value(&Value::Data(vec!['F' as u8]))
        );
        assert_eq!(
            Ok(device::Value::Bool(true)),
            from_value(&Value::Data(vec!['T' as u8]))
        );
    }

    // Test correct encoding of Value::Bool values.

    #[tokio::test]
    async fn test_bool_encoder() {
        assert_eq!(vec!['F' as u8], to_redis(&device::Value::Bool(false)));
        assert_eq!(vec!['T' as u8], to_redis(&device::Value::Bool(true)));
    }

    const INT_TEST_CASES: &[(i64, &[u8])] = &[
        (
            0,
            &['I' as u8, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
        ),
        (
            1,
            &['I' as u8, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01],
        ),
        (
            -1,
            &['I' as u8, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff],
        ),
        (
            0x7fffffffffffffff,
            &['I' as u8, 0x7f, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff],
        ),
        (
            -0x8000000000000000,
            &['I' as u8, 0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
        ),
        (
            0x0123456789abcdef,
            &['I' as u8, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef],
        ),
    ];

    // Test correct encoding of Value::Int values.

    #[tokio::test]
    async fn test_int_encoder() {
        for (v, rv) in INT_TEST_CASES {
            assert_eq!(*rv, to_redis(&device::Value::Int(*v)));
        }
    }

    // Test correct decoding of Value::Int values.

    #[tokio::test]
    async fn test_int_decoder() {
        assert!(from_value(&Value::Data(vec![])).is_err());
        assert!(from_value(&Value::Data(vec!['I' as u8])).is_err());
        assert!(from_value(&Value::Data(vec!['I' as u8, 0u8])).is_err());
        assert!(from_value(&Value::Data(vec!['I' as u8, 0u8, 0u8])).is_err());
        assert!(
            from_value(&Value::Data(vec!['I' as u8, 0u8, 0u8, 0u8])).is_err()
        );

        for (v, rv) in INT_TEST_CASES {
            let data = Value::Data(rv.to_vec());

            assert_eq!(Ok(device::Value::Int(*v)), from_value(&data));
        }
    }
}

pub async fn open(cfg: &backend::Config) -> Result<impl Store> {
    RedisStore::new(cfg, None, None).await
}

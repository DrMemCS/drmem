use async_trait::async_trait;
use drmem_api::{types::{device::Value, Error}, device::Device, Store, Result, driver::{ReportReading, RxDeviceSetting}};
use std::collections::HashMap;
use std::convert::TryInto;
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

fn xlat_result<T>(res: redis::RedisResult<T>) -> Result<T> {
    match res {
        Ok(res) => Ok(res),
        Err(err) => match err.kind() {
            redis::ErrorKind::ResponseError
            | redis::ErrorKind::ClusterDown
            | redis::ErrorKind::CrossSlot
            | redis::ErrorKind::MasterDown
            | redis::ErrorKind::IoError => Err(Error::DbCommunicationError),

            redis::ErrorKind::AuthenticationFailed
            | redis::ErrorKind::InvalidClientConfig => {
                Err(Error::AuthenticationError)
            }

            redis::ErrorKind::TypeError => Err(Error::TypeError),

            redis::ErrorKind::ExecAbortError
            | redis::ErrorKind::BusyLoadingError
            | redis::ErrorKind::TryAgain
            | redis::ErrorKind::ClientError
            | redis::ErrorKind::ExtensionError
            | redis::ErrorKind::ReadOnly => Err(Error::OperationError),

            redis::ErrorKind::NoScriptError
            | redis::ErrorKind::Moved
            | redis::ErrorKind::Ask => Err(Error::NotFound),

            _ => Err(Error::UnknownError),
        },
    }
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

        Value::Rgba(c) => {
            let mut buf: Vec<u8> = Vec::with_capacity(5);

            buf.push(b'C');
            buf.extend_from_slice(&c.to_be_bytes());
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

// Decodes an RGBA value from a 4-byte buffer.

fn decode_color(buf: &[u8]) -> Result<Value> {
    if buf.len() >= 4 {
        let buf = buf[..4].try_into().unwrap();

        return Ok(Value::Rgba(u32::from_be_bytes(buf)));
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
                'C' => decode_color(&buf[1..]),

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
    /// The base name used by the instance of the driver. Defining
    /// `Device` instances will add the last segment to the name.
    base: String,

    /// This connection is used for interacting with the database.
    db_con: redis::aio::Connection,
}

impl RedisStore {
    // Creates a connection to redis.

    async fn make_connection(
        cfg: &drmem_config::backend::Config, name: Option<String>,
        pword: Option<String>,
    ) -> Result<redis::aio::Connection> {
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

        xlat_result(client.get_tokio_connection().await)
    }

    /// Builds a new backend context which interacts with `redis`.
    /// The parameters in `cfg` will be used to locate the `redis`
    /// instance. If `name` and `pword` are not `None`, they will be
    /// used for credentials when connecting to `redis`.

    pub async fn new(
        base_name: &str, cfg: &drmem_config::backend::Config,
        name: Option<String>, pword: Option<String>,
    ) -> Result<Self> {
        let db_con = RedisStore::make_connection(cfg, name, pword).await?;

        Ok(RedisStore {
            base: String::from(base_name),
            db_con,
        })
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

    async fn get_device<T: Into<Value> + Send>(
        &mut self, name: &str,
    ) -> Result<Device<T>> {
        let info_key = self.info_key(name);
        let data_type: String = xlat_result(
            redis::cmd("TYPE")
                .arg(&info_key)
                .query_async(&mut self.db_con)
                .await,
        )?;

        // If the info key is a "hash" type, we assume the device has
        // been created and maintained properly.

        if data_type.as_str() == "hash" {
            let mut result: HashMap<String, redis::Value> = xlat_result(
                redis::Cmd::hgetall(&info_key)
                    .query_async(&mut self.db_con)
                    .await,
            )?;

            // Convert the HaspMap<String, redis::Value> into a
            // HashMap<String, Value>. As it converts each entry, it
            // checks to see if the associated redis::Value can be
            // translated. If not, it is ignored.

            let fields = result
                .drain()
                .filter_map(|(k, v)| {
                    if let Ok(v) = from_value(&v) {
                        Some((k, v))
                    } else {
                        None
                    }
                })
                .collect();

            Device::create_from_map(name, fields)
        } else {
            Err(Error::NotFound)
        }
    }
}

#[async_trait]
impl Store for RedisStore {

    /// Registers a device in the redis backend.

    async fn register_read_only_device(
        &mut self, name: &str,
    ) -> Result<ReportReading> {
        let dev_name = format!("{}:{}", &self.base, &name);

        debug!("defining '{}'", &dev_name);

        match self.get_device(name).await {
            Ok(v) => Ok(v),
            Err(e) => {
                warn!("'{}' isn't defined properly -- {:?}", &dev_name, e);

                let hist_key = self.history_key(name);
                let info_key = self.info_key(name);
                let dev = Device::create(name, String::from("summary"), Some(String::from("units")));

                let temp = dev.to_vec();
                let fields: Vec<(String, Vec<u8>)> = temp
                    .iter()
                    .map(|(k, v)| (String::from(*k), to_redis(v)))
                    .collect();

                // Create a command pipeline that deletes the two keys
                // and then creates them properly with default values.

                let _: () = xlat_result(
                    redis::pipe()
                        .atomic()
                        .del(&hist_key)
                        .xadd(
                            &hist_key,
                            "1",
                            &[("value", to_redis(&0i64.into()))],
                        )
                        .xdel(&hist_key, &["1"])
                        .del(&info_key)
                        .hset_multiple(&info_key, &fields)
                        .query_async(&mut self.db_con)
                        .await,
                )?;

                info!("'{}' has been successfully created", &dev_name);
                Ok(dev)
            }
        }
    }

    async fn register_read_write_device(
        &mut self, name: &str,
    ) -> Result<(ReportReading, RxDeviceSetting, Option<Value>)> {
        //let mut pipe = redis::pipe();
        //let mut cmd = pipe.atomic();

        //for (dev, val) in values {
        //    let key = self.history_key(dev);

        //    cmd = cmd.xadd(key, "*", &[("value", to_redis(val))]);

            // TODO: need to check alarm limits -- and add the command
            // to announce it -- as the command is built-up.
        //}

        //xlat_result(cmd.query_async(&mut self.db_con).await)
	unimplemented!()
    }
}

// This section holds code used for testing the module. The
// "#[cfg(test)]" attribute means the module will only be compiled and
// included in the test executable; debug and release versions won't
// have the code.

#[cfg(test)]
mod tests {
    use super::*;
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
            Ok(Value::Bool(false)),
            from_value(&Value::Data(vec!['F' as u8]))
        );
        assert_eq!(
            Ok(Value::Bool(true)),
            from_value(&Value::Data(vec!['T' as u8]))
        );
    }

    // Test correct encoding of Value::Bool values.

    #[tokio::test]
    async fn test_bool_encoder() {
        assert_eq!(vec!['F' as u8], to_redis(&Value::Bool(false)));
        assert_eq!(vec!['T' as u8], to_redis(&Value::Bool(true)));
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
            assert_eq!(*rv, to_redis(&Value::Int(*v)));
        }
    }

    // Test correct decoding of Value::Int values.

    #[tokio::test]
    async fn test_int_decoder() {
        for (v, rv) in INT_TEST_CASES {
            let data = Value::Data(rv.to_vec());

            assert_eq!(Ok(Value::Int(*v)), from_value(&data));
        }
    }

    const RGBA_TEST_CASES: &[(u32, &[u8])] = &[
        (0, &['C' as u8, 0x00, 0x00, 0x00, 0x00]),
        (0xff, &['C' as u8, 0x00, 0x00, 0x00, 0xff]),
        (0xff00, &['C' as u8, 0x00, 0x00, 0xff, 0x00]),
        (0xff0000, &['C' as u8, 0x00, 0xff, 0x00, 0x00]),
        (0xff000000, &['C' as u8, 0xff, 0x00, 0x00, 0x00]),
    ];

    // Test correct encoding of Value::Rgba values.

    #[tokio::test]
    async fn test_rgb_encoder() {
        for (v, rv) in RGBA_TEST_CASES {
            assert_eq!(*rv, to_redis(&Value::Rgba(*v)));
        }
    }

    // Test correct decoding of Value::Rgba values.

    #[tokio::test]
    async fn test_rgb_decoder() {
        for (v, rv) in RGBA_TEST_CASES {
            let data = Value::Data(rv.to_vec());

            assert_eq!(Ok(Value::Rgba(*v)), from_value(&data));
        }
    }
}

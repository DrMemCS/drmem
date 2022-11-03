use async_trait::async_trait;
use drmem_api::{
    client,
    driver::{ReportReading, RxDeviceSetting, TxDeviceSetting},
    types::{
        device::{self, Value},
        Error,
    },
    Result, Store,
};
use drmem_config::backend;
use std::collections::HashMap;
use std::convert::TryInto;
use tokio::sync::{broadcast, mpsc};
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
        Value::Bool(false) => vec![b'B', b'F'],
        Value::Bool(true) => vec![b'B', b'T'],

        // Integers start with an 'I' followed by 4 bytes.
        Value::Int(v) => {
            let mut buf: Vec<u8> = Vec::with_capacity(9);

            buf.push(b'I');
            buf.extend_from_slice(&v.to_be_bytes());
            buf
        }

        // Floating point values start with a 'D' and are followed by
        // 8 bytes.
        Value::Flt(v) => {
            let mut buf: Vec<u8> = Vec::with_capacity(9);

            buf.push(b'D');
            buf.extend_from_slice(&v.to_be_bytes());
            buf
        }

        // Strings start with an 'S', followed by a 4-byte length
        // field, and then followed by the string content.
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

// Decodes an `i32` from an 4-byte buffer.

fn decode_integer(buf: &[u8]) -> Result<Value> {
    if buf.len() >= 4 {
        let buf = buf[..4].try_into().unwrap();

        return Ok(Value::Int(i32::from_be_bytes(buf)));
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
                'B' if buf.len() > 1 => match buf[1] {
                    b'F' => Ok(Value::Bool(false)),
                    b'T' => Ok(Value::Bool(true)),
                    _ => Err(Error::TypeError),
                },
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
    table: HashMap<device::Name, TxDeviceSetting>,
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

    fn info_key(name: &str) -> String {
        format!("{}#info", name)
    }

    // Returns the key that returns time-series information for the
    // device.

    fn history_key(name: &str) -> String {
        format!("{}#hist", name)
    }

    // Builds the low-level command that returns the last value of the
    // device.

    fn last_value_cmd(name: &str) -> redis::Pipeline {
        let name = RedisStore::history_key(name);

        redis::pipe()
            .xrevrange_count(name, "+", "-", 1usize)
            .clone()
    }

    fn match_pattern_cmd(pattern: &Option<String>) -> redis::Pipeline {
        // Take the pattern from the caller and append "#info" since
        // we only want to look at device information keys.

        let pattern = pattern
            .as_ref()
            .map(|v| RedisStore::info_key(v))
            .unwrap_or_else(|| String::from("*#info"));

        // Query REDIS to return all keys that match our pattern.

        redis::pipe().keys(pattern).clone()
    }

    // Builds the low-level command that returns the type of the
    // device's meta record.

    fn type_cmd(name: &str) -> redis::Cmd {
        let key = RedisStore::info_key(name);

        redis::cmd("TYPE").arg(&key).clone()
    }

    async fn lookup_device(&self, name: &str) -> Result<client::DevInfoReply> {
        todo!()
    }

    // Obtains the last value reported for a device, or `None` if
    // there is no history for it.

    async fn last_value(&mut self, name: &str) -> Option<Value> {
        let result: Result<HashMap<String, HashMap<String, redis::Value>>> =
            xlat_result(
                RedisStore::last_value_cmd(name)
                    .query_async(&mut self.db_con)
                    .await,
            );

        if let Ok(v) = result {
            if let Some((_k, m)) = v.iter().next() {
                if let Some(val) = m.get("value") {
                    return from_value(val).ok();
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
            let cmd = RedisStore::type_cmd(name);
            let result: Result<String> =
                xlat_result(cmd.query_async(&mut self.db_con).await);

            match result {
                Ok(data_type) if data_type.as_str() == "hash" => (),
                Ok(_) => {
                    warn!("{} is of the wrong key type", name);
                    return Err(Error::TypeError);
                }
                Err(_) => {
                    warn!("{} doesn't exist", name);
                    return Err(Error::NotFound);
                }
            }
        }

        // This section verifies the device has a NAME#hist key that
        // is a time-series stream.

        {
            let hist_key = RedisStore::history_key(name);
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

        let hist_key = RedisStore::history_key(name);
        let info_key = RedisStore::info_key(name);

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

    fn report_new_value_cmd(key: &str, val: &device::Value) -> redis::Pipeline {
        let data = [("value", to_redis(val))];

        redis::pipe().xadd(key, "*", &data).clone()
    }

    fn mk_report_func(&self, name: &str) -> ReportReading {
        let db_con = self.db_con.clone();
        let name = String::from(name);

        Box::new(move |v| {
            let mut db_con = db_con.clone();
            let hist_key = RedisStore::history_key(&name);
            let name = name.clone();
            let data = [("value", to_redis(&v))];

            Box::pin(async move {
                if let Err(e) = RedisStore::report_new_value_cmd(&hist_key, &v)
                    .query_async::<redis::aio::MultiplexedConnection, ()>(
                        &mut db_con,
                    )
                    .await
                {
                    warn!("couldn't save {} data to redis ... {}", &name, e)
                }
            })
        })
    }
}

#[async_trait]
impl Store for RedisStore {
    /// Registers a device in the redis backend.

    async fn register_read_only_device(
        &mut self, _driver_name: &str, name: &device::Name,
        units: &Option<String>,
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
        &mut self, _driver_name: &str, name: &device::Name,
        units: &Option<String>,
    ) -> Result<(ReportReading, RxDeviceSetting, Option<Value>)> {
        let sname = name.to_string();

        debug!("registering '{}' as read-write", &sname);

        if self.validate_device(&sname).await.is_err() {
            self.init_device(&sname, units).await?;

            info!("'{}' has been successfully created", &sname);
        }

        let (tx, rx) = mpsc::channel(20);
        let _ = self.table.insert(name.clone(), tx);

        Ok((
            self.mk_report_func(&sname),
            rx,
            self.last_value(&sname).await,
        ))
    }

    // Implement the GraphQL query to pull device information.

    async fn get_device_info(
        &mut self, pattern: &Option<String>,
    ) -> Result<Vec<client::DevInfoReply>> {
        todo!()
    }

    async fn set_device(
        &self, name: device::Name, value: Value,
    ) -> Result<Value> {
        todo!()
    }

    async fn monitor_device(
        &self, name: device::Name,
    ) -> Result<broadcast::Receiver<device::Reading>> {
        todo!()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use drmem_api::types::device;
    use redis::Value;

    // We only want to convert Value::Data() forms. These tests make
    // sure the other variants don't translate.

    #[test]
    fn test_reject_invalid_forms() {
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

    #[test]
    fn test_bool_decoder() {
        assert_eq!(
            Ok(device::Value::Bool(false)),
            from_value(&Value::Data(vec![b'B', b'F']))
        );
        assert_eq!(
            Ok(device::Value::Bool(true)),
            from_value(&Value::Data(vec![b'B', b'T']))
        );
    }

    // Test correct encoding of Value::Bool values.

    #[test]
    fn test_bool_encoder() {
        assert_eq!(vec![b'B', b'F'], to_redis(&device::Value::Bool(false)));
        assert_eq!(vec![b'B', b'T'], to_redis(&device::Value::Bool(true)));
    }

    const INT_TEST_CASES: &[(i32, &[u8])] = &[
        (0, &[b'I', 0x00, 0x00, 0x00, 0x00]),
        (1, &[b'I', 0x00, 0x00, 0x00, 0x01]),
        (-1, &[b'I', 0xff, 0xff, 0xff, 0xff]),
        (0x7fffffff, &[b'I', 0x7f, 0xff, 0xff, 0xff]),
        (-0x80000000, &[b'I', 0x80, 0x00, 0x00, 0x00]),
        (0x01234567, &[b'I', 0x01, 0x23, 0x45, 0x67]),
    ];

    // Test correct encoding of Value::Int values.

    #[test]
    fn test_int_encoder() {
        for (v, rv) in INT_TEST_CASES {
            assert_eq!(*rv, to_redis(&device::Value::Int(*v)));
        }
    }

    // Test correct decoding of Value::Int values.

    #[test]
    fn test_int_decoder() {
        assert!(from_value(&Value::Data(vec![])).is_err());
        assert!(from_value(&Value::Data(vec![b'I'])).is_err());
        assert!(from_value(&Value::Data(vec![b'I', 0u8])).is_err());
        assert!(from_value(&Value::Data(vec![b'I', 0u8, 0u8])).is_err());
        assert!(from_value(&Value::Data(vec![b'I', 0u8, 0u8, 0u8])).is_err());

        for (v, rv) in INT_TEST_CASES {
            let data = Value::Data(rv.to_vec());

            assert_eq!(Ok(device::Value::Int(*v)), from_value(&data));
        }
    }

    const FLT_TEST_CASES: &[(f64, &[u8])] = &[
        (0.0, &[b'D', 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]),
        (
            -0.0,
            &[b'D', 0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
        ),
        (1.0, &[b'D', 0x3f, 0xf0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]),
        (
            -1.0,
            &[b'D', 0xbf, 0xf0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
        ),
        (
            9007199254740991.0,
            &[b'D', 67, 63, 255, 255, 255, 255, 255, 255],
        ),
        (9007199254740992.0, &[b'D', 67, 64, 0, 0, 0, 0, 0, 0]),
    ];

    // Test correct encoding of Value::Flt values.

    #[test]
    fn test_float_encoder() {
        for (v, rv) in FLT_TEST_CASES {
            assert_eq!(*rv, to_redis(&device::Value::Flt(*v)));
        }
    }

    // Test correct decoding of Value::Int values.

    #[test]
    fn test_float_decoder() {
        assert!(from_value(&Value::Data(vec![])).is_err());
        assert!(from_value(&Value::Data(vec![b'D'])).is_err());
        assert!(from_value(&Value::Data(vec![b'D', 0u8])).is_err());
        assert!(from_value(&Value::Data(vec![b'D', 0u8, 0u8])).is_err());
        assert!(from_value(&Value::Data(vec![b'D', 0u8, 0u8, 0u8])).is_err());
        assert!(
            from_value(&Value::Data(vec![b'D', 0u8, 0u8, 0u8, 0u8])).is_err()
        );
        assert!(
            from_value(&Value::Data(vec![b'D', 0u8, 0u8, 0u8, 0u8, 0u8]))
                .is_err()
        );
        assert!(from_value(&Value::Data(vec![
            b'D', 0u8, 0u8, 0u8, 0u8, 0u8, 0u8
        ]))
        .is_err());
        assert!(from_value(&Value::Data(vec![
            b'D', 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8
        ]))
        .is_err());

        for (v, rv) in FLT_TEST_CASES {
            let data = Value::Data(rv.to_vec());

            assert_eq!(Ok(device::Value::Flt(*v)), from_value(&data));
        }
    }

    const STR_TEST_CASES: &[(&str, &[u8])] = &[
        ("", &[b'S', 0u8, 0u8, 0u8, 0u8]),
        ("ABC", &[b'S', 0u8, 0u8, 0u8, 3u8, b'A', b'B', b'C']),
    ];

    // Test correct encoding of Value::Str values.

    #[test]
    fn test_string_encoder() {
        for (v, rv) in STR_TEST_CASES {
            assert_eq!(*rv, to_redis(&device::Value::Str(String::from(*v))));
        }
    }

    // Test correct decoding of Value::Str values.

    #[test]
    fn test_string_decoder() {
        // Buffers smaller than 5 bytes are an error.

        assert!(from_value(&Value::Data(vec![])).is_err());
        assert!(from_value(&Value::Data(vec![b'S'])).is_err());
        assert!(from_value(&Value::Data(vec![b'S', 0u8])).is_err());
        assert!(from_value(&Value::Data(vec![b'S', 0u8, 0u8])).is_err());
        assert!(from_value(&Value::Data(vec![b'S', 0u8, 0u8, 0u8])).is_err());

        // Loop through the test cases.

        for (v, rv) in STR_TEST_CASES {
            let data = Value::Data(rv.to_vec());

            assert_eq!(
                Ok(device::Value::Str(String::from(*v))),
                from_value(&data)
            );
        }

        // Verify proper response (both good and bad) when the buffer
        // doesn't match the size of the string.

        assert!(
            from_value(&Value::Data(vec![b'S', 0u8, 0u8, 0u8, 1u8])).is_err()
        );
        assert!(
            from_value(&Value::Data(vec![b'S', 0u8, 0u8, 0u8, 2u8, b'A']))
                .is_err()
        );
        assert_eq!(
            Ok(device::Value::Str(String::from("AB"))),
            from_value(&Value::Data(vec![
                b'S', 0u8, 0u8, 0u8, 2u8, b'A', b'B', 0, 0
            ]))
        );
    }

    #[test]
    fn test_pattern_cmd() {
        assert_eq!(
            &RedisStore::match_pattern_cmd(&None).get_packed_pipeline(),
            b"*2\r
$4\r\nKEYS\r
$6\r\n*#info\r\n"
        );
        assert_eq!(
            &RedisStore::match_pattern_cmd(&Some(String::from("device")))
                .get_packed_pipeline(),
            b"*2\r
$4\r\nKEYS\r
$11\r\ndevice#info\r\n"
        );
    }

    #[test]
    fn test_type_cmd() {
        let cmd = RedisStore::type_cmd("device");

        assert_eq!(
            &cmd.get_packed_command(),
            b"*2\r
$4\r\nTYPE\r
$11\r\ndevice#info\r\n"
        );
    }

    #[test]
    fn test_last_value() {
        let pipe = RedisStore::last_value_cmd("device");

        assert_eq!(
            &pipe.get_packed_pipeline(),
            b"*6\r
$9\r\nXREVRANGE\r
$11\r\ndevice#hist\r
$1\r\n+\r
$1\r\n-\r
$5\r\nCOUNT\r
$1\r\n1\r\n"
        );
    }

    #[test]
    fn test_report_value_cmd() {
        assert_eq!(
            &RedisStore::report_new_value_cmd("key", &(true.into()))
                .get_packed_pipeline(),
            b"*5\r
$4\r\nXADD\r
$3\r\nkey\r
$1\r\n*\r
$5\r\nvalue\r
$2\r\nBT\r\n"
        );
        assert_eq!(
            &RedisStore::report_new_value_cmd("key", &(0x00010203i32.into()))
                .get_packed_pipeline(),
            b"*5\r
$4\r\nXADD\r
$3\r\nkey\r
$1\r\n*\r
$5\r\nvalue\r
$5\r\nI\x00\x01\x02\x03\r\n"
        );
        assert_eq!(
            &RedisStore::report_new_value_cmd("key", &(0x12345678i32.into()))
                .get_packed_pipeline(),
            b"*5\r
$4\r\nXADD\r
$3\r\nkey\r
$1\r\n*\r
$5\r\nvalue\r
$5\r\nI\x12\x34\x56\x78\r\n"
        );
        assert_eq!(
            &RedisStore::report_new_value_cmd("key", &(1.0.into()))
                .get_packed_pipeline(),
            b"*5\r
$4\r\nXADD\r
$3\r\nkey\r
$1\r\n*\r
$5\r\nvalue\r
$9\r\nD\x3f\xf0\x00\x00\x00\x00\x00\x00\r\n"
        );
        assert_eq!(
            &RedisStore::report_new_value_cmd(
                "key",
                &(String::from("hello").into())
            )
            .get_packed_pipeline(),
            b"*5\r
$4\r\nXADD\r
$3\r\nkey\r
$1\r\n*\r
$5\r\nvalue\r
$10\r\nS\x00\x00\x00\x05hello\r\n"
        );
    }
}

pub async fn open(cfg: &backend::Config) -> Result<impl Store> {
    RedisStore::new(cfg, None, None).await
}

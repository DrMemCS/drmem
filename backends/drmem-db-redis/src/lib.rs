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
use tokio::sync::{broadcast, mpsc, oneshot};
use tracing::{debug, error, info, info_span, warn};
use tracing_futures::Instrument;

type AioConnection = redis::aio::MultiplexedConnection;
type SettingTable = HashMap<device::Name, TxDeviceSetting>;

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
    db_con: AioConnection,
    table: SettingTable,
}

impl RedisStore {
    // Creates a connection to redis.

    async fn make_connection(
        cfg: &backend::Config, name: Option<String>, pword: Option<String>,
    ) -> Result<AioConnection> {
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

        async {
            let client = redis::Client::open(ci).map_err(xlat_err)?;

            client
                .get_multiplexed_tokio_connection()
                .await
                .map_err(|e| {
                    error!("redis error: {}", &e);

                    xlat_err(e)
                })
        }
        .instrument(info_span!("init"))
        .await
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

    fn hist_key(name: &str) -> String {
        format!("{}#hist", name)
    }

    fn init_device_cmd(
        name: &str, driver: &str, units: &Option<String>,
    ) -> redis::Pipeline {
        let hist_key = RedisStore::hist_key(name);
        let info_key = RedisStore::info_key(name);

        // Start an array of required fields.

        let mut fields: Vec<(&str, String)> =
            vec![("driver", String::from(driver))];

        // Optionally add a "units" field.

        if let Some(units) = units {
            fields.push(("units", units.clone()))
        };

        // Create a command pipeline that deletes the two keys and
        // then creates them properly with default values.

        redis::pipe()
            .atomic()
            .del(&hist_key)
            .xadd(&hist_key, "1", &[("value", &[1u8])])
            .xdel(&hist_key, &["1"])
            .del(&info_key)
            .hset_multiple(&info_key, &fields)
            .clone()
    }

    // Builds the low-level command that returns the last value of the
    // device.

    fn last_value_cmd(name: &str) -> redis::Pipeline {
        let name = RedisStore::hist_key(name);

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

    fn info_type_cmd(name: &str) -> redis::Cmd {
        let key = RedisStore::info_key(name);

        redis::cmd("TYPE").arg(&key).clone()
    }

    // Builds the low-level command that returns the type of the
    // device's history record.

    fn hist_type_cmd(name: &str) -> redis::Cmd {
        let key = RedisStore::hist_key(name);

        redis::cmd("TYPE").arg(&key).clone()
    }

    // Creates a redis command pipeline which returns the standard,
    // meta-data for a device.

    fn device_info_cmd(name: &str) -> redis::Pipeline {
        let info_key = RedisStore::info_key(name);

        redis::pipe().hgetall(&info_key).clone()
    }

    fn hash_to_info(
        st: &SettingTable, name: &device::Name, hmap: &HashMap<String, String>,
    ) -> Result<client::DevInfoReply> {
        // Redis doesn't return an error if the key doesn't exist; it
        // returns an empty array. So if our HashMap is empty, the key
        // didn't exist.

        if !hmap.is_empty() {
            // If a "units" field exists and it's a string, we can
            // save it in the `units` field of the reply.

            let units = hmap.get("units").map(String::clone);

            // If a "driver" field exists and it's a string, save it
            // in the "drivers" field of the reply.

            let driver = hmap
                .get("driver")
                .map(String::clone)
                .unwrap_or_else(|| String::from("*missing*"));

            Ok(client::DevInfoReply {
                name: name.clone(),
                units,
                settable: st.contains_key(name),
                driver,
            })
        } else {
            Err(Error::NotFound)
        }
    }

    // Looks up a device in the redis store and, if found, returns a
    // `client::DevInfoReply` containing the information.

    async fn lookup_device(
        &mut self, name: device::Name,
    ) -> Result<client::DevInfoReply> {
        RedisStore::device_info_cmd(name.to_string().as_str())
            .query_async::<AioConnection, (HashMap<String, String>,)>(
                &mut self.db_con,
            )
            .await
            .map_err(xlat_err)
            .and_then(|(v,)| RedisStore::hash_to_info(&self.table, &name, &v))
    }

    // Obtains the last value reported for a device, or `None` if
    // there is no history for it.

    async fn last_value(&mut self, name: &str) -> Option<Value> {
        let result: Result<HashMap<String, HashMap<String, redis::Value>>> =
            RedisStore::last_value_cmd(name)
                .query_async(&mut self.db_con)
                .await
                .map_err(xlat_err);

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
            let cmd = RedisStore::info_type_cmd(name);
            let result: redis::RedisResult<String> =
                cmd.query_async(&mut self.db_con).await;

            match result {
                Ok(data_type) if data_type.as_str() == "hash" => (),
                Ok(_) => {
                    warn!("{} meta is of the wrong key type", name);
                    return Err(Error::TypeError);
                }
                Err(_) => {
                    warn!("{} meta doesn't exist", name);
                    return Err(Error::NotFound);
                }
            }
        }

        // This section verifies the device has a NAME#hist key that
        // is a time-series stream.

        {
            let cmd = RedisStore::hist_type_cmd(name);
            let result: redis::RedisResult<String> =
                cmd.query_async(&mut self.db_con).await;

            match result {
                Ok(data_type) if data_type.as_str() == "stream" => Ok(()),
                Ok(_) => {
                    warn!("{} history is of the wrong key type", name);
                    Err(Error::TypeError)
                }
                Err(_) => {
                    warn!("{} history doesn't exist", name);
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
        &mut self, name: &str, driver: &str, units: &Option<String>,
    ) -> Result<()> {
        debug!("initializing {}", name);
        RedisStore::init_device_cmd(name, driver, units)
            .query_async(&mut self.db_con)
            .await
            .map_err(xlat_err)
    }

    // Generates a redis command pipeline that adds a value to a
    // device's history.

    fn report_new_value_cmd(key: &str, val: &device::Value) -> redis::Pipeline {
        let data = [("value", to_redis(val))];

        redis::pipe().xadd(key, "*", &data).clone()
    }

    // Creates a closure for a driver to report a device's changing
    // values.

    fn mk_report_func(&self, name: &str) -> ReportReading {
        let db_con = self.db_con.clone();
        let name = String::from(name);

        Box::new(move |v| {
            let mut db_con = db_con.clone();
            let hist_key = RedisStore::hist_key(&name);
            let name = name.clone();

            Box::pin(async move {
                if let Err(e) = RedisStore::report_new_value_cmd(&hist_key, &v)
                    .query_async::<AioConnection, ()>(&mut db_con)
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
        &mut self, driver_name: &str, name: &device::Name,
        units: &Option<String>,
    ) -> Result<(ReportReading, Option<Value>)> {
        let name = name.to_string();

        debug!("registering '{}' as read-only", &name);

        if self.validate_device(&name).await.is_err() {
            self.init_device(&name, driver_name, units).await?;

            info!("'{}' has been successfully created", &name);
        }
        Ok((self.mk_report_func(&name), self.last_value(&name).await))
    }

    async fn register_read_write_device(
        &mut self, driver_name: &str, name: &device::Name,
        units: &Option<String>,
    ) -> Result<(ReportReading, RxDeviceSetting, Option<Value>)> {
        let sname = name.to_string();

        debug!("registering '{}' as read-write", &sname);

        if self.validate_device(&sname).await.is_err() {
            self.init_device(&sname, driver_name, units).await?;

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

    // Implement the request to pull device information. Any task with
    // a client channel can make this request although the primary
    // client will be from GraphQL requests.

    async fn get_device_info(
        &mut self, pattern: &Option<String>,
    ) -> Result<Vec<client::DevInfoReply>> {
        // Get a list of all the keys that match the pattern. For
        // Redis, these keys will have "#info" appended at the end.

        let (result,): (Vec<String>,) = RedisStore::match_pattern_cmd(pattern)
            .query_async(&mut self.db_con)
            .await
            .map_err(xlat_err)?;

        // Create an empty container to hold the device info records.

        let mut devices = vec![];

        // Loop through the results and pull all the device
        // information. Strip off the trailing "#info" before getting
        // the device information.

        for key in result {
            // Only process keys that are valid device names.

            if let Ok(name) =
                key.trim_end_matches("#info").parse::<device::Name>()
            {
                let dev_info = self.lookup_device(name).await?;

                devices.push(dev_info)
            }
        }
        Ok(devices)
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
    fn test_info_type_cmd() {
        let cmd = RedisStore::info_type_cmd("device");

        assert_eq!(
            &cmd.get_packed_command(),
            b"*2\r
$4\r\nTYPE\r
$11\r\ndevice#info\r\n"
        );
    }

    #[test]
    fn test_hist_type_cmd() {
        let cmd = RedisStore::hist_type_cmd("device");

        assert_eq!(
            &cmd.get_packed_command(),
            b"*2\r
$4\r\nTYPE\r
$11\r\ndevice#hist\r\n"
        );
    }

    #[test]
    fn test_dev_info_cmd() {
        let cmd = RedisStore::device_info_cmd("device");

        assert_eq!(
            &cmd.get_packed_pipeline(),
            b"*2\r
$7\r\nHGETALL\r
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

    #[test]
    fn test_init_dev() {
        assert_eq!(
            String::from_utf8_lossy(
                &RedisStore::init_device_cmd("device", "mem", &None)
                    .get_packed_pipeline()
            ),
            "*1\r
$5\r\nMULTI\r
*2\r
$3\r\nDEL\r
$11\r\ndevice#hist\r
*5\r
$4\r\nXADD\r
$11\r\ndevice#hist\r
$1\r\n1\r
$5\r\nvalue\r
$1\r\n\x01\r
*3\r
$4\r\nXDEL\r
$11\r\ndevice#hist\r
$1\r\n1\r
*2\r
$3\r\nDEL\r
$11\r\ndevice#info\r
*4\r
$5\r\nHMSET\r
$11\r\ndevice#info\r
$6\r\ndriver\r
$3\r\nmem\r
*1\r
$4\r\nEXEC\r\n"
        );
        assert_eq!(
            String::from_utf8_lossy(
                &RedisStore::init_device_cmd(
                    "device",
                    "pump",
                    &Some(String::from("gpm"))
                )
                .get_packed_pipeline()
            ),
            "*1\r
$5\r\nMULTI\r
*2\r
$3\r\nDEL\r
$11\r\ndevice#hist\r
*5\r
$4\r\nXADD\r
$11\r\ndevice#hist\r
$1\r\n1\r
$5\r\nvalue\r
$1\r\n\x01\r
*3\r
$4\r\nXDEL\r
$11\r\ndevice#hist\r
$1\r\n1\r
*2\r
$3\r\nDEL\r
$11\r\ndevice#info\r
*6\r
$5\r\nHMSET\r
$11\r\ndevice#info\r
$6\r\ndriver\r
$4\r\npump\r
$5\r\nunits\r
$3\r\ngpm\r
*1\r
$4\r\nEXEC\r\n"
        );
    }

    #[test]
    fn test_hash_to_info() {
        let device = "path:junk".parse::<device::Name>().unwrap();
        let mut st = HashMap::new();
        let mut fm = HashMap::new();

        assert_eq!(
            RedisStore::hash_to_info(
                &st,
                &"path:junk".parse::<device::Name>().unwrap(),
                &fm
            ),
            Err(Error::NotFound)
        );

        let _ = fm.insert("units".to_string(), "gpm".to_string());

        assert_eq!(
            RedisStore::hash_to_info(&st, &device, &fm),
            Ok(client::DevInfoReply {
                name: device.clone(),
                units: Some(String::from("gpm")),
                settable: false,
                driver: String::from("*missing*"),
            })
        );

        let _ = fm.insert("driver".to_string(), "sump".to_string().into());

        assert_eq!(
            RedisStore::hash_to_info(&st, &device, &fm),
            Ok(client::DevInfoReply {
                name: device.clone(),
                units: Some(String::from("gpm")),
                settable: false,
                driver: String::from("sump"),
            })
        );

        let (tx, _) = mpsc::channel(10);
        let _ = st.insert(device.clone(), tx);

        assert_eq!(
            RedisStore::hash_to_info(&st, &device, &fm),
            Ok(client::DevInfoReply {
                name: device.clone(),
                units: Some(String::from("gpm")),
                settable: true,
                driver: String::from("sump"),
            })
        );
    }
}

pub async fn open(cfg: &backend::Config) -> Result<impl Store> {
    RedisStore::new(cfg, None, None).await
}

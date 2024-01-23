use async_trait::async_trait;
use chrono::*;
use drmem_api::{
    client, device,
    driver::{ReportReading, RxDeviceSetting, TxDeviceSetting},
    Error, Result, Store,
};
use futures::task::{Context, Poll};
use futures::Future;
use redis::{
    aio,
    streams::{StreamId, StreamInfoStreamReply},
};
use std::collections::HashMap;
use std::convert::TryInto;
use std::pin::Pin;
use std::time;
use tokio::sync::{mpsc, oneshot};
use tokio_stream::{self, Stream, StreamExt};
use tracing::{debug, error, info, info_span, warn};
use tracing_futures::Instrument;

type AioMplexConnection = aio::MultiplexedConnection;
type AioConnection = aio::Connection;
type SettingTable = HashMap<device::Name, TxDeviceSetting>;

pub mod config;

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
        redis::ErrorKind::AuthenticationFailed
        | redis::ErrorKind::InvalidClientConfig => Error::AuthenticationError,

        redis::ErrorKind::TypeError => Error::TypeError,

        redis::ErrorKind::ExecAbortError
        | redis::ErrorKind::BusyLoadingError
        | redis::ErrorKind::TryAgain
        | redis::ErrorKind::ClientError
        | redis::ErrorKind::ExtensionError
        | redis::ErrorKind::ReadOnly => {
            Error::OperationError("backend is read-only".to_owned())
        }

        redis::ErrorKind::NoScriptError
        | redis::ErrorKind::Moved
        | redis::ErrorKind::Ask => Error::NotFound,

        _ => Error::BackendError(format!("{}", &e)),
    }
}

// Encodes a `device::Value` into a binary which gets stored in
// redis. This encoding lets us store type information in redis so
// there's no rounding errors or misinterpretation of the data.

fn to_redis(val: &device::Value) -> Vec<u8> {
    match val {
        device::Value::Bool(false) => vec![b'B', b'F'],
        device::Value::Bool(true) => vec![b'B', b'T'],

        // Integers start with an 'I' followed by 4 bytes.
        device::Value::Int(v) => {
            let mut buf: Vec<u8> = Vec::with_capacity(9);

            buf.push(b'I');
            buf.extend_from_slice(&v.to_be_bytes());
            buf
        }

        // Floating point values start with a 'D' and are followed by
        // 8 bytes.
        device::Value::Flt(v) => {
            let mut buf: Vec<u8> = Vec::with_capacity(9);

            buf.push(b'D');
            buf.extend_from_slice(&v.to_be_bytes());
            buf
        }

        // Strings start with an 'S', followed by a 4-byte length
        // field, and then followed by the string content.
        device::Value::Str(s) => {
            let s = s.as_bytes();
            let mut buf: Vec<u8> = Vec::with_capacity(5 + s.len());

            buf.push(b'S');
            buf.extend_from_slice(&(s.len() as u32).to_be_bytes());
            buf.extend_from_slice(s);
            buf
        }

        // Colors start with a 'C', followed by 3 u8 values,
        // representing red, green, and blue intensities,
        // respectively.
        device::Value::Color(v) => vec![b'C', v.red, v.green, v.blue],
    }
}

// Decodes an `i32` from an 4-byte buffer.

fn decode_integer(buf: &[u8]) -> Result<device::Value> {
    if buf.len() >= 4 {
        let buf = buf[..4].try_into().unwrap();

        return Ok(device::Value::Int(i32::from_be_bytes(buf)));
    }
    Err(Error::TypeError)
}

// Decodes an `f64` from an 8-byte buffer.

fn decode_float(buf: &[u8]) -> Result<device::Value> {
    if buf.len() >= 8 {
        let buf = buf[..8].try_into().unwrap();

        return Ok(device::Value::Flt(f64::from_be_bytes(buf)));
    }
    Err(Error::TypeError)
}

// Decodes a UTF-8 encoded string from a raw, u8 buffer.

fn decode_string(buf: &[u8]) -> Result<device::Value> {
    if buf.len() >= 4 {
        let len_buf = buf[..4].try_into().unwrap();
        let len = u32::from_be_bytes(len_buf) as usize;

        if buf.len() >= (4 + len) {
            let str_vec = buf[4..4 + len].to_vec();

            return match String::from_utf8(str_vec) {
                Ok(s) => Ok(device::Value::Str(s)),
                Err(_) => Err(Error::TypeError),
            };
        }
    }
    Err(Error::TypeError)
}

fn decode_color(buf: &[u8]) -> Result<device::Value> {
    if buf.len() >= 3 {
        let rgb = palette::LinSrgb::new(buf[0], buf[1], buf[2]);

        Ok(device::Value::Color(rgb))
    } else {
        Err(Error::TypeError)
    }
}

// Returns a `device::Value` from a `redis::Value`. The only
// enumeration we support is the `redis::Value::Data` form since
// that's the one used to return redis data.

fn from_value(v: &redis::Value) -> Result<device::Value> {
    if let redis::Value::Data(buf) = v {
        // The buffer has to have at least one character in order to
        // be decoded.

        if !buf.is_empty() {
            match buf[0] as char {
                'B' if buf.len() > 1 => match buf[1] {
                    b'F' => Ok(device::Value::Bool(false)),
                    b'T' => Ok(device::Value::Bool(true)),
                    _ => Err(Error::TypeError),
                },
                'I' => decode_integer(&buf[1..]),
                'D' => decode_float(&buf[1..]),
                'S' => decode_string(&buf[1..]),
                'C' => decode_color(&buf[1..]),

                // Any other character in the tag field is unknown and
                // can't be decoded as a `device::Value`.
                _ => Err(Error::TypeError),
            }
        } else {
            Err(Error::TypeError)
        }
    } else {
        Err(Error::TypeError)
    }
}

// Subtracts 1 microsecond from a SystemTime value. If subtracting
// can't be done (would put the SystemTime out of range) then the
// passed in value is returned.

fn st_minus_1us(st: time::SystemTime) -> time::SystemTime {
    if let Some(val) = st.checked_sub(time::Duration::from_micros(1)) {
        val
    } else {
        st
    }
}

// Converts millisecond/microsecond values to SystemTime. The
// microsecond parameter is clipped to the range 0 - 999.

fn msus_to_st(ms: u64, us: u64) -> Result<time::SystemTime> {
    let ts = time::UNIX_EPOCH.checked_add(time::Duration::from_millis(ms));

    if let Some(ts) = ts {
        let ts = ts.checked_add(time::Duration::from_micros(std::cmp::min(
            us, 999u64,
        )));

        if let Some(ts) = ts {
            return Ok(ts);
        }
    }
    Err(Error::InvArgument(String::from("bad timestamp value")))
}

// Converts a redis stream ID ("###-###") into a `SystemTime`. In
// DrMem, we follow the redis convention that the main portion of the
// ID is milliseconds from 1970. The secondary portion is used to hold
// microseconds ("0" - "999", not "000" - "999").

fn id_to_ts(id: &str) -> Result<time::SystemTime> {
    let fields: Vec<&str> = id.split('-').collect();

    if let &[a, b] = &fields[..] {
        // The redis stream id has the form "#-#" where the first
        // number is a 64-bit value representing milliseconds since
        // 1970. The second portion is a sequence number which is only
        // used if the first number has a duplicate.  This keeps the
        // timestamps increasing in value. DrMem has a base time of 20
        // Hz (50 ms), so we should never have more than one timestamp
        // occur in the same millisecond. However, some may want to
        // push the boundaries, so we'll use the second number as a
        // microsecond field. This code will accept the second field
        // to be 0 - 999. If it exceeds 999, we'll clip it.

        if let (Ok(ms), Ok(us)) = (a.parse::<u64>(), b.parse::<u64>()) {
            return msus_to_st(ms, us);
        }
    }

    Err(Error::InvArgument(String::from("unknown timestamp format")))
}

type ReadFuture = Pin<
    Box<
        dyn Future<Output = (AioConnection, redis::RedisResult<redis::Value>)>
            + Send,
    >,
>;

type ReadingResult = ((String, ((String, HashMap<String, redis::Value>),)),);

struct ReadingStream {
    key: String,
    id: String,
    fut: ReadFuture,
}

impl ReadingStream {
    const TIMEOUT: usize = 5_000;

    // Converts a `time::SystemTime` into a redis stream id.
    // Microseconds are mapping into the secondary portion of the id.
    //
    // XXX: This function uses `.unwrap()`, which means this function
    // could `panic!`. However, the timestamps are being generated by
    // redis or DrMem so they should always be in range. If we allow
    // drivers to specify their own timestamps, this may need to be
    // revisited.

    fn ts_to_id(ts: time::SystemTime) -> String {
        let us = ts.duration_since(time::UNIX_EPOCH).unwrap().as_micros();

        format!("{}-{}", us / 1000, us % 1000)
    }

    fn read_next_cmd(key: &str, id: &str) -> redis::Cmd {
        let opts = redis::streams::StreamReadOptions::default()
            .block(Self::TIMEOUT)
            .count(1);

        redis::Cmd::xread_options(&[key], &[id], &opts)
    }

    // Create a future that returns the next device reading from a
    // redis stream (or times-out trying.) The connection is
    // "threaded" through the future (i.e. it takes ownership and
    // returns it with the result.) This is necessary because an
    // AioConnection isn't clonable.

    fn mk_fut(mut con: AioConnection, key: String, id: String) -> ReadFuture {
        Box::pin(async move {
            let result =
                Self::read_next_cmd(&key, &id).query_async(&mut con).await;

            (con, result)
        })
    }

    pub fn new(
        con: AioConnection,
        key: &str,
        id: Option<time::SystemTime>,
    ) -> Self {
        let key = key.to_string();
        let id = id.map(Self::ts_to_id).unwrap_or_else(|| String::from("$"));
        let fut = Self::mk_fut(con, key.clone(), id.clone());

        ReadingStream { key, id, fut }
    }

    fn parse_reading(data: &redis::Value) -> Option<(String, device::Reading)> {
        let result: redis::RedisResult<ReadingResult> =
            redis::from_redis_value(data);

        match result {
            Ok(((_, ((ref new_id, ref rmap),)),)) => {
                let reading = device::Reading {
                    ts: id_to_ts(new_id).ok()?,
                    value: from_value(rmap.get("value")?).ok()?,
                };

                Some((new_id.to_string(), reading))
            }
            Err(e) => {
                error!("couldn't parse reading: {:?}", &e);
                None
            }
        }
    }
}

// Implements a stream. Note this stream is probably not cancel-safe;
// without knowing the internals of the Future that returns the redis
// result, we can only assume the connection can be put in a bad state
// if this future (poll_next) is canceled. In DrMem, we typically
// consume the entire stream and, in the case of a client canceling
// its use of the stream, we've going to tear down the redis
// connection.

impl Stream for ReadingStream {
    type Item = device::Reading;

    // Polls the redis connection (via XREAD) for another reading and
    // returns it through the stream. The redis command set doesn't
    // have an infinite blocking command so this future uses the
    // timeout parameter to periodically wake up and retry the read.

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        loop {
            // If there is a `Poll::Ready` return value, then redis
            // sent an update.

            if let Poll::Ready(result) = Pin::new(&mut self.fut).poll(cx) {
                // If redis returned an error, report it and close the
                // stream.

                let (con, result) = match result {
                    (con, Ok(v)) => (con, v),
                    (_, Err(e)) => {
                        warn!("read error -- {}", &e);
                        break Poll::Ready(None);
                    }
                };

                // If there's no data, the timeout occurred. Redis has
                // no "block forever" request, so the best that can be
                // done is to wake up periodically and retry the
                // request.

                if result != redis::Value::Nil {
                    if let Some((id, reading)) = Self::parse_reading(&result) {
                        // This future is no longer good. Create a new
                        // future using the updated `id`.

                        self.id = id;
                        self.fut = Self::mk_fut(
                            con,
                            self.key.clone(),
                            self.id.clone(),
                        );

                        // Return the reading data.

                        break Poll::Ready(Some(reading));
                    } else {
                        break Poll::Ready(None);
                    }
                } else {
                    // The read command timed out. Re-issue the future
                    // using the same `id` and loop.

                    self.fut =
                        Self::mk_fut(con, self.key.clone(), self.id.clone());
                }
            } else {
                break Poll::Pending;
            }
        }
    }
}

/// Defines a context that uses redis for the back-end storage.
pub struct RedisStore {
    /// This connection is used for interacting with the database.
    db_con: AioMplexConnection,
    table: SettingTable,
    cfg: config::Config,
}

impl RedisStore {
    fn make_client(
        cfg: &config::Config,
        name: Option<&String>,
        pword: Option<&String>,
    ) -> Result<redis::Client> {
        use redis::{ConnectionAddr, ConnectionInfo, RedisConnectionInfo};

        let addr = cfg.get_addr();

        let ci = ConnectionInfo {
            addr: ConnectionAddr::Tcp(addr.ip().to_string(), addr.port()),
            redis: RedisConnectionInfo {
                db: cfg.get_dbn(),
                username: name.cloned(),
                password: pword.cloned(),
            },
        };

        redis::Client::open(ci).map_err(xlat_err)
    }

    // Creates a single-user connection to redis.

    async fn make_connection(
        cfg: &config::Config,
        name: Option<String>,
        pword: Option<String>,
    ) -> Result<AioConnection> {
        let client = Self::make_client(cfg, name.as_ref(), pword.as_ref())?;

        debug!("creating new redis connection");

        client.get_tokio_connection().await.map_err(|e| {
            error!("redis error: {}", &e);
            xlat_err(e)
        })
    }

    // Creates a mulitplexed connection to redis.

    async fn make_mplex_connection(
        cfg: &config::Config,
        name: Option<String>,
        pword: Option<String>,
    ) -> Result<AioMplexConnection> {
        let client = Self::make_client(cfg, name.as_ref(), pword.as_ref())?;

        debug!("creating new, shared redis connection");

        client
            .get_multiplexed_tokio_connection()
            .await
            .map_err(|e| {
                error!("redis error: {}", &e);
                xlat_err(e)
            })
    }

    /// Builds a new backend context which interacts with `redis`.
    /// The parameters in `cfg` will be used to locate the `redis`
    /// instance. If `name` and `pword` are not `None`, they will be
    /// used for credentials when connecting to `redis`.

    pub async fn new(
        cfg: &config::Config,
        name: Option<String>,
        pword: Option<String>,
    ) -> Result<Self> {
        let db_con = Self::make_mplex_connection(cfg, name, pword).await?;

        Ok(RedisStore {
            db_con,
            table: HashMap::new(),
            cfg: cfg.clone(),
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
        name: &str,
        driver: &str,
        units: Option<&String>,
    ) -> redis::Pipeline {
        let hist_key = Self::hist_key(name);
        let info_key = Self::info_key(name);

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
            .ignore()
            .xadd(&hist_key, "1", &[("value", &[1u8])])
            .ignore()
            .xdel(&hist_key, &["1"])
            .ignore()
            .del(&info_key)
            .ignore()
            .hset_multiple(&info_key, &fields)
            .ignore()
            .clone()
    }

    // Builds the low-level command that returns the last value of the
    // device.

    fn last_value_cmd(name: &str) -> redis::Cmd {
        let name = Self::hist_key(name);

        redis::Cmd::xrevrange_count(name, "+", "-", 1usize)
    }

    fn match_pattern_cmd(pattern: Option<&str>) -> redis::Cmd {
        // Take the pattern from the caller and append "#info" since
        // we only want to look at device information keys.

        let pattern = pattern
            .map(Self::info_key)
            .unwrap_or_else(|| String::from("*#info"));

        // Query REDIS to return all keys that match our pattern.

        redis::Cmd::keys(pattern)
    }

    // Builds the low-level command that returns the type of the
    // device's meta record.

    fn info_type_cmd(name: &str) -> redis::Cmd {
        let key = Self::info_key(name);

        redis::cmd("TYPE").arg(&key).clone()
    }

    // Builds the low-level command that returns the type of the
    // device's history record.

    fn hist_type_cmd(name: &str) -> redis::Cmd {
        let key = Self::hist_key(name);

        redis::cmd("TYPE").arg(&key).clone()
    }

    // Creates a redis command pipeline which returns the standard,
    // meta-data for a device.

    fn device_info_cmd(name: &str) -> redis::Cmd {
        let info_key = Self::info_key(name);

        redis::Cmd::hgetall(info_key)
    }

    // Creates the redis command to pull stream information associated
    // with the device's history.

    fn xinfo_cmd(name: &str) -> redis::Cmd {
        let hist_key = Self::hist_key(name);

        redis::Cmd::xinfo_stream(hist_key)
    }

    // Generates a redis command pipeline that adds a value to a
    // device's history.

    fn report_new_value_cmd(key: &str, val: &device::Value) -> redis::Cmd {
        let data = [("value", to_redis(val))];

        redis::Cmd::xadd(key, "*", &data)
    }

    fn report_bounded_new_value_cmd(
        key: &str,
        val: &device::Value,
        mh: usize,
    ) -> redis::Cmd {
        let opts = redis::streams::StreamMaxlen::Approx(mh);
        let data = [("value", to_redis(val))];

        redis::Cmd::xadd_maxlen(key, opts, "*", &data)
    }

    fn hash_to_info(
        st: &SettingTable,
        name: &device::Name,
        hmap: &HashMap<String, String>,
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
                driver: driver.into(),
                total_points: 0,
                first_point: None,
                last_point: None,
            })
        } else {
            Err(Error::NotFound)
        }
    }

    // Converts the `StreamId` type, from redis, into our
    // `device::Reading` type.

    fn stream_id_to_reading(sid: &StreamId) -> Result<device::Reading> {
        if let Some(val) = sid.map.get("value") {
            Ok(device::Reading {
                ts: id_to_ts(sid.id.as_str())?,
                value: from_value(val)?,
            })
        } else {
            Err(Error::TypeError)
        }
    }

    // Looks up a device in the redis store and, if found, returns a
    // `client::DevInfoReply` containing the information.

    async fn lookup_device(
        &mut self,
        name: device::Name,
    ) -> Result<client::DevInfoReply> {
        let info = Self::device_info_cmd(name.to_string().as_str())
            .query_async::<AioMplexConnection, HashMap<String, String>>(
                &mut self.db_con,
            )
            .await
            .map_err(xlat_err)
            .and_then(|v| Self::hash_to_info(&self.table, &name, &v))?;

        Self::xinfo_cmd(name.to_string().as_str())
            .query_async::<AioMplexConnection, StreamInfoStreamReply>(
                &mut self.db_con,
            )
            .await
            .map_err(xlat_err)
            .and_then(move |v| {
                let info = if v.length > 0 {
                    client::DevInfoReply {
                        total_points: v.length as u32,
                        first_point: Some(Self::stream_id_to_reading(
                            &v.first_entry,
                        )?),
                        last_point: Some(Self::stream_id_to_reading(
                            &v.last_entry,
                        )?),
                        ..info
                    }
                } else {
                    client::DevInfoReply {
                        total_points: 0,
                        first_point: None,
                        last_point: None,
                        ..info
                    }
                };

                Ok(info)
            })
    }

    fn parse_last_value(
        name: &str,
        reply: &redis::Value,
    ) -> Option<device::Reading> {
        if redis::Value::Bulk(vec![]) == *reply {
            warn!("no previous value for {}", name);
            return None;
        }

        let data: redis::RedisResult<((
            String,
            HashMap<String, redis::Value>,
        ),)> = redis::from_redis_value(reply);

        match data {
            Ok(((key, m),)) => {
                if let Ok(ts) = id_to_ts(&key) {
                    if let Some(val) = m.get("value") {
                        if let Ok(val) = from_value(val) {
                            return Some(device::Reading { ts, value: val });
                        } else {
                            error!(
                                "last value for {} is in an unknown format",
                                name
                            );
                        }
                    } else {
                        error!(
                            "last value for {} doesn't have a \"value\" field",
                            name
                        );
                    }
                } else {
                    error!("couldn't parse timestamp, {}, for {}", key, name)
                }
            }
            Err(e) => {
                error!(
                    "redis error ({}) when converting last value of {}",
                    e, name
                )
            }
        }
        None
    }

    // Obtains the last value reported for a device, or `None` if
    // there is no history for it.

    async fn last_value(&mut self, name: &str) -> Option<device::Reading> {
        let result: redis::RedisResult<redis::Value> =
            Self::last_value_cmd(name)
                .query_async(&mut self.db_con)
                .await;

        match result {
            Ok(reply) => Self::parse_last_value(name, &reply),
            Err(e) => {
                error!(
                    "redis error ({}) when getting last value of {}",
                    e, name
                );
                None
            }
        }
    }

    // Does some sanity checks on a device to see if it appears to be
    // valid.

    async fn validate_device(&mut self, name: &str) -> Result<()> {
        // This section verifies the device has a NAME#info key that
        // is a hash map.

        {
            let cmd = Self::info_type_cmd(name);
            let result: redis::RedisResult<String> =
                cmd.query_async(&mut self.db_con).await;

            match result {
                Ok(data_type) if data_type.as_str() == "hash" => (),
                Ok(_) => {
                    error!("{} info is of the wrong key type", name);
                    return Err(Error::TypeError);
                }
                Err(_) => {
                    warn!("{} info doesn't exist", name);
                    return Err(Error::NotFound);
                }
            }
        }

        // This section verifies the device has a NAME#hist key that
        // is a time-series stream.

        {
            let cmd = Self::hist_type_cmd(name);
            let result: redis::RedisResult<String> =
                cmd.query_async(&mut self.db_con).await;

            match result {
                Ok(data_type) if data_type.as_str() == "stream" => Ok(()),
                Ok(_) => {
                    error!("{} history is of the wrong key type", name);
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
        &mut self,
        name: &str,
        driver: &str,
        units: Option<&String>,
    ) -> Result<()> {
        debug!("initializing {}", name);
        Self::init_device_cmd(name, driver, units)
            .query_async(&mut self.db_con)
            .await
            .map_err(xlat_err)
    }

    // Creates a closure for a driver to report a device's changing
    // values.

    fn mk_report_func(
        &self,
        name: &str,
        max_history: Option<usize>,
    ) -> ReportReading<device::Value> {
        let db_con = self.db_con.clone();
        let name = String::from(name);

        if let Some(mh) = max_history {
            Box::new(move |v| {
                let mut db_con = db_con.clone();
                let hist_key = Self::hist_key(&name);
                let name = name.clone();

                Box::pin(async move {
                    if let Err(e) =
                        Self::report_bounded_new_value_cmd(&hist_key, &v, mh)
                            .query_async::<AioMplexConnection, ()>(&mut db_con)
                            .await
                    {
                        warn!("couldn't save {} data to redis ... {}", &name, e)
                    }
                })
            })
        } else {
            Box::new(move |v| {
                let mut db_con = db_con.clone();
                let hist_key = Self::hist_key(&name);
                let name = name.clone();

                Box::pin(async move {
                    if let Err(e) = Self::report_new_value_cmd(&hist_key, &v)
                        .query_async::<AioMplexConnection, ()>(&mut db_con)
                        .await
                    {
                        warn!("couldn't save {} data to redis ... {}", &name, e)
                    }
                })
            })
        }
    }
}

#[async_trait]
impl Store for RedisStore {
    /// Registers a device in the redis backend.

    async fn register_read_only_device(
        &mut self,
        driver_name: &str,
        name: &device::Name,
        units: Option<&String>,
        max_history: Option<usize>,
    ) -> Result<(ReportReading<device::Value>, Option<device::Value>)> {
        let name = name.to_string();

        debug!("registering '{}' as read-only", &name);

        if self.validate_device(&name).await.is_err() {
            self.init_device(&name, driver_name, units).await?;

            info!("'{}' has been successfully created", &name);
        }
        Ok((
            self.mk_report_func(&name, max_history),
            self.last_value(&name).await.map(|v| v.value),
        ))
    }

    async fn register_read_write_device(
        &mut self,
        driver_name: &str,
        name: &device::Name,
        units: Option<&String>,
        max_history: Option<usize>,
    ) -> Result<(
        ReportReading<device::Value>,
        RxDeviceSetting,
        Option<device::Value>,
    )> {
        let sname = name.to_string();

        debug!("registering '{}' as read-write", &sname);

        if self.validate_device(&sname).await.is_err() {
            self.init_device(&sname, driver_name, units).await?;

            info!("'{}' has been successfully created", &sname);
        }

        let (tx, rx) = mpsc::channel(20);

        if self.table.insert(name.clone(), tx).is_some() {
            warn!("{} already had a setting channel", &name);
        }

        Ok((
            self.mk_report_func(&sname, max_history),
            rx,
            self.last_value(&sname).await.map(|v| v.value),
        ))
    }

    // Implement the request to pull device information. Any task with
    // a client channel can make this request although the primary
    // client will be from GraphQL requests.

    async fn get_device_info(
        &mut self,
        pattern: Option<&str>,
    ) -> Result<Vec<client::DevInfoReply>> {
        // Get a list of all the keys that match the pattern. For
        // Redis, these keys will have "#info" appended at the end.

        let result: Vec<String> = Self::match_pattern_cmd(pattern)
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

    // This method implements the set_device mutation in the GraphQL
    // API.

    async fn set_device(
        &self,
        name: device::Name,
        value: device::Value,
    ) -> Result<device::Value> {
        if let Some(tx) = self.table.get(&name) {
            let (tx_rpy, rx_rpy) = oneshot::channel();

            // Send the request and return from the function with the
            // reply. If any error occurs during communication, fall
            // through to report it.

            if let Ok(()) = tx.send((value, tx_rpy)).await {
                if let Ok(reply) = rx_rpy.await {
                    return reply;
                }
            }

            // Some portion of the RPC failed. Return an error.

            Err(Error::MissingPeer(
                "cannot communicate with driver".to_string(),
            ))
        } else {
            Err(Error::NotFound)
        }
    }

    async fn get_setting_chan(
        &self,
        name: device::Name,
        _own: bool,
    ) -> Result<TxDeviceSetting> {
        if let Some(tx) = self.table.get(&name) {
            Ok(tx.clone())
        } else {
            Err(Error::NotFound)
        }
    }

    async fn monitor_device(
        &mut self,
        name: device::Name,
        start: Option<DateTime<Utc>>,
        end: Option<DateTime<Utc>>,
    ) -> Result<device::DataStream<device::Reading>> {
        match Self::make_connection(&self.cfg, None, None).await {
            Ok(con) => {
                let name = name.to_string();
                let key = RedisStore::hist_key(&name);

                match (start.map(|v| v.into()), end.map(|v| v.into())) {
                    // With no start time, use the latest value of the
                    // device. If there's an end time, add a stream
                    // combinator that ends the stream once the date
                    // reaches it.
                    (None, end) => {
                        let ts = self
                            .last_value(&name)
                            .await
                            .map(|tmp| st_minus_1us(tmp.ts));

                        // If there's an end date, append a filter to
                        // the stream so it stops once the timestamp
                        // reach it.

                        if let Some(end) = end {
                            let date_test =
                                move |v: &device::Reading| v.ts <= end;

                            Ok(Box::pin(
                                ReadingStream::new(con, &key, ts)
                                    .take_while(date_test),
                            )
                                as device::DataStream<device::Reading>)
                        } else {
                            Ok(Box::pin(ReadingStream::new(con, &key, ts))
                                as device::DataStream<device::Reading>)
                        }
                    }

                    // Given a start time with no end time, start
                    // reading the redis stream at that point.
                    (Some(start), None) => Ok(Box::pin(ReadingStream::new(
                        con,
                        &key,
                        Some(st_minus_1us(start)),
                    ))
                        as device::DataStream<device::Reading>),

                    // Start reading at the start time and stop the
                    // stream at the end time.
                    (Some(start_tmp), Some(end_tmp)) => {
                        let start = std::cmp::min(start_tmp, end_tmp);
                        let end = std::cmp::max(start_tmp, end_tmp);
                        let date_test = move |v: &device::Reading| v.ts <= end;

                        Ok(Box::pin(
                            ReadingStream::new(
                                con,
                                &key,
                                Some(st_minus_1us(start)),
                            )
                            .take_while(date_test),
                        )
                            as device::DataStream<device::Reading>)
                    }
                }
            }
            Err(e) => {
                error!("couldn't make a connection : {}", e);

                Ok(Box::pin(tokio_stream::empty())
                    as device::DataStream<device::Reading>)
            }
        }
    }
}

pub async fn open(cfg: &config::Config) -> Result<impl Store> {
    RedisStore::new(cfg, None, None)
        .instrument(
            info_span!("redis-db", addr=?cfg.get_addr(), db=cfg.get_dbn()),
        )
        .await
}

// This is the test module to make sure the redis backend works
// correctly. We're not testing redis itself -- we assume the redis
// project is verifying its behavior. Many functions have been broken
// out into smaller, helper functions so that we can create tests
// for them without the need of a redis installation.
//
// That being said, there are some requirements which are hard to test
// because they're dependent on redis' behavior.
//
// For instance, when monitoring a device, we want to immediately
// return any "current" value before blocking for future values. If
// there is a "last value", we need to use its timestamp for the next,
// blocking call. We should test this, but it doesn't seem like an
// easy thing to do (without requiring a redis instance.)

#[cfg(test)]
mod tests {
    use super::*;
    use drmem_api::device;

    // We only want to convert Value::Data() forms. These tests make
    // sure the other variants don't translate.

    #[test]
    fn test_reject_invalid_forms() {
        if let Ok(v) = from_value(&redis::Value::Int(0)) {
            panic!("Value::Int incorrectly translated to {:?}", v);
        }
        if let Ok(v) = from_value(&redis::Value::Bulk(vec![])) {
            panic!("Value::Bulk incorrectly translated to {:?}", v);
        }
        if let Ok(v) = from_value(&redis::Value::Status(String::from(""))) {
            panic!("Value::Status incorrectly translated to {:?}", v);
        }
        if let Ok(v) = from_value(&redis::Value::Okay) {
            panic!("Value::Okay incorrectly translated to {:?}", v);
        }
    }

    // Test correct decoding of device::Value::Bool values.

    #[test]
    fn test_bool_decoder() {
        assert_eq!(
            Ok(device::Value::Bool(false)),
            from_value(&redis::Value::Data(vec![b'B', b'F']))
        );
        assert_eq!(
            Ok(device::Value::Bool(true)),
            from_value(&redis::Value::Data(vec![b'B', b'T']))
        );
    }

    // Test correct encoding of device::Value::Bool values.

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

    // Test correct encoding of device::Value::Int values.

    #[test]
    fn test_int_encoder() {
        for (v, rv) in INT_TEST_CASES {
            assert_eq!(*rv, to_redis(&device::Value::Int(*v)));
        }
    }

    // Test correct decoding of device::Value::Int values.

    #[test]
    fn test_int_decoder() {
        assert!(from_value(&redis::Value::Data(vec![])).is_err());
        assert!(from_value(&redis::Value::Data(vec![b'I'])).is_err());
        assert!(from_value(&redis::Value::Data(vec![b'I', 0u8])).is_err());
        assert!(from_value(&redis::Value::Data(vec![b'I', 0u8, 0u8])).is_err());
        assert!(
            from_value(&redis::Value::Data(vec![b'I', 0u8, 0u8, 0u8])).is_err()
        );

        for (v, rv) in INT_TEST_CASES {
            let data = redis::Value::Data(rv.to_vec());

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

    // Test correct encoding of device::Value::Flt values.

    #[test]
    fn test_float_encoder() {
        for (v, rv) in FLT_TEST_CASES {
            assert_eq!(*rv, to_redis(&device::Value::Flt(*v)));
        }
    }

    // Test correct decoding of device::Value::Flt values.

    #[test]
    fn test_float_decoder() {
        assert!(from_value(&redis::Value::Data(vec![])).is_err());
        assert!(from_value(&redis::Value::Data(vec![b'D'])).is_err());
        assert!(from_value(&redis::Value::Data(vec![b'D', 0u8])).is_err());
        assert!(from_value(&redis::Value::Data(vec![b'D', 0u8, 0u8])).is_err());
        assert!(
            from_value(&redis::Value::Data(vec![b'D', 0u8, 0u8, 0u8])).is_err()
        );
        assert!(from_value(&redis::Value::Data(vec![
            b'D', 0u8, 0u8, 0u8, 0u8
        ]))
        .is_err());
        assert!(from_value(&redis::Value::Data(vec![
            b'D', 0u8, 0u8, 0u8, 0u8, 0u8
        ]))
        .is_err());
        assert!(from_value(&redis::Value::Data(vec![
            b'D', 0u8, 0u8, 0u8, 0u8, 0u8, 0u8
        ]))
        .is_err());
        assert!(from_value(&redis::Value::Data(vec![
            b'D', 0u8, 0u8, 0u8, 0u8, 0u8, 0u8, 0u8
        ]))
        .is_err());

        for (v, rv) in FLT_TEST_CASES {
            let data = redis::Value::Data(rv.to_vec());

            assert_eq!(Ok(device::Value::Flt(*v)), from_value(&data));
        }
    }

    const COLOR_TEST_CASES: &[((u8, u8, u8), [u8; 4])] = &[
        ((0, 0, 0), [b'C', 0, 0, 0]),
        ((4, 2, 1), [b'C', 4, 2, 1]),
        ((8, 4, 2), [b'C', 8, 4, 2]),
        ((12, 6, 3), [b'C', 12, 6, 3]),
        ((16, 8, 4), [b'C', 16, 8, 4]),
        ((20, 10, 5), [b'C', 20, 10, 5]),
        ((24, 12, 6), [b'C', 24, 12, 6]),
        ((28, 14, 7), [b'C', 28, 14, 7]),
        ((32, 16, 8), [b'C', 32, 16, 8]),
    ];

    #[test]
    fn test_color_encoder() {
        for ((r, g, b), rv) in COLOR_TEST_CASES {
            assert_eq!(
                &rv[..],
                to_redis(&device::Value::Color(palette::LinSrgb::new(
                    *r, *g, *b
                )))
            );
        }
    }

    #[test]
    fn test_color_decoder() {
        for ((r, g, b), rv) in COLOR_TEST_CASES {
            assert_eq!(
                from_value(&redis::Value::Data(rv.to_vec())).unwrap(),
                device::Value::Color(palette::LinSrgb::new(*r, *g, *b))
            );
        }
    }

    const STR_TEST_CASES: &[(&str, &[u8])] = &[
        ("", &[b'S', 0u8, 0u8, 0u8, 0u8]),
        ("ABC", &[b'S', 0u8, 0u8, 0u8, 3u8, b'A', b'B', b'C']),
    ];

    // Test correct encoding of device::Value::Str values.

    #[test]
    fn test_string_encoder() {
        for (v, rv) in STR_TEST_CASES {
            assert_eq!(*rv, to_redis(&device::Value::Str(String::from(*v))));
        }
    }

    // Test correct decoding of device::Value::Str values.

    #[test]
    fn test_string_decoder() {
        // Buffers smaller than 5 bytes are an error.

        assert!(from_value(&redis::Value::Data(vec![])).is_err());
        assert!(from_value(&redis::Value::Data(vec![b'S'])).is_err());
        assert!(from_value(&redis::Value::Data(vec![b'S', 0u8])).is_err());
        assert!(from_value(&redis::Value::Data(vec![b'S', 0u8, 0u8])).is_err());
        assert!(
            from_value(&redis::Value::Data(vec![b'S', 0u8, 0u8, 0u8])).is_err()
        );

        // Loop through the test cases.

        for (v, rv) in STR_TEST_CASES {
            let data = redis::Value::Data(rv.to_vec());

            assert_eq!(
                Ok(device::Value::Str(String::from(*v))),
                from_value(&data)
            );
        }

        // Verify proper response (both good and bad) when the buffer
        // doesn't match the size of the string.

        assert!(from_value(&redis::Value::Data(vec![
            b'S', 0u8, 0u8, 0u8, 1u8
        ]))
        .is_err());
        assert!(from_value(&redis::Value::Data(vec![
            b'S', 0u8, 0u8, 0u8, 2u8, b'A'
        ]))
        .is_err());
        assert_eq!(
            Ok(device::Value::Str(String::from("AB"))),
            from_value(&redis::Value::Data(vec![
                b'S', 0u8, 0u8, 0u8, 2u8, b'A', b'B', 0, 0
            ]))
        );
    }

    #[test]
    fn test_pattern_cmd() {
        assert_eq!(
            &RedisStore::match_pattern_cmd(None).get_packed_command(),
            b"*2\r
$4\r\nKEYS\r
$6\r\n*#info\r\n"
        );
        assert_eq!(
            &RedisStore::match_pattern_cmd(Some("device")).get_packed_command(),
            b"*2\r
$4\r\nKEYS\r
$11\r\ndevice#info\r\n"
        );
        assert_eq!(
            &RedisStore::match_pattern_cmd(Some("*weather*"))
                .get_packed_command(),
            b"*2\r
$4\r\nKEYS\r
$14\r\n*weather*#info\r\n"
        );
    }

    #[test]
    fn test_ts_to_id() {
        let dur = time::Duration::from_secs(1000);
        let ts = time::UNIX_EPOCH + dur;

        assert_eq!(ReadingStream::ts_to_id(ts), "1000000-0");

        let dur = time::Duration::from_micros(1234567);
        let ts = time::UNIX_EPOCH + dur;

        assert_eq!(ReadingStream::ts_to_id(ts), "1234-567");
    }

    #[test]
    fn test_read_next_cmd() {
        let cmd = ReadingStream::read_next_cmd("device#hist", "$");

        assert_eq!(
            &cmd.get_packed_command(),
            b"*8\r
$5\r\nXREAD\r
$5\r\nBLOCK\r
$4\r\n5000\r
$5\r\nCOUNT\r
$1\r\n1\r
$7\r\nSTREAMS\r
$11\r\ndevice#hist\r
$1\r\n$\r\n"
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
            &cmd.get_packed_command(),
            b"*2\r
$7\r\nHGETALL\r
$11\r\ndevice#info\r\n"
        );
    }

    #[test]
    fn test_last_value_cmd() {
        let pipe = RedisStore::last_value_cmd("device");

        assert_eq!(
            &pipe.get_packed_command(),
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
    fn test_parsing_last_value() {
        const NAME: &str = "device";

        assert_eq!(
            RedisStore::parse_last_value(NAME, &redis::Value::Nil),
            None
        );
        assert_eq!(
            RedisStore::parse_last_value(NAME, &redis::Value::Bulk(vec![])),
            None
        );

        let val = redis::Value::Bulk(vec![redis::Value::Bulk(vec![
            redis::Value::Data(b"1000000-0".to_vec()),
            redis::Value::Bulk(vec![
                redis::Value::Data(b"value".to_vec()),
                redis::Value::Data(b"BT".to_vec()),
            ]),
        ])]);

        assert_eq!(
            RedisStore::parse_last_value(NAME, &val),
            Some(device::Reading {
                ts: time::UNIX_EPOCH + time::Duration::from_secs(1000),
                value: device::Value::Bool(true)
            })
        );

        let val = redis::Value::Bulk(vec![redis::Value::Bulk(vec![
            redis::Value::Data(b"1234-567".to_vec()),
            redis::Value::Bulk(vec![
                redis::Value::Data(b"value".to_vec()),
                redis::Value::Data(b"BF".to_vec()),
            ]),
        ])]);

        assert_eq!(
            RedisStore::parse_last_value(NAME, &val),
            Some(device::Reading {
                ts: time::UNIX_EPOCH + time::Duration::from_micros(1234567),
                value: device::Value::Bool(false)
            })
        );
    }

    #[test]
    fn test_xinfo_cmd() {
        assert_eq!(
            &RedisStore::xinfo_cmd("junk").get_packed_command(),
            b"*3\r
$5\r\nXINFO\r
$6\r\nSTREAM\r
$9\r\njunk#hist\r\n"
        );
    }

    #[test]
    fn test_report_value_cmd() {
        assert_eq!(
            &RedisStore::report_new_value_cmd("key", &(true.into()))
                .get_packed_command(),
            b"*5\r
$4\r\nXADD\r
$3\r\nkey\r
$1\r\n*\r
$5\r\nvalue\r
$2\r\nBT\r\n"
        );
        assert_eq!(
            &RedisStore::report_new_value_cmd("key", &(0x00010203i32.into()))
                .get_packed_command(),
            b"*5\r
$4\r\nXADD\r
$3\r\nkey\r
$1\r\n*\r
$5\r\nvalue\r
$5\r\nI\x00\x01\x02\x03\r\n"
        );
        assert_eq!(
            &RedisStore::report_new_value_cmd("key", &(0x12345678i32.into()))
                .get_packed_command(),
            b"*5\r
$4\r\nXADD\r
$3\r\nkey\r
$1\r\n*\r
$5\r\nvalue\r
$5\r\nI\x12\x34\x56\x78\r\n"
        );
        assert_eq!(
            &RedisStore::report_new_value_cmd("key", &(1.0.into()))
                .get_packed_command(),
            b"*5\r
$4\r\nXADD\r
$3\r\nkey\r
$1\r\n*\r
$5\r\nvalue\r
$9\r\nD\x3f\xf0\x00\x00\x00\x00\x00\x00\r\n"
        );
        assert_eq!(
            &RedisStore::report_new_value_cmd("key", &("hello".into()))
                .get_packed_command(),
            b"*5\r
$4\r\nXADD\r
$3\r\nkey\r
$1\r\n*\r
$5\r\nvalue\r
$10\r\nS\x00\x00\x00\x05hello\r\n"
        );

        assert_eq!(
            &RedisStore::report_bounded_new_value_cmd("key", &(true.into()), 0)
                .get_packed_command(),
            b"*8\r
$4\r\nXADD\r
$3\r\nkey\r
$6\r\nMAXLEN\r
$1\r\n~\r
$1\r\n0\r
$1\r\n*\r
$5\r\nvalue\r
$2\r\nBT\r\n"
        );
        assert_eq!(
            &RedisStore::report_bounded_new_value_cmd(
                "key",
                &(0x00010203i32.into()),
                1
            )
            .get_packed_command(),
            b"*8\r
$4\r\nXADD\r
$3\r\nkey\r
$6\r\nMAXLEN\r
$1\r\n~\r
$1\r\n1\r
$1\r\n*\r
$5\r\nvalue\r
$5\r\nI\x00\x01\x02\x03\r\n"
        );
        assert_eq!(
            &RedisStore::report_bounded_new_value_cmd(
                "key",
                &(0x12345678i32.into()),
                2
            )
            .get_packed_command(),
            b"*8\r
$4\r\nXADD\r
$3\r\nkey\r
$6\r\nMAXLEN\r
$1\r\n~\r
$1\r\n2\r
$1\r\n*\r
$5\r\nvalue\r
$5\r\nI\x12\x34\x56\x78\r\n"
        );
        assert_eq!(
            &RedisStore::report_bounded_new_value_cmd("key", &(1.0.into()), 3)
                .get_packed_command(),
            b"*8\r
$4\r\nXADD\r
$3\r\nkey\r
$6\r\nMAXLEN\r
$1\r\n~\r
$1\r\n3\r
$1\r\n*\r
$5\r\nvalue\r
$9\r\nD\x3f\xf0\x00\x00\x00\x00\x00\x00\r\n"
        );
        assert_eq!(
            &RedisStore::report_bounded_new_value_cmd(
                "key",
                &("hello".into()),
                4
            )
            .get_packed_command(),
            b"*8\r
$4\r\nXADD\r
$3\r\nkey\r
$6\r\nMAXLEN\r
$1\r\n~\r
$1\r\n4\r
$1\r\n*\r
$5\r\nvalue\r
$10\r\nS\x00\x00\x00\x05hello\r\n"
        );
    }

    #[test]
    fn test_init_dev() {
        assert_eq!(
            String::from_utf8_lossy(
                &RedisStore::init_device_cmd("device", "mem", None)
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
                    Some(&String::from("gpm"))
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
    fn test_streamid_to_reading() {
        // Look for various failure modes.

        assert!(RedisStore::stream_id_to_reading(&StreamId {
            id: "1000-0".into(),
            map: HashMap::from([])
        })
        .is_err());
        assert!(RedisStore::stream_id_to_reading(&StreamId {
            id: "1000-0".into(),
            map: HashMap::from([(
                "junk".into(),
                redis::Value::Data(b"10".to_vec())
            )])
        })
        .is_err());
        assert!(RedisStore::stream_id_to_reading(&StreamId {
            id: "1000-0".into(),
            map: HashMap::from([(
                "value".into(),
                redis::Value::Data(b"10".to_vec())
            )])
        })
        .is_err());

        // Look for valid conversions.

        assert_eq!(
            RedisStore::stream_id_to_reading(&StreamId {
                id: "1000-0".into(),
                map: HashMap::from([(
                    "value".into(),
                    redis::Value::Data(to_redis(&device::Value::Bool(true)))
                )])
            }),
            Ok(device::Reading {
                ts: time::UNIX_EPOCH + time::Duration::from_millis(1000),
                value: device::Value::Bool(true)
            })
        );
        assert_eq!(
            RedisStore::stream_id_to_reading(&StreamId {
                id: "1500-0".into(),
                map: HashMap::from([(
                    "value".into(),
                    redis::Value::Data(to_redis(&device::Value::Int(123)))
                )])
            }),
            Ok(device::Reading {
                ts: time::UNIX_EPOCH + time::Duration::from_millis(1500),
                value: device::Value::Int(123)
            })
        );
        assert_eq!(
            RedisStore::stream_id_to_reading(&StreamId {
                id: "2500-0".into(),
                map: HashMap::from([(
                    "value".into(),
                    redis::Value::Data(to_redis(&device::Value::Int(-321)))
                )])
            }),
            Ok(device::Reading {
                ts: time::UNIX_EPOCH + time::Duration::from_millis(2500),
                value: device::Value::Int(-321)
            })
        );
        assert_eq!(
            RedisStore::stream_id_to_reading(&StreamId {
                id: "2500-0".into(),
                map: HashMap::from([(
                    "value".into(),
                    redis::Value::Data(to_redis(&device::Value::Flt(1.0)))
                )])
            }),
            Ok(device::Reading {
                ts: time::UNIX_EPOCH + time::Duration::from_millis(2500),
                value: device::Value::Flt(1.0)
            })
        );
        assert_eq!(
            RedisStore::stream_id_to_reading(&StreamId {
                id: "2500-0".into(),
                map: HashMap::from([(
                    "value".into(),
                    redis::Value::Data(to_redis(&device::Value::Flt(-1.0)))
                )])
            }),
            Ok(device::Reading {
                ts: time::UNIX_EPOCH + time::Duration::from_millis(2500),
                value: device::Value::Flt(-1.0)
            })
        );
        assert_eq!(
            RedisStore::stream_id_to_reading(&StreamId {
                id: "2500-0".into(),
                map: HashMap::from([(
                    "value".into(),
                    redis::Value::Data(to_redis(&device::Value::Flt(1.0e100)))
                )])
            }),
            Ok(device::Reading {
                ts: time::UNIX_EPOCH + time::Duration::from_millis(2500),
                value: device::Value::Flt(1.0e100)
            })
        );
        assert_eq!(
            RedisStore::stream_id_to_reading(&StreamId {
                id: "2500-0".into(),
                map: HashMap::from([(
                    "value".into(),
                    redis::Value::Data(to_redis(&device::Value::Flt(1.0e-100)))
                )])
            }),
            Ok(device::Reading {
                ts: time::UNIX_EPOCH + time::Duration::from_millis(2500),
                value: device::Value::Flt(1.0e-100)
            })
        );
        assert_eq!(
            RedisStore::stream_id_to_reading(&StreamId {
                id: "2500-0".into(),
                map: HashMap::from([(
                    "value".into(),
                    redis::Value::Data(to_redis(&device::Value::Str(
                        "Hello".into()
                    )))
                )])
            }),
            Ok(device::Reading {
                ts: time::UNIX_EPOCH + time::Duration::from_millis(2500),
                value: device::Value::Str("Hello".into())
            })
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
                driver: "*missing*".into(),
                total_points: 0,
                first_point: None,
                last_point: None,
            })
        );

        let _ = fm.insert("driver".to_string(), "sump".to_string().into());

        assert_eq!(
            RedisStore::hash_to_info(&st, &device, &fm),
            Ok(client::DevInfoReply {
                name: device.clone(),
                units: Some(String::from("gpm")),
                settable: false,
                driver: "sump".into(),
                total_points: 0,
                first_point: None,
                last_point: None,
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
                driver: "sump".into(),
                total_points: 0,
                first_point: None,
                last_point: None,
            })
        );
    }
}

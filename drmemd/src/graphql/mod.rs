use crate::config::Logic;
use async_graphql::{
    Context, Error, InputObject, Object, Result, SimpleObject, Subscription,
};
use async_graphql_axum::GraphQLSubscription;
use axum::{
    extract::{Query as AxumQuery, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
    Extension, Router,
};
#[cfg(feature = "graphiql")]
use axum::{response::Html, routing::get};
use chrono::prelude::*;
use drmem_api::{
    client, device,
    driver::{self, Reporter},
};
use futures::Future;
use libmdns::Responder;
use std::{sync::Arc, time::Duration};
use tower_http::{compression::CompressionLayer, cors::CorsLayer};
use tracing::{debug, error, info, info_span, Instrument};

pub mod config;

// The Context parameter for Queries - holds references to driver database,
// client request channel, and logic blocks configuration.

#[derive(Clone)]
struct ConfigDb<R: Reporter>(
    crate::driver::DriverDb<R>,
    client::RequestChan,
    Vec<Arc<LogicBlock<R>>>,
);

// `DriverInfo` is an object that can be returned by a GraphQL
// query. It contains information related to drivers that are
// available in the DrMem environment (executable.)

struct DriverInfo<R: Reporter> {
    name: driver::Name,
    summary: &'static str,
    description: &'static str,
    _phant: std::marker::PhantomData<R>,
}

#[Object]
impl<R: Reporter> DriverInfo<R> {
    /// The name of the driver.
    async fn name(&self) -> driver::Name {
        self.name.clone()
    }

    /// A short summary of the driver's purpose.
    async fn summary(&self) -> &str {
        self.summary
    }

    /// Detailed information about the driver: the configuration parameters;
    /// the devices it registers; and other pertinent information. This
    /// information is formatted in Markdown.
    async fn description(&self) -> &str {
        self.description
    }
}

#[derive(InputObject)]
#[graphql(name = "SettingData")]
/// Describes data that can be sent to devices. When specifying data, one --
/// and only one -- field must be set.
struct SettingData {
    #[graphql(name = "int")]
    /// Placeholder for integer values.
    f_int: Option<i32>,
    #[graphql(name = "flt")]
    /// Placeholder for float values.
    f_float: Option<f64>,
    #[graphql(name = "bool")]
    /// Placeholder for boolean values.
    f_bool: Option<bool>,
    #[graphql(name = "str")]
    /// Placeholder for string values.
    f_string: Option<String>,
    #[graphql(name = "color")]
    /// Placeholder for color values.
    f_color: Option<Vec<i32>>,
}

// Contains information about a device's history in the backend.

#[derive(SimpleObject)]
/// Contains information about a device's history, as currently stored in the
/// backend. This information is a snapshot from when it was obtained.
/// Depending on how frequently a device gets updated, this information may be
/// obsolete in a short time.
struct DeviceHistory {
    /// Total number of points in backend storage.
    total_points: i32,
    /// The oldest data point in storage. If the total is 0, then this field
    /// will be null. Note that this value is accurate at the time of this
    /// query. However, at any moment, the oldest data point could be thrown
    /// away if new data arrives.
    first_point: Option<Reading>,
    /// The latest data point in storage. If the total is 0, then this field
    /// will be null. Note that this value is accurate at the time of this
    /// query. However, at any moment, newer data could be added.
    last_point: Option<Reading>,
}

// `DeviceInfo` is a GraphQL object which contains information about a device.

struct DeviceInfo<R: Reporter> {
    device_name: String,
    units: Option<String>,
    settable: bool,
    driver_name: driver::Name,
    history: DeviceHistory,
    db: crate::driver::DriverDb<R>,
}

#[Object]
impl<R: Reporter> DeviceInfo<R> {
    /// The name of the device.
    async fn device_name(&self) -> &str {
        &self.device_name
    }

    /// The engineering units of the device's value.
    async fn units(&self) -> Option<&String> {
        self.units.as_ref()
    }

    /// Indicates whether the device is read-only or can be controlled.
    async fn settable(&self) -> bool {
        self.settable
    }

    /// Information about the driver that implements this device.
    async fn driver(&self) -> DriverInfo<R> {
        self.db
            .get_driver(&self.driver_name)
            .map(|di| DriverInfo {
                name: self.driver_name.clone(),
                summary: di.0,
                description: di.1,
                _phant: std::marker::PhantomData,
            })
            .unwrap()
    }

    async fn history(&self) -> &DeviceHistory {
        &self.history
    }
}

struct LogicBlockVariable<R: Reporter> {
    name: String,
    device: String,
    _phant: std::marker::PhantomData<R>,
}

#[Object]
impl<R: Reporter> LogicBlockVariable<R> {
    /// The name of the variable.
    async fn name(&self) -> &str {
        &self.name
    }

    /// The name of the device.
    async fn device(&self) -> &str {
        &self.device
    }
}

struct LogicBlockExpression<R: Reporter> {
    name: String,
    expr: String,
    _phant: std::marker::PhantomData<R>,
}

#[Object]
impl<R: Reporter> LogicBlockExpression<R> {
    /// The name of the definition.
    async fn name(&self) -> &str {
        &self.name
    }

    /// The expression.
    async fn expr(&self) -> &str {
        &self.expr
    }
}

struct LogicBlock<R: Reporter> {
    name: Arc<str>,
    description: String,
    inputs: Vec<LogicBlockVariable<R>>,
    outputs: Vec<LogicBlockVariable<R>>,
    defs: Vec<LogicBlockExpression<R>>,
    expr: Vec<String>,
}

#[Object]
impl<R: Reporter> LogicBlock<R> {
    /// The name of the logic block.
    async fn name(&self) -> &str {
        &self.name
    }

    /// A description of the logic block's purpose.
    async fn description(&self) -> &str {
        &self.description
    }

    /// The inputs needed by the logic block.
    async fn inputs(&self) -> &[LogicBlockVariable<R>] {
        &self.inputs
    }

    /// The outputs controlled by the logic block.
    async fn outputs(&self) -> &[LogicBlockVariable<R>] {
        &self.outputs
    }

    /// Shared expressions used by the logic block.
    async fn defs(&self) -> &[LogicBlockExpression<R>] {
        &self.defs
    }

    /// Control expressions used by the logic block.
    async fn expr(&self) -> &[String] {
        &self.expr
    }
}

// This defines the top-level Query API.

struct Query<R: Reporter>(std::marker::PhantomData<R>);

impl<R: Reporter> Query<R> {
    // These helper functions are used by a call to `Iterator::filter`
    // to select a set of devices.

    fn is_settable(e: &&client::DevInfoReply) -> bool {
        e.settable
    }

    fn is_not_settable(e: &&client::DevInfoReply) -> bool {
        !e.settable
    }

    fn is_true(_e: &&client::DevInfoReply) -> bool {
        true
    }

    // This method returns a closure that can be used with
    // `Iterator<Item = Arc<LogicBlock>>::filter`.

    fn logic_block_filter(
        name: Option<String>,
        devices: Option<Vec<String>>,
    ) -> impl FnMut(&Arc<LogicBlock<R>>) -> bool {
        move |lb: &Arc<LogicBlock<R>>| {
            // If a name was specified, return `false` if the current
            // LogicBlock doesn't have that name. If it has the name, we
            // still need to see if the device name filter further
            // restricts the results.

            if let Some(ref name) = name {
                if *name != *lb.name {
                    return false;
                }
            }

            // If a list of device names was specified, look through
            // the inputs and outputs to see if any devices match any
            // in the list.

            if let Some(ref devices) = devices {
                for ins in lb.inputs.iter() {
                    if devices.iter().any(|v| v == &ins.device) {
                        return true;
                    }
                }
                for outs in lb.outputs.iter() {
                    if devices.iter().any(|v| v == &outs.device) {
                        return true;
                    }
                }
                return false;
            }

            // If neither filter was given or the name filter matched,
            // then return `true` to keep the current entry in the
            // results.

            true
        }
    }
}

#[Object]
impl<R: Reporter + Clone> Query<R> {
    /// Returns logic blocks configured in the node. By default, all logic
    /// blocks are returned. If either parameter is given, the results are
    /// filtered to only return information that matches the selection values.
    async fn logic_blocks(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "If provided, only the logic block with the specified name will be returned."
        )]
        sel_name: Option<String>,
        #[graphql(
            desc = "This parameter can specify a list of device names. Only logic blocks that use any of the devices in either input or output will be included in the results."
        )]
        sel_devices: Option<Vec<String>>,
    ) -> Result<Vec<Arc<LogicBlock<R>>>> {
        let db = ctx.data::<ConfigDb<R>>()?;
        Ok(db
            .2
            .iter()
            .cloned()
            .filter(Self::logic_block_filter(sel_name, sel_devices))
            .collect())
    }

    /// Returns information about the available drivers in the running
    /// instance of `drmemd`. If `name` isn't provided, an array of all driver
    /// information is returned. If `name` is specified and a driver with that
    /// name exists, a single element array is returned. Otherwise an error is
    /// returned.
    async fn driver_info(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "An optional argument which, when provided, only returns driver information whose name matches. If this argument isn't provided, every driver's information will be returned."
        )]
        name: Option<String>,
    ) -> Result<Vec<DriverInfo<R>>> {
        let db = ctx.data::<ConfigDb<R>>()?;
        if let Some(name) = name {
            if let Some((n, s, d)) = db.0.find(&name) {
                Ok(vec![DriverInfo {
                    name: n,
                    summary: s,
                    description: d,
                    _phant: std::marker::PhantomData,
                }])
            } else {
                Err(Error::new(format!("driver not found: {}", name)))
            }
        } else {
            let result =
                db.0.get_all()
                    .map(|(n, s, d)| DriverInfo {
                        name: n,
                        summary: s,
                        description: d,
                        _phant: std::marker::PhantomData,
                    })
                    .collect();

            Ok(result)
        }
    }

    /// Returns information associated with the devices that are active in the
    /// running system. Arguments to the query will filter the results.
    ///
    /// If the argument `pattern` is provided, only the devices whose name
    /// matches the pattern will be included in the results. The pattern
    /// follows the shell "glob" style.
    ///
    /// If the argument `settable` is provided, it returns devices that are or
    /// aren't settable, depending on the value of the argument.
    async fn device_info(
        &self,
        ctx: &Context<'_>,
        #[graphql(
            desc = "If this argument is provided, the query returns information for devices whose name matches the pattern. The pattern uses 'globbing' grammar: '?' matches one character, '*' matches zero or more, '**' matches arbitrary levels of the path (between ':'s)."
        )]
        pattern: Option<String>,
        #[graphql(
            desc = "If this argument is provided, the query filters the result based on whether the device can be set or not."
        )]
        settable: Option<bool>,
    ) -> Result<Vec<DeviceInfo<R>>> {
        let db = ctx.data::<ConfigDb<R>>()?;
        let tx = db.1.clone();
        let filt = settable
            .map(|v| {
                if v {
                    Query::<R>::is_settable
                } else {
                    Query::<R>::is_not_settable
                }
            })
            .unwrap_or(Query::<R>::is_true);

        tx.get_device_info(pattern)
            .await
            .map(|v| {
                v.iter()
                    .filter(filt)
                    .map(|e| DeviceInfo {
                        device_name: e.name.to_string(),
                        units: e.units.clone(),
                        settable: e.settable,
                        driver_name: e.driver.clone(),
                        history: DeviceHistory {
                            total_points: e.total_points as i32,
                            first_point: e.first_point.as_ref().map(|v| {
                                Reading {
                                    device: e.name.to_string(),
                                    ..v.into()
                                }
                            }),
                            last_point: e.last_point.as_ref().map(|v| {
                                Reading {
                                    device: e.name.to_string(),
                                    ..v.into()
                                }
                            }),
                        },
                        db: db.0.clone(),
                    })
                    .collect()
            })
            .map_err(|_| Error::new("error looking-up device"))
    }
}

// The `Mutation` type is used to group queries that attempt to
// control devices by sending them settings.

struct Mutation<R: Reporter>(std::marker::PhantomData<R>);

impl<R: Reporter> Mutation<R> {
    // Sends a new value to a device.

    async fn perform_setting<
        T: Into<device::Value> + TryFrom<device::Value, Error = drmem_api::Error>,
    >(
        db: &ConfigDb<R>,
        device: &str,
        value: T,
    ) -> Result<T> {
        // Make sure the device name is properly formed.

        if let Ok(name) = device.try_into() {
            let tx = db.1.clone();

            // Send the setting to the driver. Map the error, if any,
            // to an async-graphql `Error` type.

            tx.set_device::<T>(name, value)
                .await
                .map_err(|e| Error::new(format!("error making setting: {}", e)))
        } else {
            Err(Error::new("badly formed device name"))
        }
    }

    // Helper function which returns a closure that converts a
    // boolean value to a `Reading` type.

    fn bool_to_reading(name: String) -> impl FnOnce(bool) -> Reading {
        |v| Reading {
            device: name,
            stamp: Utc::now(),
            int_value: None,
            float_value: None,
            bool_value: Some(v),
            string_value: None,
            color_value: None,
        }
    }

    // Helper function which returns a closure that converts an
    // integer to a `Reading` type.

    fn int_to_reading(name: String) -> impl FnOnce(i32) -> Reading {
        |v| Reading {
            device: name,
            stamp: Utc::now(),
            int_value: Some(v),
            float_value: None,
            bool_value: None,
            string_value: None,
            color_value: None,
        }
    }

    // Helper function which returns a closure that converts a
    // floating point value to a `Reading` type.

    fn flt_to_reading(name: String) -> impl FnOnce(f64) -> Reading {
        |v| Reading {
            device: name,
            stamp: Utc::now(),
            int_value: None,
            float_value: Some(v),
            bool_value: None,
            string_value: None,
            color_value: None,
        }
    }

    // Helper function which returns a closure that converts a
    // string value to a `Reading` type.

    fn str_to_reading(name: String) -> impl FnOnce(String) -> Reading {
        |v| Reading {
            device: name,
            stamp: Utc::now(),
            int_value: None,
            float_value: None,
            bool_value: None,
            string_value: Some(v.into()),
            color_value: None,
        }
    }

    // Helper function which returns a closure that converts a color
    // value to a `Reading` type.

    fn color_to_reading(
        name: String,
    ) -> impl FnOnce(palette::LinSrgba<u8>) -> Reading {
        |v| Reading {
            device: name,
            stamp: Utc::now(),
            int_value: None,
            float_value: None,
            bool_value: None,
            string_value: None,
            color_value: Some(if v.alpha == 255 {
                vec![v.red as i32, v.green as i32, v.blue as i32]
            } else {
                vec![
                    v.red as i32,
                    v.green as i32,
                    v.blue as i32,
                    v.alpha as i32,
                ]
            }),
        }
    }
}

#[Object]
impl<R: Reporter> Mutation<R> {
    /// Submits `value` to be applied to the device associated with the given
    /// `name`. If the data is in a format the device doesn't support an error
    /// is returned. The `value` parameter contains several fields. Only one
    /// should be set. It is an error to have all fields `null` or more than
    /// one field non-`null`.
    async fn set_device(
        &self,
        ctx: &Context<'_>,
        name: String,
        value: SettingData,
    ) -> Result<Reading> {
        let db = ctx.data::<ConfigDb<R>>()?;
        match value {
            SettingData {
                f_int: None,
                f_float: None,
                f_bool: None,
                f_string: None,
                f_color: None,
            } => Err(Error::new("no data provided")),

            SettingData {
                f_int: Some(v),
                f_float: None,
                f_bool: None,
                f_string: None,
                f_color: None,
            } => Mutation::perform_setting(db, &name, v)
                .await
                .map(Mutation::<R>::int_to_reading(name)),

            SettingData {
                f_int: None,
                f_float: Some(v),
                f_bool: None,
                f_string: None,
                f_color: None,
            } => Mutation::perform_setting(db, &name, v)
                .await
                .map(Mutation::<R>::flt_to_reading(name)),

            SettingData {
                f_int: None,
                f_float: None,
                f_bool: Some(v),
                f_string: None,
                f_color: None,
            } => Mutation::perform_setting(db, &name, v)
                .await
                .map(Mutation::<R>::bool_to_reading(name)),

            SettingData {
                f_int: None,
                f_float: None,
                f_bool: None,
                f_string: Some(v),
                f_color: None,
            } => Mutation::perform_setting(db, &name, v)
                .await
                .map(Mutation::<R>::str_to_reading(name)),

            SettingData {
                f_int: None,
                f_float: None,
                f_bool: None,
                f_string: None,
                f_color: Some(v),
            } => match v[..] {
                [r, g, b] => {
                    if let (Ok(r), Ok(g), Ok(b)) =
                        (u8::try_from(r), u8::try_from(g), u8::try_from(b))
                    {
                        Mutation::<R>::perform_setting(
                            db,
                            &name,
                            palette::LinSrgba::<u8>::new(r, g, b, 255),
                        )
                        .await
                        .map(Mutation::<R>::color_to_reading(name))
                    } else {
                        Err(Error::new("color component is out of range"))
                    }
                }
                [r, g, b, a] => {
                    if let (Ok(r), Ok(g), Ok(b), Ok(a)) = (
                        u8::try_from(r),
                        u8::try_from(g),
                        u8::try_from(b),
                        u8::try_from(a),
                    ) {
                        Mutation::perform_setting(
                            db,
                            &name,
                            palette::LinSrgba::<u8>::new(r, g, b, a),
                        )
                        .await
                        .map(Mutation::<R>::color_to_reading(name))
                    } else {
                        Err(Error::new("color component is out of range"))
                    }
                }
                _ => Err(Error::new(
                    "color values have three or four components",
                )),
            },

            SettingData { .. } => {
                Err(Error::new("must only specify one item of data"))
            }
        }
    }
}

#[derive(InputObject)]
/// Defines a range of time between two dates.
struct DateRange {
    /// The start of the date range (in UTC.) If `null`, it means "now".
    start: Option<DateTime<Utc>>,
    /// The end of the date range (in UTC.) If `null`, it means "infinity".
    end: Option<DateTime<Utc>>,
}

#[derive(SimpleObject)]
/// Represents a value of a device at an instant of time.
struct Reading {
    device: String,
    stamp: DateTime<Utc>,
    /// Placeholder for integer values.
    int_value: Option<i32>,
    /// Placeholder for float values.
    float_value: Option<f64>,
    /// Placeholder for boolean values.
    bool_value: Option<bool>,
    /// Placeholder for string values.
    string_value: Option<Arc<str>>,
    /// Placeholder for color values. Values are a 3-element array holding
    /// red, green, and blue values or a 4-element array holding red, green,
    /// blue, and alpha values. Each value ranges from 0 - 255.
    color_value: Option<Vec<i32>>,
}

impl From<&device::Reading> for Reading {
    fn from(value: &device::Reading) -> Self {
        match &value.value {
            device::Value::Bool(v) => Reading {
                device: "".into(),
                stamp: DateTime::<Utc>::from(value.ts),
                int_value: None,
                float_value: None,
                bool_value: Some(*v),
                string_value: None,
                color_value: None,
            },
            device::Value::Int(v) => Reading {
                device: "".into(),
                stamp: DateTime::<Utc>::from(value.ts),
                int_value: Some(*v),
                float_value: None,
                bool_value: None,
                string_value: None,
                color_value: None,
            },
            device::Value::Flt(v) => Reading {
                device: "".into(),
                stamp: DateTime::<Utc>::from(value.ts),
                int_value: None,
                float_value: Some(*v),
                bool_value: None,
                string_value: None,
                color_value: None,
            },
            device::Value::Str(v) => Reading {
                device: "".into(),
                stamp: DateTime::<Utc>::from(value.ts),
                int_value: None,
                float_value: None,
                bool_value: None,
                string_value: Some(v.clone()),
                color_value: None,
            },
            device::Value::Color(device::ColorType::Rgba { color })
                if color.alpha == 255 =>
            {
                Reading {
                    device: "".into(),
                    stamp: DateTime::<Utc>::from(value.ts),
                    int_value: None,
                    float_value: None,
                    bool_value: None,
                    string_value: None,
                    color_value: Some(vec![
                        color.red as i32,
                        color.green as i32,
                        color.blue as i32,
                    ]),
                }
            }
            device::Value::Color(device::ColorType::Rgba { color }) => {
                Reading {
                    device: "".into(),
                    stamp: DateTime::<Utc>::from(value.ts),
                    int_value: None,
                    float_value: None,
                    bool_value: None,
                    string_value: None,
                    color_value: Some(vec![
                        color.red as i32,
                        color.green as i32,
                        color.blue as i32,
                        color.alpha as i32,
                    ]),
                }
            }
            device::Value::Color(device::ColorType::Ccta { kelvin, a }) => {
                Reading {
                    device: "".into(),
                    stamp: DateTime::<Utc>::from(value.ts),
                    int_value: None,
                    float_value: None,
                    bool_value: None,
                    string_value: None,
                    color_value: Some(vec![*kelvin as i32, *a as i32]),
                }
            }
        }
    }
}

struct SubscriptionRoot<R: Reporter>(std::marker::PhantomData<R>);

impl<R: Reporter> SubscriptionRoot<R> {
    fn xlat(name: String) -> impl Fn(device::Reading) -> Result<Reading> {
        move |e: device::Reading| {
            let mut reading = Reading {
                device: name.clone(),
                stamp: DateTime::<Utc>::from(e.ts),
                bool_value: None,
                int_value: None,
                float_value: None,
                string_value: None,
                color_value: None,
            };

            match e.value {
                device::Value::Bool(v) => reading.bool_value = Some(v),
                device::Value::Int(v) => reading.int_value = Some(v),
                device::Value::Flt(v) => reading.float_value = Some(v),
                device::Value::Str(v) => reading.string_value = Some(v.clone()),
                device::Value::Color(device::ColorType::Rgba { color })
                    if color.alpha == 255 =>
                {
                    reading.color_value = Some(vec![
                        color.red as i32,
                        color.green as i32,
                        color.blue as i32,
                    ])
                }
                device::Value::Color(device::ColorType::Rgba { color }) => {
                    reading.color_value = Some(vec![
                        color.red as i32,
                        color.green as i32,
                        color.blue as i32,
                        color.alpha as i32,
                    ])
                }
                device::Value::Color(device::ColorType::Ccta { kelvin, a }) => {
                    reading.color_value = Some(vec![kelvin as i32, a as i32])
                }
            }

            Ok(reading)
        }
    }
}

#[Subscription]
impl<R: Reporter> SubscriptionRoot<R> {
    /// Sets up a connection to receive all updates to a device. The GraphQL
    /// request must provide the name of a device. This method returns a stream
    /// which generates a reply each time a device's value changes.
    async fn monitor_device(
        &self,
        ctx: &Context<'_>,
        device: String,
        range: Option<DateRange>,
    ) -> Result<impl futures::Stream<Item = Result<Reading>>> {
        let db = ctx.data::<ConfigDb<R>>()?;

        let name: device::Name = device
            .clone()
            .try_into()
            .map_err(|_| Error::new("badly formed device name"))?;

        debug!("setting monitor for '{}'", &name);

        let start = range.as_ref().and_then(|v| v.start);
        let end = range.as_ref().and_then(|v| v.end);

        let rx =
            db.1.monitor_device(name, start, end)
                .await
                .map_err(|_| Error::new("device not found"))?;

        Ok(tokio_stream::StreamExt::map(
            rx,
            SubscriptionRoot::<R>::xlat(device),
        ))
    }
}

type Schema<R> =
    async_graphql::Schema<Query<R>, Mutation<R>, SubscriptionRoot<R>>;

fn schema<R: Reporter + Clone>(config_db: ConfigDb<R>) -> Schema<R> {
    async_graphql::Schema::build(
        Query::<R>(std::marker::PhantomData),
        Mutation::<R>(std::marker::PhantomData),
        SubscriptionRoot::<R>(std::marker::PhantomData),
    )
    .data(config_db)
    .finish()
}

// Define the URI paths used by the GraphQL interface.

mod paths {
    pub const BASE: &str = "drmem";
    pub const QUERY: &str = "q";
    pub const SUBSCRIBE: &str = "s";

    // Until we can build strings at compile-time, we use the
    // `lazy_static` macro.

    lazy_static! {
        pub static ref FULL_QUERY: String = format!("/{}/{}", BASE, QUERY);
        pub static ref FULL_SUBSCRIBE: String =
            format!("/{}/{}", BASE, SUBSCRIBE);
    }
}

fn logic_to_gql<R: Reporter>(logic: &Logic) -> Arc<LogicBlock<R>> {
    LogicBlock {
        name: logic.name.clone().into(),
        description: logic
            .summary
            .clone()
            .unwrap_or_else(|| "".into())
            .to_string(),
        inputs: logic
            .inputs
            .iter()
            .map(|v| LogicBlockVariable::<R> {
                name: v.0.clone(),
                device: v.1.to_string(),
                _phant: std::marker::PhantomData,
            })
            .collect(),
        outputs: logic
            .outputs
            .iter()
            .map(|v| LogicBlockVariable::<R> {
                name: v.0.clone(),
                device: v.1.to_string(),
                _phant: std::marker::PhantomData,
            })
            .collect(),
        defs: logic
            .defs
            .iter()
            .map(|v| LogicBlockExpression::<R> {
                name: v.0.clone(),
                expr: v.1.clone(),
                _phant: std::marker::PhantomData,
            })
            .collect(),
        expr: logic.exprs.clone(),
    }
    .into()
}

// GraphQL query/mutation handler for POST requests
async fn graphql_post_handler<R: Reporter + Clone>(
    State(schema): State<Schema<R>>,
    Extension(config_db): Extension<ConfigDb<R>>,
    headers: HeaderMap,
    axum::Json(gql_request): axum::Json<async_graphql::Request>,
) -> Response {
    execute_graphql_request(schema, config_db, headers, gql_request).await
}

// GraphQL query handler for GET requests (used for introspection)
async fn graphql_get_handler<R: Reporter + Clone>(
    State(schema): State<Schema<R>>,
    Extension(config_db): Extension<ConfigDb<R>>,
    headers: HeaderMap,
    AxumQuery(gql_request): AxumQuery<async_graphql::Request>,
) -> Response {
    execute_graphql_request(schema, config_db, headers, gql_request).await
}

// Common GraphQL execution logic
async fn execute_graphql_request<R: Reporter + Clone>(
    schema: Schema<R>,
    config_db: ConfigDb<R>,
    headers: HeaderMap,
    gql_request: async_graphql::Request,
) -> Response {
    let mut request = gql_request.data(config_db);

    // Add any custom headers to the request context if needed
    if let Some(client_id) = headers.get("X-DrMem-Client-Id") {
        if let Ok(id) = client_id.to_str() {
            request = request.data(id.to_string());
        }
    }

    let response = schema.execute(request).await;
    let body = match serde_json::to_string(&response) {
        Ok(json) => json,
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };

    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        body,
    )
        .into_response()
}

// GraphiQL playground handler (only when graphiql feature is enabled)
#[cfg(feature = "graphiql")]
async fn graphiql() -> Html<String> {
    Html(
        async_graphql::http::GraphiQLSource::build()
            .endpoint(&*paths::FULL_QUERY)
            .subscription_endpoint(&*paths::FULL_SUBSCRIBE)
            .finish(),
    )
}

// Middleware to check client authorization
async fn check_authorization(
    headers: HeaderMap,
    Extension(allowed_clients): Extension<Arc<[String]>>,
    request: axum::http::Request<axum::body::Body>,
    next: axum::middleware::Next,
) -> std::result::Result<Response, StatusCode> {
    if let Some(client_id) = headers.get("X-DrMem-Client-Id") {
        if let Ok(client_str) = client_id.to_str() {
            if allowed_clients.iter().any(|v| cmp_fprints(v, client_str)) {
                return Ok(next.run(request).await);
            }
        }
    }
    Err(StatusCode::FORBIDDEN)
}

// Build the base routes for GraphQL
fn build_base_routes<R: Reporter + Clone>(
    schema: Schema<R>,
    context: ConfigDb<R>,
) -> Router {
    #[allow(unused_mut)]
    let mut router = Router::new()
        .route(
            &format!("/{}", paths::QUERY),
            post(graphql_post_handler::<R>).get(graphql_get_handler::<R>),
        )
        .route_service(
            &format!("/{}", paths::SUBSCRIBE),
            GraphQLSubscription::new(schema.clone()),
        )
        .with_state(schema)
        .layer(Extension(context));

    #[cfg(feature = "graphiql")]
    {
        router = router.route("/", get(graphiql));
    }

    router
}

// "Sanitizes" a string containing a digital fingerprint by returning
// an Iterator that only returns the hex digits in uppercase.

fn sanitize(ii: impl Iterator<Item = char>) -> impl Iterator<Item = char> {
    ii.filter(char::is_ascii_hexdigit)
        .map(|v| v.to_ascii_uppercase())
}

// Compares two `str`s as if they held digital fingerprints.

fn cmp_fprints(a: &str, b: &str) -> bool {
    let mut a = sanitize(a.chars());
    let mut b = sanitize(b.chars());

    loop {
        match (a.next(), b.next()) {
            (None, None) => break true,
            (Some(a), Some(b)) if a == b => continue,
            (_, _) => break false,
        }
    }
}

const ALLOWED_METHODS: [axum::http::Method; 3] = [
    axum::http::Method::OPTIONS,
    axum::http::Method::GET,
    axum::http::Method::POST,
];

const ALLOWED_HEADERS: [header::HeaderName; 3] = [
    header::CONTENT_TYPE,
    header::HeaderName::from_static("access-control-allow-origin"),
    header::HeaderName::from_static("x-drmem-client-id"),
];

fn cors_layer() -> CorsLayer {
    CorsLayer::new()
        .allow_origin(tower_http::cors::Any)
        .allow_headers(ALLOWED_HEADERS.to_vec())
        .allow_methods(ALLOWED_METHODS)
        .max_age(Duration::from_secs(3_600))
}

// Builds the server object that will handle GraphQL requests. If the
// configuration contains the `security` key, the server will require
// TLS connections.

async fn build_server<R: Reporter + Clone>(
    addr: std::net::SocketAddr,
    security: Option<config::Security>,
    db: crate::driver::DriverDb<R>,
    cchan: client::RequestChan,
    logic_blocks: Vec<Arc<LogicBlock<R>>>,
) -> std::io::Result<()> {
    let context = ConfigDb::<R>(db, cchan, logic_blocks);
    let schema = schema::<R>(context.clone());
    let graphql_routes = build_base_routes(schema, context);

    match security {
        Some(security) => {
            let allowed_clients = Arc::clone(&security.clients);

            let app = Router::new()
                .nest(&format!("/{}", paths::BASE), graphql_routes)
                .layer(axum::middleware::from_fn(
                    move |headers, ext, req, next| {
                        check_authorization(headers, ext, req, next)
                    },
                ))
                .layer(Extension(allowed_clients))
                .layer(cors_layer())
                .layer(CompressionLayer::new());

            // Build TLS configuration
            let config = axum_server::tls_rustls::RustlsConfig::from_pem_file(
                security.cert_file.clone(),
                security.key_file.clone(),
            );

            let tls_config = config.await.map_err(|e| {
                std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("TLS config error: {}", e),
                )
            })?;

            axum_server::bind_rustls(addr, tls_config)
                .serve(app.into_make_service())
                .await
        }
        None => {
            let app = Router::new()
                .nest(&format!("/{}", paths::BASE), graphql_routes)
                .layer(cors_layer())
                .layer(CompressionLayer::new());

            let listener =
                tokio::net::TcpListener::bind(addr).await.map_err(|e| {
                    std::io::Error::new(
                        std::io::ErrorKind::Other,
                        format!("Failed to bind to address: {}", e),
                    )
                })?;

            axum::serve(listener, app).await
        }
    }
}

fn calc_fingerprint(cert: &[u8]) -> String {
    use ring::digest::{Context, Digest, SHA256};
    use std::fmt::Write;

    let mut context = Context::new(&SHA256);

    context.update(cert);

    let digest: Digest = context.finish();

    // Format the fingerprint as a hexadecimal string.

    digest.as_ref().iter().fold(String::new(), |mut output, b| {
        let _ = write!(output, "{b:02X}");
        output
    })
}

fn build_mdns_payload(
    cfg: &config::Config,
) -> std::result::Result<Vec<String>, drmem_api::Error> {
    // Get the boot-time and store it in the mDNS payload.

    let boot_time: DateTime<Utc> = Utc::now();

    // Build the mDNS payload section. This is a vector of "KEY=VALUE"
    // strings which will get added to the `txt` section of the mDNS
    // announcement.

    let mut payload: Vec<String> = vec![
        format!("version={}", env!("CARGO_PKG_VERSION")),
        format!("location={}", cfg.location),
        format!(
            "boot-time={}",
            boot_time.to_rfc3339_opts(SecondsFormat::Secs, true)
        ),
        format!("queries={}", &*paths::FULL_QUERY),
        format!("mutations={}", &*paths::FULL_QUERY),
        format!("subscriptions={}", &*paths::FULL_SUBSCRIBE),
    ];

    // If security is specified, this section of code adds the digital
    // signature of the certificate to the payload.

    if let Some(sec) = &cfg.security {
        use rustls_pki_types::{pem::PemObject, CertificateDer};

        match CertificateDer::pem_file_iter(sec.cert_file.clone()) {
            Ok(mut certs) => match certs.next() {
                Some(Ok(cert)) => {
                    payload.push(format!("sig_sha={}", calc_fingerprint(&cert)))
                }
                Some(Err(e)) => {
                    return Err(drmem_api::Error::ConfigError(format!(
                        "couldn't parse certificate : {e}"
                    )))
                }
                None => {
                    return Err(drmem_api::Error::ConfigError(format!(
                        "no certificate(s) found in {}",
                        sec.cert_file.display()
                    )))
                }
            },
            Err(e) => {
                return Err(drmem_api::Error::ConfigError(format!(
                    "error accessing certificate file '{}' : {}",
                    &sec.cert_file.display(),
                    e
                )))
            }
        }
    }

    // If the configuration specifies a preferred address to use, add
    // it to the payload.

    if let Some(host) = &cfg.pref_host {
        info!("adding preferred address: {}:{}", &host, cfg.pref_port);
        payload.push(format!("pref-addr={}:{}", &host, cfg.pref_port))
    }

    Ok(payload)
}

pub fn server<R: Reporter + Clone>(
    cfg: &config::Config,
    db_logic: &[Logic],
    db: crate::driver::DriverDb<R>,
    cchan: client::RequestChan,
) -> impl Future<Output = ()> {
    let (resp, task) = Responder::with_default_handle().unwrap();
    let logic_blocks: Vec<Arc<LogicBlock<R>>> =
        db_logic.iter().map(logic_to_gql).collect();
    let http_task =
        build_server(cfg.addr, cfg.security.clone(), db, cchan, logic_blocks);

    match build_mdns_payload(cfg) {
        Ok(payload) => {
            // Register DrMem's mDNS entry. In the properties field,
            // inform the client with which paths to use for each
            // GraphQL query type.

            let service = resp.register(
                "_drmem._tcp".into(),
                &cfg.name,
                cfg.addr.port(),
                &payload.iter().map(String::as_str).collect::<Vec<&str>>(),
            );

            // Make mDNS run in the background.

            let jh = tokio::spawn(async move {
                task.await;
                drop(service)
            });

            std::mem::drop(jh);

            async move {
                if let Err(e) = http_task.await {
                    error!("HTTP server error: {}", e);
                }
            }
            .instrument(info_span!("http"))
        }
        Err(e) => {
            panic!("GraphQL config error : {e}")
        }
    }
}

#[cfg(test)]
mod test {
    use super::{cmp_fprints, sanitize};

    #[test]
    fn test_sanitizer() {
        assert_eq!(sanitize("1234".chars()).collect::<String>(), "1234");
        assert_eq!(
            sanitize("0123456789abcdefABCDEF".chars()).collect::<String>(),
            "0123456789ABCDEFABCDEF"
        );
        assert_eq!(sanitize("01:ff:45".chars()).collect::<String>(), "01FF45");
    }

    #[test]
    fn test_fprint_comparisons() {
        assert_eq!(cmp_fprints("", ""), true);
        assert_eq!(cmp_fprints("z", ""), true);
        assert_eq!(cmp_fprints("", "z"), true);

        assert_eq!(cmp_fprints("a", ""), false);
        assert_eq!(cmp_fprints("", "a"), false);

        assert_eq!(cmp_fprints("1234", "1234"), true);
        assert_eq!(cmp_fprints("abcd", "ABCD"), true);
        assert_eq!(cmp_fprints("1234", "ABCD"), false);

        assert_eq!(cmp_fprints("12:34", "1234"), true);
        assert_eq!(cmp_fprints("a:b:c:d", "AB:CD"), true);
    }

    #[test]
    fn test_digital_fingerprint() {
        use rustls_pki_types::{pem::PemObject, CertificateDer};

        // Expired Mozilla certificate.

        const CERT: &[u8] = b"-----BEGIN CERTIFICATE-----
MIIDujCCAqKgAwIBAgILBAAAAAABD4Ym5g0wDQYJKoZIhvcNAQEFBQAwTDEgMB4G
A1UECxMXR2xvYmFsU2lnbiBSb290IENBIC0gUjIxEzARBgNVBAoTCkdsb2JhbFNp
Z24xEzARBgNVBAMTCkdsb2JhbFNpZ24wHhcNMDYxMjE1MDgwMDAwWhcNMjExMjE1
MDgwMDAwWjBMMSAwHgYDVQQLExdHbG9iYWxTaWduIFJvb3QgQ0EgLSBSMjETMBEG
A1UEChMKR2xvYmFsU2lnbjETMBEGA1UEAxMKR2xvYmFsU2lnbjCCASIwDQYJKoZI
hvcNAQEBBQADggEPADCCAQoCggEBAKbPJA6+Lm8omUVCxKs+IVSbC9N/hHD6ErPL
v4dfxn+G07IwXNb9rfF73OX4YJYJkhD10FPe+3t+c4isUoh7SqbKSaZeqKeMWhG8
eoLrvozps6yWJQeXSpkqBy+0Hne/ig+1AnwblrjFuTosvNYSuetZfeLQBoZfXklq
tTleiDTsvHgMCJiEbKjNS7SgfQx5TfC4LcshytVsW33hoCmEofnTlEnLJGKRILzd
C9XZzPnqJworc5HGnRusyMvo4KD0L5CLTfuwNhv2GXqF4G3yYROIXJ/gkwpRl4pa
zq+r1feqCapgvdzZX99yqWATXgAByUr6P6TqBwMhAo6CygPCm48CAwEAAaOBnDCB
mTAOBgNVHQ8BAf8EBAMCAQYwDwYDVR0TAQH/BAUwAwEB/zAdBgNVHQ4EFgQUm+IH
V2ccHsBqBt5ZtJot39wZhi4wNgYDVR0fBC8wLTAroCmgJ4YlaHR0cDovL2NybC5n
bG9iYWxzaWduLm5ldC9yb290LXIyLmNybDAfBgNVHSMEGDAWgBSb4gdXZxwewGoG
3lm0mi3f3BmGLjANBgkqhkiG9w0BAQUFAAOCAQEAmYFThxxol4aR7OBKuEQLq4Gs
J0/WwbgcQ3izDJr86iw8bmEbTUsp9Z8FHSbBuOmDAGJFtqkIk7mpM0sYmsL4h4hO
291xNBrBVNpGP+DTKqttVCL1OmLNIG+6KYnX3ZHu01yiPqFbQfXf5WRDLenVOavS
ot+3i9DAgBkcRcAtjOj4LaR0VknFBbVPFd5uRHg5h6h+u/N5GJG79G+dwfCMNYxd
AfvDbbnvRG15RjF+Cv6pgsH/76tuIMRQyV+dTZsXjAzlAcmgQWpzU/qlULRuJQ/7
TBj0/VLZjmmx6BEP3ojY+x1J96relc8geMJgEtslQIxq/H5COEBkEveegeGTLg==
-----END CERTIFICATE-----";

        let cert = CertificateDer::from_pem_slice(CERT).unwrap();

        assert!(
	    super::cmp_fprints(
		&super::calc_fingerprint(&cert),
		"CA:42:DD:41:74:5F:D0:B8:1E:B9:02:36:2C:F9:D8:BF:71:9D:A1:BD:1B:1E:FC:94:6F:5B:4C:99:F4:2C:1B:9E"
	    )
	);
    }
}

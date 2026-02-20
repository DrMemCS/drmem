use crate::config::Logic;
use chrono::prelude::*;
use drmem_api::{
    client, device,
    driver::{self, Reporter},
    Error,
};
use futures::Future;
use juniper::{
    executor::FieldError, graphql_object, graphql_subscription, graphql_value,
    FieldResult, GraphQLInputObject, GraphQLObject, RootNode, Value,
};
use juniper_graphql_ws::ConnectionConfig;
use juniper_warp::subscriptions::serve_graphql_ws;
use libmdns::Responder;
use std::{pin::Pin, result, sync::Arc, time::Duration};
use tracing::{debug, error, info, info_span, warn, Instrument};
use warp::{http::StatusCode, reject, reply, Filter, Rejection, Reply};

pub mod config;

#[derive(Debug)]
struct NoAuthorization;

impl reject::Reject for NoAuthorization {}

// The Context parameter for Queries.

#[derive(Clone)]
struct ConfigDb<R: Reporter>(
    crate::driver::DriverDb<R>,
    client::RequestChan,
    Vec<Arc<LogicBlock<R>>>,
);

impl<R: Reporter> juniper::Context for ConfigDb<R> {}

// `DriverInfo` is an object that can be returned by a GraphQL
// query. It contains information related to drivers that are
// available in the DrMem environment (executable.)

struct DriverInfo<R: Reporter> {
    name: driver::Name,
    summary: &'static str,
    description: &'static str,
    phant: std::marker::PhantomData<R>,
}

#[graphql_object(
    Context = ConfigDb<R>,
    description = "Information about a driver in the running version \
		   of `drmemd`.\n\n\
		   An instance of DrMem has a set of drivers that are \
		   compiled into the executable. This set will be fixed \
		   until the next time the node is restarted (because the \
		   restart may be a new executable with a different set \
		   of drivers.)"
)]
impl<R: Reporter> DriverInfo<R> {
    #[graphql(description = "The name of the driver.")]
    fn name(&self) -> driver::Name {
        self.name.clone()
    }

    #[graphql(description = "A short summary of the driver's purpose.")]
    fn summary(&self) -> &str {
        self.summary
    }

    #[graphql(description = "Detailed information about the driver: the \
                             configuration parameters; the devices it \
                             registers; and other pertinent information. \
                             This information is formatted in Markdown.")]
    fn description(&self) -> &str {
        self.description
    }
}

#[derive(GraphQLInputObject)]
#[graphql(description = "Describes data that can be sent to devices. When \
			 specifying data, one -- and only one -- field \
			 must be set.")]
struct SettingData {
    #[graphql(name = "int", description = "Placeholder for integer values.")]
    f_int: Option<i32>,
    #[graphql(name = "flt", description = "Placeholder for float values.")]
    f_float: Option<f64>,
    #[graphql(name = "bool", description = "Placeholder for boolean values.")]
    f_bool: Option<bool>,
    #[graphql(name = "str", description = "Placeholder for string values.")]
    f_string: Option<String>,
    #[graphql(name = "color", description = "Placeholder for color values.")]
    f_color: Option<Vec<i32>>,
}

// Contains information about a device's history in the backend.

#[derive(GraphQLObject)]
#[graphql(description = "Contains information about a device's history, \
			 as currently stored in the backend. This information \
			 is a snapshot from when it was obtained. Depending \
			 on how frequently a device gets updated, this \
			 information may be obsolete in a short time.")]
struct DeviceHistory {
    #[graphql(description = "Total number of points in backend storage.")]
    total_points: i32,
    #[graphql(description = "The oldest data point in storage. If the total\
			     is 0, then this field will be null. Note that\
			     this value is accurate at the time of this\
			     query. However, at any moment, the oldest data\
			     point could be thrown away if new data arrives.")]
    first_point: Option<Reading>,
    #[graphql(description = "The latest data point in storage. If the total\
			     is 0, then this field will be null. Note that\
			     this value is accurate at the time of this\
			     query. However, at any moment, newer data could\
			     be added.")]
    last_point: Option<Reading>,
}

// `DeviceInfo` is a GraphQL object which contains information about a
// device.

struct DeviceInfo<R: Reporter> {
    device_name: String,
    units: Option<String>,
    settable: bool,
    driver_name: driver::Name,
    history: DeviceHistory,
    db: crate::driver::DriverDb<R>,
}

#[graphql_object(
    Context = ConfigDb<R>,
    description = "Information about a registered device in the running \
		   version of `drmemd`."
)]
impl<R: Reporter> DeviceInfo<R> {
    #[graphql(description = "The name of the device.")]
    fn device_name(&self) -> &str {
        &self.device_name
    }

    #[graphql(description = "The engineering units of the device's value.")]
    fn units(&self) -> Option<&String> {
        self.units.as_ref()
    }

    #[graphql(description = "Indicates whether the device is read-only \
			     or can be controlled.")]
    fn settable(&self) -> bool {
        self.settable
    }

    #[graphql(description = "Information about the driver that implements \
			     this device.")]
    fn driver(&self) -> DriverInfo<R> {
        self.db
            .get_driver(&self.driver_name)
            .map(|di| DriverInfo {
                name: self.driver_name.clone(),
                summary: di.0,
                description: di.1,
                phant: std::marker::PhantomData,
            })
            .unwrap()
    }

    fn history(&self) -> &DeviceHistory {
        &self.history
    }
}

struct LogicBlockVariable<R: Reporter> {
    name: String,
    device: String,
    phant: std::marker::PhantomData<R>,
}

#[graphql_object(
    Context = ConfigDb<R>,
    description = "Shows the input/output variable mapping in a logic block."
)]
impl<R: Reporter> LogicBlockVariable<R> {
    #[graphql(description = "The name of the variable.")]
    fn name(&self) -> &str {
        &self.name
    }

    #[graphql(description = "The name of the variable.")]
    fn device(&self) -> &str {
        &self.device
    }
}

struct LogicBlockExpression<R: Reporter> {
    name: String,
    expr: String,
    phant: std::marker::PhantomData<R>,
}

#[graphql_object(
    Context = ConfigDb<R>,
    description = "Shows the expression definitions in a logic block."
)]
impl<R: Reporter> LogicBlockExpression<R> {
    #[graphql(description = "The name of the definition.")]
    fn name(&self) -> &str {
        &self.name
    }

    #[graphql(description = "The expression.")]
    fn expr(&self) -> &str {
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

#[graphql_object(
    Context = ConfigDb<R>,
    description = "Shows the configuration of a logic block."
)]
impl<R: Reporter> LogicBlock<R> {
    #[graphql(description = "The name of the logic block.")]
    fn name(&self) -> &str {
        &self.name
    }

    #[graphql(description = "A description of the logic block's purpose.")]
    fn description(&self) -> &str {
        &self.description
    }

    #[graphql(description = "The inputs needed by the logic block.")]
    fn inputs(&self) -> &[LogicBlockVariable<R>] {
        &self.inputs
    }

    #[graphql(description = "The outputs controlled by the logic block.")]
    fn outputs(&self) -> &[LogicBlockVariable<R>] {
        &self.outputs
    }

    #[graphql(description = "Shared expressions used by the logic block.")]
    fn defs(&self) -> &[LogicBlockExpression<R>] {
        &self.defs
    }

    #[graphql(description = "Control expressions used by the logic block.")]
    fn expr(&self) -> &[String] {
        &self.expr
    }
}

// This defines the top-level Query API.

struct Config<R: Reporter>(std::marker::PhantomData<R>);

impl<R: Reporter> Config<R> {
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
            // LogicBlock doesn't that name. If it has the name, we
            // still need to see if the device name  filter further
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

#[graphql_object(
    context = ConfigDb<R>,
    description = "Reports configuration information for `drmemd`."
)]
impl<R: Reporter + Clone> Config<R> {
    #[graphql(description = "Returns logic blocks configured in the node. By \
                             default, all logic blocks are returned. If either \
                             parameter is given, the results are filtered to \
                             only return information that matches the selection \
                             values.")]
    fn logic_blocks(
        #[graphql(context)] db: &ConfigDb<R>,
        #[graphql(description = "If provided, only the logic block with the \
                                 specified name will be returned.")]
        sel_name: Option<String>,
        #[graphql(
            description = "This parameter can specify a list of device \
                                 names. Only logic blocks that use any of the \
                                 devices in either input or output will be \
                                 included in the results."
        )]
        sel_devices: Option<Vec<String>>,
    ) -> result::Result<Vec<Arc<LogicBlock<R>>>, FieldError> {
        Ok(db
            .2
            .iter()
            .cloned()
            .filter(Self::logic_block_filter(sel_name, sel_devices))
            .collect())
    }

    #[graphql(description = "Returns information about the available drivers \
			     in the running instance of `drmemd`. If `name` \
			     isn't provided, an array of all driver \
			     information is returned. If `name` is specified \
			     and a driver with that name exists, a single \
			     element array is returned. Otherwise `null` is \
			     returned.")]
    fn driver_info(
        #[graphql(context)] db: &ConfigDb<R>,
        #[graphql(description = "An optional argument which, when provided, \
				 only returns driver information whose name \
				 matches. If this argument isn't provided, \
				 every drivers' information will be returned.")]
        name: Option<String>,
    ) -> result::Result<Vec<DriverInfo<R>>, FieldError> {
        if let Some(name) = name {
            if let Some((n, s, d)) = db.0.find(&name) {
                Ok(vec![DriverInfo {
                    name: n,
                    summary: s,
                    description: d,
                    phant: std::marker::PhantomData,
                }])
            } else {
                Err(FieldError::new(
                    "driver not found",
                    graphql_value!({ "missing_driver": name }),
                ))
            }
        } else {
            let result =
                db.0.get_all()
                    .map(|(n, s, d)| DriverInfo {
                        name: n,
                        summary: s,
                        description: d,
                        phant: std::marker::PhantomData,
                    })
                    .collect();

            Ok(result)
        }
    }

    #[graphql(
        description = "Returns information associated with the devices that \
		       are active in the running system. Arguments to the \
		       query will filter the results.\n\n\
		       If the argument `pattern` is provided, only the \
		       devices whose name matches the pattern will be \
		       included in the results. The pattern follows the \
		       shell \"glob\" style.\n\n\
		       If the argument `settable` is provided, it returns \
		       devices that are or aren't settable, depending on the \
		       value of the agument."
    )]
    async fn device_info(
        #[graphql(context)] db: &ConfigDb<R>,
        #[graphql(
            name = "pattern",
            description = "If this argument is provided, the query returns \
			   information for devices whose name matches the \
			   pattern. The pattern uses \"globbing\" grammar: \
			   '?' matches one character, '*' matches zero or \
			   more, '**' matches arbtrary levels of the path \
			   (between ':'s)."
        )]
        pattern: Option<String>,
        #[graphql(
            name = "settable",
            description = "If this argument is provided, the query filters \
			   the result based on whether the device can be set \
			   or not."
        )]
        settable: Option<bool>,
    ) -> result::Result<Vec<DeviceInfo<R>>, FieldError> {
        let tx = db.1.clone();
        let filt = settable
            .map(|v| {
                if v {
                    Config::<R>::is_settable
                } else {
                    Config::<R>::is_not_settable
                }
            })
            .unwrap_or(Config::<R>::is_true);

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
            .map_err(|_| {
                FieldError::new("error looking-up device", Value::null())
            })
    }
}

// The `Control` mutation is used to group queries that attempt to
// control devices by sending them settings.

struct Control<R: Reporter>(std::marker::PhantomData<R>);

impl<R: Reporter> Control<R> {
    // Sends a new value to a device.

    async fn perform_setting<
        T: Into<device::Value> + TryFrom<device::Value, Error = Error>,
    >(
        db: &ConfigDb<R>,
        device: &str,
        value: T,
    ) -> result::Result<T, FieldError> {
        // Make sure the device name is properly formed.

        if let Ok(name) = device.try_into() {
            let tx = db.1.clone();

            // Send the setting to the driver. Map the error, if any,
            // to a `FieldError` type.

            tx.set_device::<T>(name, value).await.map_err(|e| {
                let errmsg = format!("{}", &e);

                FieldError::new(
                    "error making setting",
                    graphql_value!({ "error": errmsg }),
                )
            })
        } else {
            Err(FieldError::new("badly formed device name", Value::null()))
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

#[graphql_object(
    context = ConfigDb<R>,
    description = "This group of queries perform modifications to devices."
)]
impl<R: Reporter> Control<R> {
    #[graphql(description = "Submits `value` to be applied to the device \
			     associated with the given `name`. If the data \
			     is in a format the device doesn't support an \
			     error is returned. The `value` parameter \
			     contains several fields. Only one should be \
			     set. It is an error to have all fields `null` \
			     or more than one field non-`null`.")]
    async fn set_device(
        #[graphql(context)] db: &ConfigDb<R>,
        name: String,
        value: SettingData,
    ) -> FieldResult<Reading> {
        match value {
            SettingData {
                f_int: None,
                f_float: None,
                f_bool: None,
                f_string: None,
                f_color: None,
            } => Err(FieldError::new("no data provided", Value::null())),

            SettingData {
                f_int: Some(v),
                f_float: None,
                f_bool: None,
                f_string: None,
                f_color: None,
            } => Control::perform_setting(db, &name, v)
                .await
                .map(Control::<R>::int_to_reading(name)),

            SettingData {
                f_int: None,
                f_float: Some(v),
                f_bool: None,
                f_string: None,
                f_color: None,
            } => Control::perform_setting(db, &name, v)
                .await
                .map(Control::<R>::flt_to_reading(name)),

            SettingData {
                f_int: None,
                f_float: None,
                f_bool: Some(v),
                f_string: None,
                f_color: None,
            } => Control::perform_setting(db, &name, v)
                .await
                .map(Control::<R>::bool_to_reading(name)),

            SettingData {
                f_int: None,
                f_float: None,
                f_bool: None,
                f_string: Some(v),
                f_color: None,
            } => Control::perform_setting(db, &name, v)
                .await
                .map(Control::<R>::str_to_reading(name)),

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
                        Control::<R>::perform_setting(
                            db,
                            &name,
                            palette::LinSrgba::<u8>::new(r, g, b, 255),
                        )
                        .await
                        .map(Control::<R>::color_to_reading(name))
                    } else {
                        Err(FieldError::new(
                            "color component is out of range",
                            Value::null(),
                        ))
                    }
                }
                [r, g, b, a] => {
                    if let (Ok(r), Ok(g), Ok(b), Ok(a)) = (
                        u8::try_from(r),
                        u8::try_from(g),
                        u8::try_from(b),
                        u8::try_from(a),
                    ) {
                        Control::perform_setting(
                            db,
                            &name,
                            palette::LinSrgba::<u8>::new(r, g, b, a),
                        )
                        .await
                        .map(Control::<R>::color_to_reading(name))
                    } else {
                        Err(FieldError::new(
                            "color component is out of range",
                            Value::null(),
                        ))
                    }
                }
                _ => Err(FieldError::new(
                    "color values have three or four components",
                    Value::null(),
                )),
            },

            SettingData { .. } => Err(FieldError::new(
                "must only specify one item of data",
                Value::null(),
            )),
        }
    }
}

#[derive(GraphQLInputObject)]
#[graphql(description = "Defines a range of time between two dates.")]
struct DateRange {
    #[graphql(
        description = "The start of the date range (in UTC.) If `null`, \
			     it means \"now\"."
    )]
    start: Option<DateTime<Utc>>,
    #[graphql(description = "The end of the date range (in UTC.) If `null`, \
			     it means \"infinity\".")]
    end: Option<DateTime<Utc>>,
}

#[derive(GraphQLObject)]
#[graphql(
    description = "Represents a value of a device at an instant of time."
)]
struct Reading {
    device: String,
    stamp: DateTime<Utc>,
    #[graphql(description = "Placeholder for integer values.")]
    int_value: Option<i32>,
    #[graphql(description = "Placeholder for float values.")]
    float_value: Option<f64>,
    #[graphql(description = "Placeholder for boolean values.")]
    bool_value: Option<bool>,
    #[graphql(description = "Placeholder for string values.")]
    string_value: Option<Arc<str>>,
    #[graphql(
        description = "Placeholder for color values. Values are a 3-element \
		       array holding red, green, and blue values or a \
		       4-element array holding red, gree, blue, and alpha \
		       values. Each value ranges from 0 - 255."
    )]
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
            device::Value::Color(v) if v.alpha == 255 => Reading {
                device: "".into(),
                stamp: DateTime::<Utc>::from(value.ts),
                int_value: None,
                float_value: None,
                bool_value: None,
                string_value: None,
                color_value: Some(vec![
                    v.red as i32,
                    v.green as i32,
                    v.blue as i32,
                ]),
            },
            device::Value::Color(v) => Reading {
                device: "".into(),
                stamp: DateTime::<Utc>::from(value.ts),
                int_value: None,
                float_value: None,
                bool_value: None,
                string_value: None,
                color_value: Some(vec![
                    v.red as i32,
                    v.green as i32,
                    v.blue as i32,
                    v.alpha as i32,
                ]),
            },
        }
    }
}

struct Subscription<R: Reporter>(std::marker::PhantomData<R>);

impl<R: Reporter> Subscription<R> {
    fn xlat(name: String) -> impl Fn(device::Reading) -> FieldResult<Reading> {
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
                device::Value::Color(v) if v.alpha == 255 => {
                    reading.color_value =
                        Some(vec![v.red as i32, v.green as i32, v.blue as i32])
                }
                device::Value::Color(v) => {
                    reading.color_value = Some(vec![
                        v.red as i32,
                        v.green as i32,
                        v.blue as i32,
                        v.alpha as i32,
                    ])
                }
            }

            Ok(reading)
        }
    }
}

#[graphql_subscription(context = ConfigDb<R>)]
impl<R: Reporter> Subscription<R> {
    #[graphql(description = "Sets up a connection to receive all updates to \
			     a device. The GraphQL request must provide the \
			     name of a device. This method returns a stream \
			     which generates a reply each time a device's \
			     value changes.")]
    async fn monitor_device(
        #[graphql(context)] db: &ConfigDb<R>,
        device: String,
        range: Option<DateRange>,
    ) -> device::DataStream<FieldResult<Reading>> {
        use tokio_stream::StreamExt;

        if let Ok(name) = device.clone().try_into() {
            debug!("setting monitor for '{}'", &name);

            let start = range.as_ref().and_then(|v| v.start);
            let end = range.as_ref().and_then(|v| v.end);

            if let Ok(rx) = db.1.monitor_device(name, start, end).await {
                let stream =
                    StreamExt::map(rx, Subscription::<R>::xlat(device));

                Box::pin(stream) as device::DataStream<FieldResult<Reading>>
            } else {
                let stream = tokio_stream::once(Err(FieldError::new(
                    "device not found",
                    Value::null(),
                )));

                Box::pin(stream) as device::DataStream<FieldResult<Reading>>
            }
        } else {
            let stream = tokio_stream::once(Err(FieldError::new(
                "badly formed device name",
                Value::null(),
            )));

            Box::pin(stream) as device::DataStream<FieldResult<Reading>>
        }
    }
}

type Schema<R> = RootNode<'static, Config<R>, Control<R>, Subscription<R>>;

fn schema<R: Reporter + Clone>() -> Schema<R> {
    Schema::new(
        Config::<R>(std::marker::PhantomData),
        Control::<R>(std::marker::PhantomData),
        Subscription::<R>(std::marker::PhantomData),
    )
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
                phant: std::marker::PhantomData,
            })
            .collect(),
        outputs: logic
            .outputs
            .iter()
            .map(|v| LogicBlockVariable::<R> {
                name: v.0.clone(),
                device: v.1.to_string(),
                phant: std::marker::PhantomData,
            })
            .collect(),
        defs: logic
            .defs
            .iter()
            .map(|v| LogicBlockExpression::<R> {
                name: v.0.clone(),
                expr: v.1.clone(),
                phant: std::marker::PhantomData,
            })
            .collect(),
        expr: logic.exprs.clone(),
    }
    .into()
}

// Build `warp::Filter`s that define the entire webspace.

fn build_base_site<R: Reporter + Clone>(
    db: crate::driver::DriverDb<R>,
    cchan: client::RequestChan,
    db_logic: &[Logic],
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
    let context = ConfigDb::<R>(
        db,
        cchan,
        db_logic.iter().map(logic_to_gql).collect::<Vec<_>>(),
    );
    let ctxt = context.clone();

    // Create filter that handles GraphQL queries and mutations.

    let state = warp::any().map(move || ctxt.clone());
    let query_filter = warp::path(paths::QUERY)
        .and(warp::path::end())
        .and(warp::post().or(warp::get()).unify())
        .and(juniper_warp::make_graphql_filter(schema(), state.boxed()));

    // Create filter that handle the interactive GraphQL app. This
    // service is found at the BASE path.

    #[cfg(feature = "graphiql")]
    let graphiql_filter =
        warp::path::end().and(juniper_warp::playground_filter(
            &*paths::FULL_QUERY,
            Some(&*paths::FULL_SUBSCRIBE),
        ));

    // Create the filter that handles subscriptions.

    let sub_filter = warp::path(paths::SUBSCRIBE)
        .and(warp::path::end())
        .and(warp::ws())
        .and(warp::addr::remote())
        .map(
            move |ws: warp::ws::Ws, addr: Option<std::net::SocketAddr>| {
                let ctxt = context.clone();
                let root_node = schema();

                let reply = ws.on_upgrade(move |websocket| {
                    async move {
                        let _ = serve_graphql_ws(
                            websocket,
                            Arc::new(root_node),
                            ConnectionConfig::new(ctxt.clone()),
                        )
                        .await;
                    }
                    .instrument(info_span!(
                        "graphql",
                        client = addr
                            .map(|v| v.to_string())
                            .unwrap_or_else(|| String::from("*unknown*"))
                            .as_str()
                    ))
                });

                warp::reply::with_header(
                    reply,
                    "Sec-Websocket-Protocol",
                    "graphql-ws",
                )
            },
        );

    #[cfg(feature = "graphiql")]
    let site = query_filter.or(graphiql_filter).or(sub_filter);

    #[cfg(not(feature = "graphiql"))]
    let site = query_filter.or(sub_filter);

    // Stitch the filters together to build the map of the web
    // interface.

    warp::path(paths::BASE)
        .and(site)
        .with(warp::log("gql::drmem"))
        .with(warp::filters::compression::gzip())
        .with(
            warp::cors()
                .allow_any_origin()
                .allow_headers(vec![
                    "content-type",
                    "Access-Control-Allow-Origin",
                    "x-drmem-client-id",
                ])
                .allow_methods(vec!["OPTIONS", "GET", "POST"])
                .max_age(Duration::from_secs(3_600)),
        )
}

fn build_site<R: Reporter + Clone>(
    db: crate::driver::DriverDb<R>,
    cchan: client::RequestChan,
    db_logic: &[Logic],
) -> impl Filter<Extract = (impl Reply,), Error = std::convert::Infallible> + Clone
{
    warn!("building insecure GraphQL interface");
    build_base_site(db, cchan, db_logic).recover(handle_rejection)
}

fn build_secure_site<R: Reporter + Clone>(
    cfg: &config::Security,
    db: crate::driver::DriverDb<R>,
    cchan: client::RequestChan,
    db_logic: &[Logic],
) -> impl Filter<Extract = (impl Reply,), Error = std::convert::Infallible> + Clone
{
    info!("building secure GraphQL interface");

    // Clone the table of clients that are allowed in to the system.

    let clients: Arc<[String]> = Arc::clone(&cfg.clients);

    // Create a closure that validates the client. It takes a client
    // fingerprint as an argument and checks to see if it exists in
    // the list of clients in the configuration.

    let check_client = move |client: String| {
        futures::future::ready(
            if clients.iter().any(|v| cmp_fprints(v, &client)) {
                Ok(())
            } else {
                Err(reject::custom(NoAuthorization))
            },
        )
    };

    // Build the TLS server.

    warp::header::<String>("X-DrMem-Client-Id")
        .and_then(check_client)
        .untuple_one()
        .and(build_base_site(db, cchan, db_logic))
        .recover(handle_rejection)
}

// "Sanitizes" a string containing a digital fingerprint by returning
// an Iterator that only returns the hex digits in uppercase.

fn sanitize<T>(ii: T) -> impl Iterator<Item = char>
where
    T: Iterator<Item = char>,
{
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

// Builds the server object that will handle GraphQL requests. If the
// configuration contains the `security` key, the server will require
// TLS connections.

fn build_server<R: Reporter + Clone>(
    cfg: &config::Config,
    db_logic: &[Logic],
    db: crate::driver::DriverDb<R>,
    cchan: client::RequestChan,
) -> Pin<Box<dyn Future<Output = ()> + Send>> {
    if let Some(security) = &cfg.security {
        Box::pin(
            warp::serve(build_secure_site(security, db, cchan, db_logic))
                .tls()
                .key_path(security.key_file.clone())
                .cert_path(security.cert_file.clone())
                .bind(cfg.addr),
        ) as Pin<Box<dyn Future<Output = ()> + Send>>
    } else {
        Box::pin(warp::serve(build_site(db, cchan, db_logic)).bind(cfg.addr))
            as Pin<Box<dyn Future<Output = ()> + Send>>
    }
}

async fn handle_rejection(
    err: Rejection,
) -> Result<impl Reply, std::convert::Infallible> {
    if err.is_not_found() {
        Ok(reply::with_status("NOT_FOUND", StatusCode::NOT_FOUND))
    } else if err.find::<NoAuthorization>().is_some()
        || err.find::<reject::MissingHeader>().is_some()
    {
        Ok(reply::with_status("FORBIDDEN", StatusCode::FORBIDDEN))
    } else {
        error!("unhandled rejection: {:?}", err);
        Ok(reply::with_status(
            "INTERNAL_SERVER_ERROR",
            StatusCode::INTERNAL_SERVER_ERROR,
        ))
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

fn build_mdns_payload(cfg: &config::Config) -> Result<Vec<String>, Error> {
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
                    return Err(Error::ConfigError(format!(
                        "couldn't parse certificate : {e}"
                    )))
                }
                None => {
                    return Err(Error::ConfigError(format!(
                        "no certificate(s) found in {}",
                        sec.cert_file.display()
                    )))
                }
            },
            Err(e) => {
                return Err(Error::ConfigError(format!(
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
    let http_task = build_server(cfg, db_logic, db, cchan);

    match build_mdns_payload(cfg) {
        Ok(payload) => {
            // Register DrMem's mDNS entry. In the properties field,
            // inform the client with which paths to use for each
            // GraphQL query type.

            let service = resp.register(
                "_drmem._tcp".into(),
                cfg.name.clone(),
                cfg.addr.port(),
                &payload.iter().map(String::as_str).collect::<Vec<&str>>(),
            );

            // Make mDNS run in the background.

            let jh = tokio::spawn(async move {
                task.await;
                drop(service)
            });

            std::mem::drop(jh);

            http_task.instrument(info_span!("http"))
        }
        Err(e) => {
            panic!("GraphQL config error : {e}")
        }
    }
}

#[cfg(test)]
mod test {
    use super::{cmp_fprints, sanitize};
    use drmem_api::{device, driver::Reporter};

    #[derive(Clone)]
    struct Report;

    impl Reporter for Report {
        async fn report_value(&mut self, _value: device::Value) {}
    }

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

    #[tokio::test]
    async fn test_base_site() {
        use super::build_site;
        use crate::driver::DriverDb;
        use drmem_api::client::RequestChan;
        use tokio::sync::mpsc;

        let (tx, _) = mpsc::channel(100);
        let filter =
            build_site(DriverDb::<Report>::create(), RequestChan::new(tx), &[]);

        #[cfg(not(feature = "graphiql"))]
        {
            let value = warp::test::request().path("/").reply(&filter).await;

            assert_eq!(value.status(), 404);
        }

        // Test a client that asks for a valid path, but is using the
        // incorrect method or a valid path and method but no body or
        // all present but the body content isn't valid. Should return
        // a BAD_REQUEST status.

        {
            let value =
                warp::test::request().path("/drmem/q").reply(&filter).await;

            assert_eq!(value.status(), 400);

            let value = warp::test::request()
                .method("POST")
                .path("/drmem/q")
                .reply(&filter)
                .await;

            assert_eq!(value.status(), 400);

            let value = warp::test::request()
                .method("POST")
                .path("/drmem/q")
                .body("query { }")
                .reply(&filter)
                .await;

            assert_eq!(value.status(), 400);
        }

        // Handle a perfect query.

        {
            let value = warp::test::request()
                .method("POST")
                .path("/drmem/q")
                .body(
                    "{
    \"query\": \"query { driverInfo { name } }\",
    \"variables\": {},
    \"operationName\": null
}",
                )
                .reply(&filter)
                .await;

            assert_eq!(value.status(), 200);
        }

        // Test clients using the WebSocket interface.

        {
            let (tx, _) = mpsc::channel(100);
            let filter = build_site(
                DriverDb::<Report>::create(),
                RequestChan::new(tx),
                &[],
            );
            let client =
                warp::test::ws().path("/drmem/s").handshake(filter).await;

            assert!(client.is_ok());
        }
    }

    #[tokio::test]
    async fn test_site_security() {
        use super::{build_secure_site, config::Security};
        use crate::driver::DriverDb;
        use drmem_api::client::RequestChan;
        use std::{path::Path, sync::Arc};
        use tokio::sync::mpsc;

        let (tx, _) = mpsc::channel(100);
        let cfg = Security {
            clients: Arc::new([
                "00:11:22:33:44:55:66:77".into(),
                "11:11:11:11:11:11:11:11".into(),
            ]),
            cert_file: Path::new("").into(),
            key_file: Path::new("").into(),
        };
        let filter = build_secure_site(
            &cfg,
            DriverDb::<Report>::create(),
            RequestChan::new(tx),
            &[],
        );

        // Test a client that didn't define the Client ID
        // header. Should generate a FORBIDDEN status.

        {
            let value = warp::test::request().path("/").reply(&filter).await;

            assert_eq!(value.status(), 403);
        }

        // Test a client that presents a client ID not in our list of
        // valid clients. Should return a FORBIDDEN status.

        {
            let value = warp::test::request()
                .header("X-DrMem-Client-Id", "77:66:55:44:33:22:11:00")
                .path("/")
                .reply(&filter)
                .await;

            assert_eq!(value.status(), 403);
        }

        // Test a valid client that asks for a URL that doesn't have
        // any handler. Should return a NOT_FOUND status.

        {
            let value = warp::test::request()
                .header("X-DrMem-Client-Id", "00:11:22:33:44:55:66:77")
                .path("/")
                .reply(&filter)
                .await;

            assert_eq!(value.status(), 404);
        }

        // Test a valid client that asks for a valid path, but is
        // using the incorrect method or a valid path and method but
        // no body or all present but the body content isn't
        // valid. Should return a BAD_REQUEST status.

        {
            let value = warp::test::request()
                .header("X-DrMem-Client-Id", "00:11:22:33:44:55:66:77")
                .path("/drmem/q")
                .reply(&filter)
                .await;

            assert_eq!(value.status(), 400);

            let value = warp::test::request()
                .method("POST")
                .header("X-DrMem-Client-Id", "00:11:22:33:44:55:66:77")
                .path("/drmem/q")
                .reply(&filter)
                .await;

            assert_eq!(value.status(), 400);

            let value = warp::test::request()
                .method("POST")
                .header("X-DrMem-Client-Id", "00:11:22:33:44:55:66:77")
                .path("/drmem/q")
                .body("query { }")
                .reply(&filter)
                .await;

            assert_eq!(value.status(), 400);
        }

        // Handle a perfect, secure query.

        {
            let value = warp::test::request()
                .method("POST")
                .header("X-DrMem-Client-Id", "00:11:22:33:44:55:66:77")
                .path("/drmem/q")
                .body(
                    "{
    \"query\": \"query { driverInfo { name } }\",
    \"variables\": {},
    \"operationName\": null
}",
                )
                .reply(&filter)
                .await;

            assert_eq!(value.status(), 200);
        }

        // Test clients using the WebSocket interface.

        {
            let (tx, _) = mpsc::channel(100);
            let filter = build_secure_site(
                &cfg,
                DriverDb::<Report>::create(),
                RequestChan::new(tx),
                &[],
            );
            let client =
                warp::test::ws().path("/drmem/s").handshake(filter).await;

            assert!(client.is_err());
        }

        {
            let (tx, _) = mpsc::channel(100);
            let filter = build_secure_site(
                &cfg,
                DriverDb::<Report>::create(),
                RequestChan::new(tx),
                &[],
            );
            let client = warp::test::ws()
                .header("X-DrMem-Client-Id", "77:66:55:44:33:22:11:00")
                .path("/drmem/s")
                .handshake(filter)
                .await;

            assert!(client.is_err());
        }

        {
            let (tx, _) = mpsc::channel(100);
            let filter = build_secure_site(
                &cfg,
                DriverDb::<Report>::create(),
                RequestChan::new(tx),
                &[],
            );
            let client = warp::test::ws()
                .header("X-DrMem-Client-Id", "00:11:22:33:44:55:66:77")
                .path("/drmem/s")
                .handshake(filter)
                .await;

            assert!(client.is_ok());
        }
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

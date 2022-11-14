use chrono::prelude::*;
use drmem_api::{
    client,
    types::{device, Error},
};
use futures::{Future, FutureExt};
use juniper::{
    self, executor::FieldError, graphql_subscription, graphql_value,
    FieldResult, GraphQLInputObject, GraphQLObject, RootNode, Value,
};
use juniper_graphql_ws::ConnectionConfig;
use juniper_warp::{playground_filter, subscriptions::serve_graphql_ws};
use std::{result, sync::Arc};
use tracing::{error, info, info_span};
use tracing_futures::Instrument;
use warp::Filter;

// The Context parameter for Queries.

#[derive(Clone)]
struct ConfigDb(crate::driver::DriverDb, client::RequestChan);

impl juniper::Context for ConfigDb {}

// `DriverInfo` is an object that can be returned by a GraphQL
// query. It contains information related to drivers that are
// available in the DrMem environment (executable.)

struct DriverInfo {
    name: String,
    summary: &'static str,
    description: &'static str,
}

#[juniper::graphql_object(
    Context = ConfigDb,
    description = "Information about a driver in the running version \
		   of `drmemd`."
)]
impl DriverInfo {
    #[graphql(description = "The name of the driver.")]
    fn name(&self) -> &str {
        &self.name
    }

    #[graphql(description = "A short summary of the driver's purpose.")]
    fn summary(&self) -> &str {
        &self.summary
    }

    #[graphql(description = "Detailed information about the driver: the \
			     configuration parameters; the devices it \
			     registers; and other pertinent information. \
			     This information is formatted in Markdown.")]
    fn description(&self) -> &str {
        &self.description
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
}

// `DeviceInfo` is a GraphQL object which contains information about a
// device.

struct DeviceInfo {
    device_name: String,
    units: Option<String>,
    settable: bool,
    driver_name: String,
    db: crate::driver::DriverDb,
}

#[juniper::graphql_object(
    Context = ConfigDb,
    description = "Information about a registered device in the running \
		   version of `drmemd`."
)]
impl DeviceInfo {
    #[graphql(description = "The name of the device.")]
    fn device_name(&self) -> &str {
        &self.device_name
    }

    #[graphql(description = "The engineering units of the device's value.")]
    fn units(&self) -> &Option<String> {
        &self.units
    }

    #[graphql(
        description = "Indicates whether the device is read-only or can be controlled."
    )]
    fn settable(&self) -> bool {
        self.settable
    }

    #[graphql(
        description = "Information about the driver that implements this device."
    )]
    fn driver(&self) -> Option<DriverInfo> {
        self.db.get_driver(&self.driver_name).map(|di| DriverInfo {
            name: self.driver_name.clone(),
            summary: di.summary,
            description: di.description,
        })
    }
}

// This defines the top-level Query API.

struct Config;

impl Config {
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
}

#[juniper::graphql_object(
    context = ConfigDb,
    description = "Reports configuration information for `drmemd`."
)]
impl Config {
    #[graphql(
        description = "Returns information about the available drivers \
		       in the running instance of `drmemd`. If `name` \
		       isn't provided, an array of all driver \
		       information is returned. If `name` is specified \
		       and a driver with that name exists, a single \
		       element array is returned. Otherwise `null` is \
		       returned.",
        arguments(arg2(
            description = "An optional argument which, when provided, \
			   only returns driver information whose name \
			   matches. If this argument isn't provided, \
			   every drivers' information will be returned."
        ),)
    )]
    fn driver_info(
        #[graphql(context)] db: &ConfigDb, name: Option<String>,
    ) -> result::Result<Vec<DriverInfo>, FieldError> {
        if let Some(name) = name {
            if let Some((n, s, d)) = db.0.find(&name) {
                Ok(vec![DriverInfo {
                    name: n,
                    summary: s,
                    description: d,
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
                    })
                    .collect();

            Ok(result)
        }
    }

    #[graphql(
        description = "Returns information associated with the devices that \
			     are active in the running system. Arguments to the \
			     query will filter the results.\n\n\
			     \
			     If the argument `pattern` is provided, only the devices \
			     whose name matches the pattern will be included in the \
			     results. The pattern follows the shell \"glob\" style.\n\n\
			     \
			     If the argument `settable` is provided, it returns \
			     devices that are or aren't settable, depending on the \
			     value of the agument.\n\n\
			     \
			     NOTE: At this point, the only supported pattern is the \
			     entire device name. Proper pattern handling will be \
			     added soon."
    )]
    async fn device_info(
        #[graphql(context)] db: &ConfigDb,
        #[graphql(
            name = "pattern",
            description = "If this argument is provided, the query returns information \
			   for devices whose name matches the pattern. The pattern uses \
			   \"globbing\" grammar: '?' matches one character, '*' matches \
			   zero or more, '**' matches arbtrary levels of the path \
			   (between ':'s)."
        )]
        pattern: Option<String>,
        #[graphql(
            name = "settable",
            description = "If this argument is provided, the query filters the result \
			   based on whether the device can be set or not."
        )]
        settable: Option<bool>,
    ) -> result::Result<Vec<DeviceInfo>, FieldError> {
        let tx = db.1.clone();
        let filt = settable
            .map(|v| {
                if v {
                    Config::is_settable
                } else {
                    Config::is_not_settable
                }
            })
            .unwrap_or(Config::is_true);

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
                        db: db.0.clone(),
                    })
                    .collect()
            })
            .map_err(|_| {
                FieldError::new("error looking-up device", Value::null())
            })
    }
}

struct EditConfig;

#[juniper::graphql_object(context = ConfigDb)]
impl EditConfig {
    fn mod_redis(_param: String) -> result::Result<bool, FieldError> {
        Err(FieldError::new("not implemented", Value::null()))
    }
}

// The `Control` mutation is used to group queries that attempt to
// control devices by sending them settings.

struct Control;

impl Control {
    // Sends a new value to a device.

    async fn perform_setting<
        T: Into<device::Value> + TryFrom<device::Value, Error = Error>,
    >(
        db: &ConfigDb, device: &str, value: T,
    ) -> result::Result<T, FieldError> {
        // Make sure the device name is properly formed.

        if let Ok(name) = device.parse::<device::Name>() {
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
}

#[juniper::graphql_object(
    context = ConfigDb,
    description = "This group of queries perform modifications to devices."
)]
impl Control {
    #[graphql(description = "Submits `value` to be applied to the device \
			     associated with the given `name`. If the data \
			     is in a format the device doesn't support an \
			     error is returned. The `value` parameter \
			     contains several fields. Only one should be \
			     set. It is an error to have all fields `null` \
			     or more than one field non-`null`.")]
    async fn set_device(
        #[graphql(context)] db: &ConfigDb, name: String, value: SettingData,
    ) -> FieldResult<Option<bool>> {
        match value {
            SettingData {
                f_int: None,
                f_float: None,
                f_bool: None,
                f_string: None,
            } => Err(FieldError::new("no data provided", Value::null())),

            SettingData {
                f_int: Some(v),
                f_float: None,
                f_bool: None,
                f_string: None,
            } => Control::perform_setting(db, &name, v).await.map(|_| None),

            SettingData {
                f_int: None,
                f_float: Some(v),
                f_bool: None,
                f_string: None,
            } => Control::perform_setting(db, &name, v).await.map(|_| None),

            SettingData {
                f_int: None,
                f_float: None,
                f_bool: Some(v),
                f_string: None,
            } => Control::perform_setting(db, &name, v).await.map(|_| None),

            SettingData {
                f_int: None,
                f_float: None,
                f_bool: None,
                f_string: Some(v),
            } => Control::perform_setting(db, &name, v).await.map(|_| None),

            SettingData { .. } => Err(FieldError::new(
                "must only specify one item of data",
                Value::null(),
            )),
        }
    }
}

struct MutRoot;

#[juniper::graphql_object(context = ConfigDb)]
impl MutRoot {
    fn config() -> EditConfig {
        EditConfig
    }
    fn control() -> Control {
        Control
    }
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
    string_value: Option<String>,
}

struct Subscription;

impl Subscription {
    fn xlat(
        name: String,
    ) -> impl Fn(
        device::Reading,
    ) -> FieldResult<Reading> {
        move |e: device::Reading| {
            let mut reading = Reading {
                device: name.clone(),
                stamp: DateTime::<Utc>::from(e.ts),
                bool_value: None,
                int_value: None,
                float_value: None,
                string_value: None,
            };

            match e.value {
                device::Value::Bool(v) => reading.bool_value = Some(v),
                device::Value::Int(v) => reading.int_value = Some(v),
                device::Value::Flt(v) => reading.float_value = Some(v),
                device::Value::Str(v) => reading.string_value = Some(v),
            }

            Ok(reading)
        }
    }
}

#[graphql_subscription(context = ConfigDb)]
impl Subscription {
    #[graphql(description = "Sets up a connection to receive all \
			     updates to a device. The GraphQL request \
			     must provide the name of a device. This \
			     method returns a stream which generates a \
			     reply each time a device's value changes.")]
    async fn monitor_device(
        #[graphql(context)] db: &ConfigDb, device: String,
    ) -> device::DataStream<FieldResult<Reading>> {
        use tokio_stream::StreamExt;

        if let Ok(name) = device.parse::<device::Name>() {
            info!("setting monitor for '{}'", &name);

            if let Ok(rx) = db.1.monitor_device(name.clone()).await {
                let stream = StreamExt::map(rx, Subscription::xlat(device));

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

type Schema = RootNode<'static, Config, MutRoot, Subscription>;

fn schema() -> Schema {
    Schema::new(Config {}, MutRoot {}, Subscription {})
}

pub fn server(
    db: crate::driver::DriverDb, cchan: client::RequestChan,
) -> impl Future<Output = ()> {
    let context = ConfigDb(db, cchan);

    // Create filter that handles GraphQL queries and mutations.

    let ctxt = context.clone();
    let state = warp::any().map(move || ctxt.clone());
    let graphql_filter =
        juniper_warp::make_graphql_filter(schema(), state.boxed());

    // Create filter that handle the interactive GraphQL app.

    let graphiql_filter = playground_filter("/graphql", Some("/subscriptions"));

    // Create the filter that handles subscriptions.

    let sub_filter = warp::ws()
        .and(warp::addr::remote())
        .map(
            move |ws: warp::ws::Ws, addr: Option<std::net::SocketAddr>| {
                let ctxt = context.clone();
                let root_node = schema();

                ws.on_upgrade(move |websocket| {
                    async move {
                        info!("subscription context created");

                        serve_graphql_ws(
                            websocket,
                            Arc::new(root_node),
                            ConnectionConfig::new(ctxt.clone()),
                        )
                        .map(|r| {
                            if let Err(e) = r {
                                error!("Websocket error: {}", &e);
                            }
                        })
                        .await;

                        info!("subscription context canceled")
                    }
                    .instrument(info_span!(
                        "ws",
                        client = addr
                            .map(|v| v.to_string())
                            .unwrap_or_else(|| String::from("*unknown*"))
                            .as_str()
                    ))
                })
            },
        )
        .map(|reply| {
            warp::reply::with_header(
                reply,
                "Sec-WebSocket-Protocol",
                "graphql-ws",
            )
        });

    // Stitch the filters together to build the map of the web
    // interface.

    let filter = (warp::path("graphiql").and(graphiql_filter))
        .or(warp::path("subscriptions").and(sub_filter))
        .or(warp::path("graphql").and(graphql_filter));

    warp::serve(filter).run(([0, 0, 0, 0], 3000))
}

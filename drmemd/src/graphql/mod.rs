use chrono::prelude::*;
use drmem_api::{client, device, Error};
use futures::Future;
use juniper::{
    self, executor::FieldError, graphql_subscription, graphql_value,
    FieldResult, GraphQLInputObject, GraphQLObject, RootNode, Value,
};
use juniper_graphql_ws::ConnectionConfig;
use juniper_warp::subscriptions::serve_graphql_ws;
use libmdns::Responder;
use std::{result, sync::Arc, time::Duration};
use tracing::{info, info_span};
use tracing_futures::Instrument;
use warp::Filter;

pub mod config;

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

// Contains information about a device's history in the backend.

#[derive(GraphQLObject)]
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

struct DeviceInfo {
    device_name: String,
    units: Option<String>,
    settable: bool,
    driver_name: String,
    history: DeviceHistory,
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
    fn units(&self) -> Option<&String> {
        self.units.as_ref()
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
    fn driver(&self) -> DriverInfo {
        self.db
            .get_driver(&self.driver_name)
            .map(|di| DriverInfo {
                name: self.driver_name.clone(),
                summary: di.0,
                description: di.1,
            })
            .unwrap()
    }

    fn history(&self) -> &DeviceHistory {
        &self.history
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
        #[graphql(context)] db: &ConfigDb,
        name: Option<String>,
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
		     value of the agument."
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

struct Control;

impl Control {
    // Sends a new value to a device.

    async fn perform_setting<
        T: Into<device::Value> + TryFrom<device::Value, Error = Error>,
    >(
        db: &ConfigDb,
        device: &str,
        value: T,
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
        #[graphql(context)] db: &ConfigDb,
        name: String,
        value: SettingData,
    ) -> FieldResult<Reading> {
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
            } => {
                Control::perform_setting(db, &name, v)
                    .await
                    .map(|v| Reading {
                        device: name,
                        stamp: Utc::now(),
                        int_value: Some(v),
                        float_value: None,
                        bool_value: None,
                        string_value: None,
                    })
            }

            SettingData {
                f_int: None,
                f_float: Some(v),
                f_bool: None,
                f_string: None,
            } => {
                Control::perform_setting(db, &name, v)
                    .await
                    .map(|v| Reading {
                        device: name,
                        stamp: Utc::now(),
                        int_value: None,
                        float_value: Some(v),
                        bool_value: None,
                        string_value: None,
                    })
            }

            SettingData {
                f_int: None,
                f_float: None,
                f_bool: Some(v),
                f_string: None,
            } => {
                Control::perform_setting(db, &name, v)
                    .await
                    .map(|v| Reading {
                        device: name,
                        stamp: Utc::now(),
                        int_value: None,
                        float_value: None,
                        bool_value: Some(v),
                        string_value: None,
                    })
            }

            SettingData {
                f_int: None,
                f_float: None,
                f_bool: None,
                f_string: Some(v),
            } => {
                Control::perform_setting(db, &name, v)
                    .await
                    .map(|v| Reading {
                        device: name,
                        stamp: Utc::now(),
                        int_value: None,
                        float_value: None,
                        bool_value: None,
                        string_value: Some(v),
                    })
            }

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
    #[graphql(description = "The start of the date range (in UTC.) If \
			     `null`, it means \"now\".")]
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
    string_value: Option<String>,
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
            },
            device::Value::Int(v) => Reading {
                device: "".into(),
                stamp: DateTime::<Utc>::from(value.ts),
                int_value: Some(*v),
                float_value: None,
                bool_value: None,
                string_value: None,
            },
            device::Value::Flt(v) => Reading {
                device: "".into(),
                stamp: DateTime::<Utc>::from(value.ts),
                int_value: None,
                float_value: Some(*v),
                bool_value: None,
                string_value: None,
            },
            device::Value::Str(v) => Reading {
                device: "".into(),
                stamp: DateTime::<Utc>::from(value.ts),
                int_value: None,
                float_value: None,
                bool_value: None,
                string_value: Some(v.to_string()),
            },
        }
    }
}

struct Subscription;

impl Subscription {
    fn xlat(name: String) -> impl Fn(device::Reading) -> FieldResult<Reading> {
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
        #[graphql(context)] db: &ConfigDb,
        device: String,
        range: Option<DateRange>,
    ) -> device::DataStream<FieldResult<Reading>> {
        use tokio_stream::StreamExt;

        if let Ok(name) = device.parse::<device::Name>() {
            info!("setting monitor for '{}'", &name);

            let start = range.as_ref().and_then(|v| v.start);
            let end = range.as_ref().and_then(|v| v.end);

            if let Ok(rx) = db.1.monitor_device(name.clone(), start, end).await
            {
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

type Schema = RootNode<'static, Config, Control, Subscription>;

fn schema() -> Schema {
    Schema::new(Config {}, Control {}, Subscription {})
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

pub fn server(
    cfg: &config::Config,
    db: crate::driver::DriverDb,
    cchan: client::RequestChan,
) -> impl Future<Output = ()> {
    #[cfg(feature = "graphiql")]
    use juniper_warp::playground_filter;

    let context = ConfigDb(db, cchan);

    // Create filter that handles GraphQL queries and mutations.

    let ctxt = context.clone();
    let state = warp::any().map(move || ctxt.clone());
    let query_filter = warp::path(paths::QUERY)
        .and(warp::path::end())
        .and(warp::post().or(warp::get()).unify())
        .and(juniper_warp::make_graphql_filter(schema(), state.boxed()));

    // Create filter that handle the interactive GraphQL app. This
    // service is found at the BASE path.

    #[cfg(feature = "graphiql")]
    let graphiql_filter = warp::path::end().and(playground_filter(
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

                ws.on_upgrade(move |websocket| {
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
                })
            },
        );

    #[cfg(feature = "graphiql")]
    let site = query_filter.or(graphiql_filter).or(sub_filter);

    #[cfg(not(feature = "graphiql"))]
    let site = query_filter.or(sub_filter);

    // Stitch the filters together to build the map of the web
    // interface.

    let filter = warp::path(paths::BASE)
        .and(site)
        .with(warp::log("gql::drmem"))
        .with(
            warp::cors()
                .allow_any_origin()
                .allow_headers(vec![
                    "content-type",
                    "Access-Control-Allow-Origin",
                ])
                .allow_methods(vec!["OPTIONS", "GET", "POST"])
                .max_age(Duration::from_secs(3_600)),
        );

    // Create the background mDNS task.

    let (resp, task) = Responder::with_default_handle().unwrap();

    // Bind to the address.

    let (addr, http_task) = warp::serve(filter).bind_ephemeral(cfg.addr);

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

    // If the configuration specifies a preferred address to use, add
    // it to the payload.

    if let Some(host) = &cfg.pref_host {
        info!("adding preferred address: {}:{}", &host, cfg.pref_port);
        payload.push(format!("pref-addr={}:{}", &host, cfg.pref_port))
    }

    // Register DrMem's mDNS entry. In the properties field, inform
    // the client with which paths to use for each GraphQL query
    // type.

    let service = resp.register(
        "_drmem._tcp".into(),
        cfg.name.clone(),
        addr.port(),
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

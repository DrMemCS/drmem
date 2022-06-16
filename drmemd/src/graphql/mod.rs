use drmem_api::{
    client,
    types::{device, Error},
};
use futures::TryFutureExt;
use hyper::service::{make_service_fn, service_fn};
use hyper::{server::Server, Body, Method, Response, StatusCode};
use juniper::{
    self, FieldResult, executor::FieldError, graphql_value, EmptySubscription,
    GraphQLInputObject, RootNode, Value,
};
use std::{convert::Infallible, result, sync::Arc};
use tracing::Instrument;
use tracing::{error, info_span};

// The Context parameter for Queries.

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
struct Data {
    #[graphql(name = "int", description = "Placeholder for integer values.")]
    f_int: Option<i32>,
    #[graphql(name = "flt", description = "Placeholder for float values.")]
    f_float: Option<f64>,
    #[graphql(name = "bool", description = "Placeholder for boolean values.")]
    f_bool: Option<bool>,
    #[graphql(name = "str", description = "Placeholder for string values.")]
    f_string: Option<String>,
}

//mod data;

// `DeviceInfo` is a GraphQL object which contains information about a
// device.

struct DeviceInfo {
    device_name: String,
    units: Option<String>,
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

    #[graphql(description = "Returns information about devices that match \
			     the specified `pattern`. If no pattern is \
			     provided, all devices are returned.\n\n\
			     NOTE: At this point, the only supported pattern \
			     is the entire device name. Proper pattern \
			     handling will be added soon.")]
    async fn device_info(
        #[graphql(context)] db: &ConfigDb, pattern: Option<String>,
    ) -> result::Result<Vec<DeviceInfo>, FieldError> {
        let tx = db.1.clone();

        tx.get_device_info(pattern)
            .await
            .map(|v| {
                v.iter()
                    .map(|e| DeviceInfo {
                        device_name: e.name.to_string(),
                        units: e.units.clone(),
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

struct Control;

impl Control {
    async fn perform_setting<
        T: Into<device::Value> + TryFrom<device::Value, Error = Error>,
    >(
        db: &ConfigDb, device: &str, value: T,
    ) -> result::Result<T, FieldError> {
        if let Ok(name) = device.parse::<device::Name>() {
            let tx = db.1.clone();

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
    description = "These queries allow devices to be modified."
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
        #[graphql(context)] db: &ConfigDb, name: String, value: Data,
    ) -> FieldResult<Option<bool>> {
        match value {
            Data {
                f_int: None,
                f_float: None,
                f_bool: None,
                f_string: None,
            } => Err(FieldError::new("no data provided", Value::null())),

            Data {
                f_int: Some(v),
                f_float: None,
                f_bool: None,
                f_string: None,
            } => Control::perform_setting(db, &name, v).await.map(|_| None),

            Data {
                f_int: None,
                f_float: Some(v),
                f_bool: None,
                f_string: None,
            } => Control::perform_setting(db, &name, v).await.map(|_| None),

            Data {
                f_int: None,
                f_float: None,
                f_bool: Some(v),
                f_string: None,
            } => Control::perform_setting(db, &name, v).await.map(|_| None),

            Data {
                f_int: None,
                f_float: None,
                f_bool: None,
                f_string: Some(v),
            } => Control::perform_setting(db, &name, v).await.map(|_| None),

            Data { .. } => Err(FieldError::new(
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

pub async fn server(
    db: crate::driver::DriverDb, cchan: client::RequestChan,
) -> Result<Infallible, Error> {
    let addr = ([0, 0, 0, 0], 3000).into();
    let db = Arc::new(ConfigDb(db, cchan));

    loop {
        let root_node =
            Arc::new(RootNode::new(Config, MutRoot, EmptySubscription::new()));
        let db = db.clone();
        let make_svc = make_service_fn(move |_| {
            let root_node = root_node.clone();
            let ctx = db.clone();

            async {
                Ok::<_, Infallible>(service_fn(move |req| {
                    let root_node = root_node.clone();
                    let ctx = ctx.clone();

                    async {
			match (req.method(), req.uri().path()) {
                            (&Method::GET, "/") => {
				let resp =
                                    juniper_hyper::graphiql("/graphql", None).await;

				Ok::<_, hyper::Error>(resp)
                            }

                            (&Method::GET, "/graphql")
                            | (&Method::POST, "/graphql") => {
                                Ok::<_, hyper::Error>(
                                    juniper_hyper::graphql(root_node, ctx, req)
                                        .instrument(info_span!("graphql"))
                                        .await,
                                )
                            }

                            _ => {
                                let mut resp = Response::new(Body::empty());

                                *resp.status_mut() = StatusCode::NOT_FOUND;
                                Ok::<_, hyper::Error>(resp)
                            }
                        }
                    }
                }))
            }
        });

        Server::bind(&addr)
            .serve(make_svc)
            .map_err(|e| {
                error!("web server stopped -- {}", &e);
                Error::UnknownError
            })
            .await?
    }
}

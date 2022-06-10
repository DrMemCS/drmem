use drmem_api::{client, types::Error};
use futures::TryFutureExt;
use hyper::service::{make_service_fn, service_fn};
use hyper::{server::Server, Body, Method, Response, StatusCode};
use juniper::{
    self, executor::FieldError, graphql_value, EmptySubscription, RootNode,
    Value,
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

#[juniper::graphql_object(
    context = ConfigDb,
    description = "These queries allow devices to be modified."
)]
impl Control {
    #[graphql(description = "Changes the value of a device to the specified, \
		       boolean value. If the device doesn't accept boolean \
		       values, an error is returned. This query will return \
		       the value that was set, which might not be the same \
		       value specified. For instance, hardware may be in a \
		       \"locked\" state and so a device can't be set to \
		       `true`. In a case like that, `false` would be returned.")]
    fn set_boolean(
        _device: String, _value: bool,
    ) -> result::Result<bool, FieldError> {
        Err(FieldError::new("not implemented", Value::null()))
    }

    #[graphql(description = "Changes the value of a device to the specified, \
		       integer value. If the device doesn't accept integer \
		       values, an error is returned. This query returns the \
		       actual value used by the driver, which may not be \
		       the same value specified. For instance, if the device \
		       only accepts a range of values, some drivers may \
		       return an error and others might clip the setting to \
		       keep it in range.")]
    fn set_integer(
        _device: String, _value: i32,
    ) -> result::Result<i32, FieldError> {
        Err(FieldError::new("not implemented", Value::null()))
    }

    #[graphql(description = "Changes the value of a device to the specified, \
		       floating point value. If the device doesn't accept \
		       floating point values, an error is returned. This \
		       query returns the actual value used by the driver, \
		       which may not be the same value specified. For \
		       instance, if the device only accepts a range of \
		       values, some drivers may return an error and others \
		       might clip the setting to keep it in range.")]
    fn set_float(
        _device: String, _value: f64,
    ) -> result::Result<f64, FieldError> {
        Err(FieldError::new("not implemented", Value::null()))
    }

    #[graphql(description = "Changes the value of a device to the specified, \
		       string value. If the device doesn't accept string \
		       values, an error is returned.")]
    fn set_string(
        _device: String, _value: String,
    ) -> result::Result<String, FieldError> {
        Err(FieldError::new("not implemented", Value::null()))
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
) -> Result<(), Error> {
    let addr = ([0, 0, 0, 0], 3000).into();
    let root_node =
        Arc::new(RootNode::new(Config, MutRoot, EmptySubscription::new()));
    let db = Arc::new(ConfigDb(db, cchan));

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
                        | (&Method::POST, "/graphql") => Ok::<_, hyper::Error>(
                            juniper_hyper::graphql(root_node, ctx, req)
                                .instrument(info_span!("graphql"))
                                .await,
                        ),

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
        .await
}

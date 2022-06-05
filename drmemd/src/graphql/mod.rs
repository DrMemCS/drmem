use drmem_api::types::Error;
use futures::TryFutureExt;
use hyper::service::{make_service_fn, service_fn};
use hyper::{server::Server, Body, Method, Response, StatusCode};
use juniper::{
    executor::FieldError, graphql_value, EmptySubscription, GraphQLObject,
    RootNode, Value,
};
use std::{convert::Infallible, result, sync::Arc};
use tracing::Instrument;
use tracing::{error, info, info_span};

#[derive(GraphQLObject)]
#[graphql(description = "Information about the available drivers in the \
			 running version of `drmemd`.")]
struct DriverInfo {
    #[graphql(description = "The name of the driver.")]
    name: String,
    #[graphql(description = "A short summary of the driver's purpose.")]
    summary: String,
    #[graphql(description = "Detailed information about the driver: the \
			     configuration parameters; the devices it \
			     registers; and other pertinent information. \
			     This information is formatted in Markdown.")]
    description: String,
}

#[derive(GraphQLObject)]
#[graphql(description = "Encapsulates the information of a device reading.")]
struct Reading {
    #[graphql(description = "The time at which the value was read. This \
			     value is given in UTC.")]
    timestamp: f64,
    #[graphql(description = "The value of the device.")]
    value: f64,
}

#[derive(GraphQLObject)]
#[graphql(description = "Information about registered devices in the running \
			 version of `drmemd`.")]
struct DeviceInfo {
    #[graphql(description = "The name of the device.")]
    name: String,
    #[graphql(description = "The engineering units of the device's value.")]
    units: Option<String>,
    #[graphql(description = "The current value of the device. If there \
			     hasn't been a reading yet, this will be set \
			     to `null`.")]
    current: Option<Reading>,
    #[graphql(description = "Information of the driver which supports this \
			     device.")]
    driver: DriverInfo,
}

impl juniper::Context for crate::driver::DriverDb {}

/// can you read this?
struct Config;

#[juniper::graphql_object(context = crate::driver::DriverDb)]
#[graphql(description = "Reports configuration information for `drmemd`.")]
impl Config {
    #[graphql(description = "Returns information about the available drivers \
			     in the running instance of `drmemd`. If `name` \
			     isn't provided, an array of all driver \
			     information is returned. If `name` is specified \
			     and a driver with that name exists, a single \
			     element array is returned. Otherwise `null` is \
			     returned.")]
    fn driver_info(
        #[graphql(context)] db: &crate::driver::DriverDb,
        #[graphql(description = "An optional argument which, when provided, \
				 only returns driver information whose name \
				 matches. If this argument isn't provided, \
				 all driver's information will be returned.")]
        name: Option<String>,
    ) -> result::Result<Vec<DriverInfo>, FieldError> {
        info!("driver_info({:?})", &name);

        if let Some(name) = name {
            if let Some((n, s, d)) = db.find(&name) {
                Ok(vec![DriverInfo {
                    name: n,
                    summary: s.to_string(),
                    description: d.to_string(),
                }])
            } else {
                Err(FieldError::new(
                    "driver not found",
                    graphql_value!({ "missing_driver": name }),
                ))
            }
        } else {
            let result = db
                .get_all()
                .map(|(n, s, d)| DriverInfo {
                    name: n,
                    summary: s.to_string(),
                    description: d.to_string(),
                })
                .collect();

            Ok(result)
        }
    }

    #[graphql(description = "Returns information about devices that match \
			     the specified `pattern`. If no pattern is \
			     provided, all devices are returned.")]
    fn device_info(
        _pattern: Option<String>,
    ) -> result::Result<Vec<DeviceInfo>, FieldError> {
        Err(FieldError::new("not implemented", Value::null()))
    }
}

struct EditConfig;

#[juniper::graphql_object(context = crate::driver::DriverDb)]
impl EditConfig {
    fn mod_redis(_param: String) -> result::Result<bool, FieldError> {
        Err(FieldError::new("not implemented", Value::null()))
    }
}

struct Control;

#[juniper::graphql_object(context = crate::driver::DriverDb)]
impl Control {
    fn modify_device(
        _device: String, _value: f64,
    ) -> result::Result<bool, FieldError> {
        Err(FieldError::new("not implemented", Value::null()))
    }
}

struct MutRoot;

#[juniper::graphql_object(context = crate::driver::DriverDb)]
impl MutRoot {
    fn config() -> EditConfig {
        EditConfig
    }
    fn control() -> Control {
        Control
    }
}

pub async fn server(db: crate::driver::DriverDb) -> Result<(), Error> {
    let addr = ([0, 0, 0, 0], 3000).into();
    let root_node =
        Arc::new(RootNode::new(Config, MutRoot, EmptySubscription::new()));
    let db = Arc::new(db);

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

use hyper::service::{make_service_fn, service_fn};
use hyper::{server::Server, Body, Response, StatusCode};
use std::convert::Infallible;

pub async fn server() -> Result<(), hyper::Error> {
    let addr = ([0, 0, 0, 0], 3000).into();

    // A `Service` is needed for every connection, so this creates one
    // from our `hello_world` function.

    let make_svc = make_service_fn(|_| async {
        Ok::<_, Infallible>(service_fn(|req| async move {
            match (req.method(), req.uri().path()) {
                //(&Method::GET, "/") =>
                //    juniper_hyper::graphiql("/graphql", None).await,
                _ => {
                    let mut response = Response::new(Body::empty());

                    *response.status_mut() = StatusCode::NOT_FOUND;
                    Ok::<_, hyper::Error>(response)
                }
            }
        }))
    });

    Server::bind(&addr).serve(make_svc).await
}

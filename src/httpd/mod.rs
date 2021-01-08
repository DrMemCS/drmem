use std::convert::Infallible;
use std::net::SocketAddr;
use hyper::{Body, Request, Response, Server};
use hyper::service::{make_service_fn, service_fn};

async fn hello_world(_req: Request<Body>)
		     -> Result<Response<Body>, Infallible> {
    Ok(Response::new("Hello, World, from Dr Memory!".into()))
}

pub async fn server() -> Result<(), hyper::Error> {
    let addr = SocketAddr::from(([0, 0, 0, 0], 3000));

    // A `Service` is needed for every connection, so this creates one
    // from our `hello_world` function.

    let make_svc = make_service_fn(|_conn| async {

        // service_fn converts our function into a `Service`

	Ok::<_, Infallible>(service_fn(hello_world))
    });

    Server::bind(&addr).serve(make_svc).await
}

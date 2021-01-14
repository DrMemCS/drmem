use hyper::{Body, Method, Response, Server, StatusCode};
use hyper::service::{make_service_fn, service_fn};

pub async fn server() -> Result<(), hyper::Error> {
    let addr = ([0, 0, 0, 0], 3000).into();

    // A `Service` is needed for every connection, so this creates one
    // from our `hello_world` function.

    let make_svc = make_service_fn(|_| async {
	Ok::<_, hyper::Error>(service_fn(|req| async move {
	    match (req.method(), req.uri().path()) {
		(&Method::GET, "/") =>
		    juniper_hyper::graphiql("/graphql", None).await,
		_ => {
		    let mut response = Response::new(Body::empty());

		    *response.status_mut() = StatusCode::NOT_FOUND;
		    Ok(response)
		}
	    }
	}))
    });

    Server::bind(&addr).serve(make_svc).await
}

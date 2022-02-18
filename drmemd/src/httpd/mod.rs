// Copyright (c) 2021-2022, Richard M Neswold, Jr.
// All rights reserved.
//
// Redistribution and use in source and binary forms, with or without
// modification, are permitted provided that the following conditions are
// met:
//
// 1. Redistributions of source code must retain the above copyright
//    notice, this list of conditions and the following disclaimer.
//
// 2. Redistributions in binary form must reproduce the above copyright
//    notice, this list of conditions and the following disclaimer in the
//    documentation and/or other materials provided with the distribution.
//
// 3. Neither the name of the copyright holder nor the names of its
//    contributors may be used to endorse or promote products derived
//    from this software without specific prior written permission.
//
// THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS
// "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT
// LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR
// A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT
// HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
// SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT
// LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE,
// DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY
// THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT
// (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE
// OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

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

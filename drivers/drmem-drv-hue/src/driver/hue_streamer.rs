use super::payload;
use drmem_api::{Error, Result};
use futures_util::StreamExt;
use reqwest::{
    self, Response,
    header::{HeaderMap, HeaderValue},
};
use std::{convert::Infallible, sync::Arc};
use tokio::task::JoinHandle;
use tracing::{Instrument, error, info, info_span};

// Builds the reqwest::Client that will make stream connection to the Hue hub.

fn build_client(
    host: Arc<str>,
    app_key: Arc<str>,
) -> Result<impl Future<Output = std::result::Result<Response, reqwest::Error>>>
{
    let url = format!("https://{}/eventstream/clip/v2", host);

    // Hue Hubs use self-signed certificates. Disable certificate validation
    // to connect.

    let client = reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .build()
        .map_err(|e| {
            Error::OperationError(format!("error building hue stream -- {}", e))
        })?;

    // Add our application's ID. This is obtained by pressing the button on the Hue
    // hub and responding to the annoucement.

    let mut headers = HeaderMap::new();

    headers.insert(
        "hue-application-key",
        HeaderValue::from_str(&app_key).map_err(|e| {
            Error::OperationError(format!("error adding HTTP header -- {}", e))
        })?,
    );
    headers.insert("Accept", HeaderValue::from_static("text/event-stream"));

    Ok(client.get(url).headers(headers).send())
}

// Host: 192.168.1.110
// App Id: pwENI2weqIJMe4eKlajSry9qgR14VENEvrJKcofR

async fn server(host: Arc<str>, app_key: Arc<str>) -> Result<Infallible> {
    // This function should never exit.

    loop {
        let mut client = build_client(host.clone(), app_key.clone())?
            .await
            .map_err(|e| {
                Error::OperationError(format!(
                    "error setting up hue stream -- {}",
                    e
                ))
            })?
            .bytes_stream();

        // Iterate over the stream as chunks of data arrive
        while let Some(item) = client.next().await {
            let chunk = match item {
                Ok(item) => item,
                Err(e) => {
                    error!("{}", e);
                    break;
                }
            };
            let text = String::from_utf8_lossy(&chunk);

            // SSE data usually starts with "data: "
            for line in text.lines() {
                if line.starts_with("data: ") {
                    let json_str = &line[6..];
                    // Inside the stream loop...
                    if let Ok(events) =
                        serde_json::from_str::<Vec<payload::HueEvent>>(json_str)
                    {
                        for event in events {
                            for resource in event.data {
                                if resource.res_type == "light" {
                                    info!(
                                        "Update for Light ID: {}",
                                        resource.id
                                    );

                                    if let Some(on) = resource.on {
                                        info!(
                                            "  - Power: {}",
                                            if on.on { "ON" } else { "OFF" }
                                        );
                                    }

                                    if let Some(dim) = resource.dimming {
                                        info!(
                                            "  - Brightness: {}%",
                                            dim.brightness
                                        );
                                    }

                                    if let Some(col) = resource.color {
                                        info!(
                                            "  - Color XY: [{}, {}]",
                                            col.xy.x, col.xy.y
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

pub async fn start(
    host: Arc<str>,
    app_key: Arc<str>,
) -> JoinHandle<Result<Infallible>> {
    tokio::spawn(
        server(host.clone(), app_key)
            .instrument(info_span!("hue_streamer", hub = host.to_string())),
    )
}

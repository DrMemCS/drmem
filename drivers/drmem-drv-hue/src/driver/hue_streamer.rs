use super::payload;
use drmem_api::{Error, Result};
use futures_util::StreamExt;
use reqwest::{
    self, Client, Response,
    header::{HeaderMap, HeaderValue},
};
use std::{convert::Infallible, sync::Arc};
use tokio::{sync::mpsc, task::JoinHandle, time::Duration};
use tracing::{Instrument, error, info_span, warn};

const RETRY_INTERVAL: Duration = Duration::from_secs(5);
const MAX_RETRY_INTERVAL: Duration = Duration::from_secs(60);

// This local module creates a "coalescing" combinator so that related,
// consecutive items in the iterator can be merged.

mod coalesce {
    use super::payload;

    pub struct Iter<I: Iterator<Item = payload::ResourceData>> {
        pub inner: std::iter::Peekable<I>,
    }

    impl<I: Iterator<Item = payload::ResourceData>> Iterator for Iter<I> {
        type Item = payload::ResourceData;

        fn next(&mut self) -> Option<Self::Item> {
            // Start with the first available item

            let mut acc = self.inner.next()?;

            // While the next item exists and has the same ID, merge it

            while let Some(next_item) = self.inner.peek() {
                if next_item.id == acc.id {
                    acc.merge(self.inner.next().unwrap());
                } else {
                    break;
                }
            }

            Some(acc)
        }
    }
}

// Builds the reqwest::Client that will make stream connection to the Hue
// hub.

async fn create_stream(client: Client, host: Arc<str>) -> Result<Response> {
    let url = format!("https://{}/eventstream/clip/v2", host);
    let mut headers = HeaderMap::new();

    headers.insert("Accept", HeaderValue::from_static("text/event-stream"));

    let req = client.get(url).headers(headers);

    Ok(req.send().await.map_err(|e| {
        let err_msg = format!("error getting event stream -- {}", e);

        error!("{}", &err_msg);
        Error::OperationError(err_msg)
    })?)
}

fn process_chunk(mut buffer: String, chunk: &[u8]) -> (String, Option<String>) {
    buffer.push_str(&String::from_utf8_lossy(chunk));

    if let Some(pos) = buffer.find("\n\n") {
        let text = buffer[..pos].to_string();

        buffer.drain(..pos + 2);
        (buffer, Some(text))
    } else {
        (buffer, None)
    }
}

fn parse_events(text: String) -> Vec<payload::ResourceData> {
    coalesce::Iter {
        inner: text
            .lines()
            .filter(|line| line.starts_with("data: "))
            .filter_map(|json_str| {
                serde_json::from_str::<Vec<payload::HueEvent>>(&json_str[6..])
                    .map_err(|v| {
                        error!("couldn't parse {} -- {}", &json_str[6..], &v);
                        v
                    })
                    .ok()
            })
            .flat_map(|events| events.into_iter())
            .flat_map(|event| event.data.into_iter())
            .peekable(),
    }
    .collect()
}

// Main body of the streamer task.

async fn server(
    host: Arc<str>,
    client: Client,
    report: mpsc::Sender<payload::ResourceData>,
) -> Result<Infallible> {
    let mut retry_interval = RETRY_INTERVAL;

    loop {
        // Connect to the bridge, specifying that we want to receive an event
        // stream. If we can't connect, go to the end of the loop, wait for a
        // few seconds, and try again.

        if let Ok(mut stream) = create_stream(client.clone(), host.clone())
            .await
            .map(|resp| resp.bytes_stream())
        {
            // Reset the retry interval on a successful connection.

            retry_interval = RETRY_INTERVAL;

            // Create a buffer to hold the incoming data. The Hue bridge sends
            // data in chunks that don't necessarily align with the JSON
            // packets, so we need to accumulate the chunks until we have a
            // complete packet to parse.

            let mut buffer = String::new();

            while let Some(data) = stream.next().await {
                match data {
                    Ok(chunk) => {
                        // Process the chunk and extract any complete JSON packets
                        // that we can parse.

                        let (new_buffer, maybe_text) =
                            process_chunk(buffer, &chunk);

                        buffer = new_buffer;
                        if let Some(text) = maybe_text {
                            let resources = parse_events(text);

                            for resource in resources.into_iter() {
                                if let Err(e) = report.send(resource).await {
                                    let msg = format!(
                                        "hue driver closed event channel -- {}",
                                        e
                                    );

                                    error!("{}", &msg);
                                    return Err(Error::MissingPeer(msg));
                                }
                            }
                        }
                    }
                    Err(e) => {
                        error!("error reading from stream -- {}", e);
                        break;
                    }
                }
            }

            std::mem::drop(stream);

            warn!(
                "lost stream connection to Hue bridge ... retrying in {} seconds",
                retry_interval.as_secs()
            );
        } else {
            warn!(
                "can't connect to Hue bridge ... retrying in {} seconds",
                retry_interval.as_secs()
            );
        }
        tokio::time::sleep(retry_interval).await;

        // Double the retry interval for the next attempt, up to a maximum.

        retry_interval = retry_interval
            .checked_mul(2)
            .unwrap_or(MAX_RETRY_INTERVAL)
            .min(MAX_RETRY_INTERVAL);
    }
}

pub fn start(
    host: Arc<str>,
    client: Client,
    report: mpsc::Sender<payload::ResourceData>,
) -> JoinHandle<Result<Infallible>> {
    tokio::spawn(
        server(host.clone(), client, report).instrument(info_span!("events")),
    )
}

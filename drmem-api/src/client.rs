//! This module defines types and interfaces that clients use to
//! interact with the core of DrMem.

use crate::{
    types::{device, Error},
    Result,
};
use tokio::sync::{mpsc, oneshot};

pub struct DevInfoReply {
    pub name: device::Name,
    pub units: Option<String>,
    pub driver: String,
}

/// Defines the requests that can be sent to core.
pub enum Request {
    QueryDeviceInfo {
        pattern: Option<String>,
        rpy_chan: oneshot::Sender<Vec<DevInfoReply>>,
    },
}

/// A handle which is used to communicate with the core of DrMem.
/// Clients will be given a handle to be used throughout its life.
///
/// This type wraps the `mpsc::Sender<>` and defines a set of helper
/// methods to send requests and receive replies with the core.
#[derive(Clone)]
pub struct RequestChan {
    req_chan: mpsc::Sender<Request>,
}

impl RequestChan {
    pub fn new(req_chan: mpsc::Sender<Request>) -> Self {
        RequestChan { req_chan }
    }

    pub async fn get_device_info(
        &self, pattern: Option<String>,
    ) -> Result<Vec<DevInfoReply>> {
        let (rpy_chan, rx) = oneshot::channel();
        let result = self
            .req_chan
            .send(Request::QueryDeviceInfo { pattern, rpy_chan })
            .await;

        if result.is_ok() {
            return rx.await.map_err(|_| {
                Error::MissingPeer(String::from("core didn't reply to request"))
            });
        }
        Err(Error::MissingPeer(String::from(
            "core didn't accept request",
        )))
    }
}

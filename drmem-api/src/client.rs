//! This module defines types and interfaces that clients use to
//! interact with the core of DrMem.

use crate::{
    types::{device, Error},
    Result,
};
use tokio::sync::{broadcast, mpsc, oneshot};

#[derive(Debug, PartialEq, Eq)]
pub struct DevInfoReply {
    pub name: device::Name,
    pub units: Option<String>,
    pub settable: bool,
    pub driver: String,
}

/// Defines the requests that can be sent to core.
pub enum Request {
    QueryDeviceInfo {
        pattern: Option<String>,
        rpy_chan: oneshot::Sender<Result<Vec<DevInfoReply>>>,
    },

    SetDevice {
        name: device::Name,
        value: device::Value,
        rpy_chan: oneshot::Sender<Result<device::Value>>,
    },

    MonitorDevice {
        name: device::Name,
        rpy_chan: oneshot::Sender<Result<broadcast::Receiver<device::Reading>>>,
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

    // Makes a request to monitor a device.

    pub async fn monitor_device(
        &self, name: device::Name,
    ) -> Result<broadcast::Receiver<device::Reading>> {
        // Create our reply channel and build the request message.

        let (tx, rx) = oneshot::channel();
        let msg = Request::MonitorDevice { name, rpy_chan: tx };

        // Send the message.

        self.req_chan.send(msg).await?;

        // Wait for a reply.

        rx.await?
    }

    // Polymorphic method which requests that a device be set to the
    // provided value. The return value is the value the driver
    // actually used to set the device. Some drivers do sanity checks
    // on the set value and, if the value is unusable, the driver may
    // return an error or clip the value to something valid. The
    // driver's documentation should indicate how it handles settings.

    pub async fn set_device<
        T: Into<device::Value> + TryFrom<device::Value, Error = Error>,
    >(
        &self, name: device::Name, value: T,
    ) -> Result<T> {
        // Create the reply channel and the request message that will
        // be sent.

        let (tx, rx) = oneshot::channel();
        let msg = Request::SetDevice {
            name,
            value: value.into(),
            rpy_chan: tx,
        };

        // Send the request to the driver.

        self.req_chan.send(msg).await?;

        // Wait for the reply and try to convert the set value back
        // into the type that was used.

        rx.await?.and_then(T::try_from)
    }

    // Requests device information for devices whose name matches the
    // provided pattern.

    pub async fn get_device_info(
        &self, pattern: Option<String>,
    ) -> Result<Vec<DevInfoReply>> {
        let (rpy_chan, rx) = oneshot::channel();

        // Send the request to the service (i.e. the backend) that has
        // the device information.

        self.req_chan
            .send(Request::QueryDeviceInfo { pattern, rpy_chan })
            .await?;

        // Return the reply from the request.

        rx.await.map_err(|e| e.into()).and_then(|v| v)
    }
}

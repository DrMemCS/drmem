//! Defines types and interfaces that internal clients use to interact
//! with the core of DrMem. The primary, internal client is the
//! GraphQL interface, but asks in logic blocks also use this module.
//!
//! Any new, internal tasks that need access to device readings or
//! wish to set the value of the device need to have a
//! `client::RequestChan` handle. As DrMem starts, it should
//! `.clone()` the `RequestChan` used to communicate with the
//! back-end.
//!
//! # Example
//!
//! ```ignore
//! async fn some_new_task(handle: client::RequestChan) {
//!    // Initialize and enter loop.
//!
//!    let device = "some:device".parse::<device::Name>().unwrap();
//!
//!    loop {
//!        // Set a device value.
//!
//!        if some_condition {
//!            handle.set_device(&device, true.into())
//!        }
//!    }
//! }
//!
//! // Somewhere in DrMem start-up.
//!
//! let task = some_new_task(backend_chan.clone());
//!
//! // Add the task to the set of tasks to be awaited.
//! ```

use crate::{
    driver,
    types::{device, Error},
    Result,
};
use chrono::*;
use tokio::sync::{mpsc, oneshot};

/// Holds information about a device. A back-end is free to store this
/// information in any way it sees fit. However, it is returned for
/// GraphQL queries, so it should be reasonably efficient to assemble
/// this reply.

#[derive(Debug, PartialEq)]
pub struct DevInfoReply {
    /// The full name of the device.
    pub name: device::Name,
    /// The device's engineering units. Some devices don't use units
    /// (boolean devices are an example.)
    pub units: Option<String>,
    /// Indicates whether the device is settable.
    pub settable: bool,
    pub total_points: u32,
    pub first_point: Option<device::Reading>,
    pub last_point: Option<device::Reading>,
    /// The name of the driver that supports this device.
    pub driver: String,
}

// Defines the requests that can be sent to core.
#[doc(hidden)]
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

    GetSettingChan {
        name: device::Name,
        _own: bool,
        rpy_chan: oneshot::Sender<Result<driver::TxDeviceSetting>>,
    },

    MonitorDevice {
        name: device::Name,
        start: Option<DateTime<Utc>>,
        end: Option<DateTime<Utc>>,
        rpy_chan: oneshot::Sender<Result<device::DataStream<device::Reading>>>,
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

    /// Makes a request to monitor the device, `name`.
    ///
    /// If sucessful, a stream is returned which yields device
    /// readings as the device is updated.

    pub async fn monitor_device(
        &self,
        name: device::Name,
        start: Option<DateTime<Utc>>,
        end: Option<DateTime<Utc>>,
    ) -> Result<device::DataStream<device::Reading>> {
        // Create our reply channel and build the request message.

        let (tx, rx) = oneshot::channel();
        let msg = Request::MonitorDevice {
            name,
            rpy_chan: tx,
            start,
            end,
        };

        // Send the message.

        self.req_chan.send(msg).await?;

        // Wait for a reply.

        rx.await?
    }

    /// Requests that a device be set to a provided value.
    ///
    /// - `name` is the name of the device
    /// - `value` is the value to be set. This value can be a
    ///   `device::Value` value or can be any type that can be coerced
    ///   into one.
    ///
    /// Returns the value the driver actually used to set the device.
    /// Some drivers do sanity checks on the set value and, if the
    /// value is unusable, the driver may return an error or clip the
    /// value to something valid. The driver's documentation should
    /// indicate how it handles invalid settings.

    pub async fn set_device<
        T: Into<device::Value> + TryFrom<device::Value, Error = Error>,
    >(
        &self,
        name: device::Name,
        value: T,
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

    pub async fn get_setting_chan(
        &self,
        name: device::Name,
        own: bool,
    ) -> Result<driver::TxDeviceSetting> {
        // Create the reply channel and the request message that will
        // be sent.

        let (tx, rx) = oneshot::channel();
        let msg = Request::GetSettingChan {
            name,
            _own: own,
            rpy_chan: tx,
        };

        // Send the request to the driver.

        self.req_chan.send(msg).await?;

        // Wait for the reply and try to convert the set value back
        // into the type that was used.

        rx.await?
    }

    /// Requests device information for devices whose name matches the
    /// provided pattern.

    pub async fn get_device_info(
        &self,
        pattern: Option<String>,
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

//! This module defines types and interfaces that driver use to
//! interact with the core of DrMem.

use crate::types::{device::Value, Error};
use async_trait::async_trait;
use tokio::sync::{mpsc, oneshot};
use toml::value;

use super::Result;

pub type Config = value::Table;
pub type TxDeviceSetting = mpsc::Sender<Value>;
pub type RxDeviceSetting = mpsc::Receiver<Value>;

pub type ReportReading = Box<dyn Fn(Value) -> Result<()> + Send>;

/// Defines the requests that can be sent to core.
pub enum Request {
    /// Registers a read-only device with core.
    ///
    /// The reply will contain a channel to broadcast values read from
    /// the hardware.
    AddReadonlyDevice {
        dev_name: String,
        rpy_chan: oneshot::Sender<Result<ReportReading>>,
    },

    /// Registers a writable device with core.
    ///
    /// The reply is a pair where the first element is a channel to
    /// broadcast values read from the hardware. The second element is
    /// a read-handle to acccept incoming setting to the device.
    AddReadWriteDevice {
        dev_name: String,
        rpy_chan: oneshot::Sender<
            Result<(ReportReading, RxDeviceSetting, Option<Value>)>,
        >,
    },
}

/// A handle which is used to communicate with the core of DrMem.
/// When a driver is created, it will be given a handle to be used
/// throughout its life.
///
/// This type wraps the `mpsc::Sender<>` and defines a set of helper
/// methods to send requests and receive replies with the core.
pub struct RequestChan {
    prefix: String,
    req_chan: mpsc::Sender<Request>,
}

impl RequestChan {
    pub fn new(prefix: &str, req_chan: &mpsc::Sender<Request>) -> Self {
        RequestChan {
            prefix: String::from(prefix),
            req_chan: req_chan.clone(),
        }
    }

    /// Registers a read-only device with the framework. `name` is the
    /// last section of the full device name. Typically a driver will
    /// register several devices, each representing a portion of the
    /// hardware being controlled. All devices for a given driver
    /// instance will have the same prefix; the `name` parameter is
    /// appended to it.
    ///
    /// If it returns `Ok()`, the value is a broadcast channel that
    /// the driver uses to announce new values of the associated
    /// hardware.
    ///
    /// If it returns `Err()`, the underlying value could be `InUse`,
    /// meaning the device name is already registered. If the error is
    /// `InternalError`, then the core has exited and the
    /// `RequestChan` has been closed. Since the driver can't report
    /// any more updates, it may as well shutdown.
    pub async fn add_ro_device(
        &self, name: &str,
    ) -> super::Result<ReportReading> {
        // Create a location for the reply.

        let (tx, rx) = oneshot::channel();

        // Send a request to Core to register the given name.
        //
        // XXX: Device names should be handled more formally. This
        // code doesn't check that the names are of the correct
        // character set.

        let result = self
            .req_chan
            .send(Request::AddReadonlyDevice {
                dev_name: format!("{}:{}", self.prefix, name),
                rpy_chan: tx,
            })
            .await;

        // If the request was sent successfully and we successfully
        // received a reply, process the payload.

        if result.is_ok() {
            if let Ok(v) = rx.await {
                return v;
            } else {
                return Err(Error::MissingPeer(String::from(
                    "core didn't reply to request",
                )));
            }
        }

        // If either communication direction failed, return an error
        // indicating we can't talk to core.

        Err(Error::MissingPeer(String::from(
            "core didn't accept request",
        )))
    }

    /// Registers a read-write device with the framework. `name` is the
    /// last section of the full device name. Typically a driver will
    /// register several devices, each representing a portion of the
    /// hardware being controlled. All devices for a given driver
    /// instance will have the same prefix; the `name` parameter is
    /// appended to it.
    ///
    /// If it returns `Ok()`, the value is a pair containing a
    /// broadcast channel that the driver uses to announce new values
    /// of the associated hardware and a receive channel for incoming
    /// settings to be applied to the hardware.
    ///
    /// If it returns `Err()`, the underlying value could be `InUse`,
    /// meaning the device name is already registered. If the error is
    /// `InternalError`, then the core has exited and the
    /// `RequestChan` has been closed. Since the driver can't report
    /// any more updates or accept new settings, it may as well shutdown.
    pub async fn add_rw_device(
        &self, name: &str,
    ) -> Result<(ReportReading, mpsc::Receiver<Value>, Option<Value>)> {
        let (tx, rx) = oneshot::channel();
        let result = self
            .req_chan
            .send(Request::AddReadWriteDevice {
                dev_name: format!("{}:{}", self.prefix, name),
                rpy_chan: tx,
            })
            .await;

        if result.is_ok() {
            if let Ok(v) = rx.await {
                return v;
            } else {
                return Err(Error::MissingPeer(String::from(
                    "core didn't reply to request",
                )));
            }
        }
        Err(Error::MissingPeer(String::from(
            "core didn't accept request",
        )))
    }
}

/// All drivers implement the `driver::API` trait.
#[async_trait]
pub trait API {
    async fn create_instance(
        cfg: &Config, drc: RequestChan,
    ) -> Result<Box<dyn API + Send>>
    where
        Self: Sized;

    async fn run(&mut self) -> Result<()>;

    /// The name of the driver. This should be relatively short, but
    /// needs to be unique across all drivers.
    fn name(&self) -> &'static str;

    /// A detailed description of the driver. The format of the string
    /// should be markdown. The description should include any
    /// configuration parameter needed in the TOML configuration
    /// file. It should also mention the endpoints provided by the
    /// driver.
    fn description(&self) -> &'static str;

    /// A short, one-line summary of the driver.
    fn summary(&self) -> &'static str;
}

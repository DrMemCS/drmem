//! This module defines types and interfaces that drivers use to
//! interact with the core of DrMem.

use crate::types::{
    device::{Base, Name, Path, Value},
    Error,
};
use std::future::Future;
use std::{convert::Infallible, pin::Pin};
use tokio::sync::{mpsc, oneshot};
use toml::value;

use super::Result;

/// Represents how configuration information is given to a driver.
/// Since each driver can have vastly different requirements, the
/// config structure needs to be as general as possible. A
/// `DriverConfig` type is a map with `String` keys and `toml::Value`
/// values.
pub type DriverConfig = value::Table;

/// Used by client APIs to send setting requests to a driver.
pub type TxDeviceSetting =
    mpsc::Sender<(Value, oneshot::Sender<Result<Value>>)>;

/// Used by a driver to receive settings from a client.
pub type RxDeviceSetting =
    mpsc::Receiver<(Value, oneshot::Sender<Result<Value>>)>;

/// A function that drivers use to report updated values of a device.
pub type ReportReading<T> = Box<
    dyn Fn(T) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>>
        + Send
        + Sync
        + 'static,
>;

/// Defines the requests that can be sent to core. Drivers don't use
/// this type directly. They are indirectly used by `RequestChan`.
pub enum Request {
    /// Registers a read-only device with core.
    ///
    /// The reply will contain a channel to broadcast values read from
    /// the hardware.
    AddReadonlyDevice {
        driver_name: String,
        dev_name: Name,
        dev_units: Option<String>,
        max_history: Option<usize>,
        rpy_chan:
            oneshot::Sender<Result<(ReportReading<Value>, Option<Value>)>>,
    },

    /// Registers a writable device with core.
    ///
    /// The reply is a pair where the first element is a channel to
    /// broadcast values read from the hardware. The second element is
    /// a read-handle to acccept incoming setting to the device.
    AddReadWriteDevice {
        driver_name: String,
        dev_name: Name,
        dev_units: Option<String>,
        max_history: Option<usize>,
        rpy_chan: oneshot::Sender<
            Result<(ReportReading<Value>, RxDeviceSetting, Option<Value>)>,
        >,
    },
}

/// A handle which is used to communicate with the core of DrMem.
/// When a driver is created, it will be given a handle to be used
/// throughout its life.
///
/// This type wraps the `mpsc::Sender<>` and defines a set of helper
/// methods to send requests and receive replies with the core.
#[derive(Clone)]
pub struct RequestChan {
    driver_name: String,
    prefix: Path,
    req_chan: mpsc::Sender<Request>,
}

impl RequestChan {
    pub fn new(
        driver_name: &str, prefix: &Path, req_chan: &mpsc::Sender<Request>,
    ) -> Self {
        RequestChan {
            driver_name: String::from(driver_name),
            prefix: prefix.clone(),
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
    pub async fn add_ro_device<T: Into<Value> + TryFrom<Value>>(
        &self, name: Base, units: Option<&str>, max_history: Option<usize>,
    ) -> super::Result<(ReportReading<T>, Option<T>)> {
        // Create a location for the reply.

        let (tx, rx) = oneshot::channel();

        // Send a request to Core to register the given name.

        let result = self
            .req_chan
            .send(Request::AddReadonlyDevice {
                driver_name: self.driver_name.clone(),
                dev_name: Name::build(self.prefix.clone(), name),
                dev_units: units.map(String::from),
                max_history,
                rpy_chan: tx,
            })
            .await;

        // If the request was sent successfully and we successfully
        // received a reply, process the payload.

        if result.is_ok() {
            match rx.await {
                Ok(Ok((rr, prev))) => Ok((
                    Box::new(move |a| rr(a.into())),
                    prev.and_then(|v| T::try_from(v).ok()),
                )),
                Ok(Err(e)) => Err(e),
                Err(_) => Err(Error::MissingPeer(String::from(
                    "core didn't reply to request",
                ))),
            }
        } else {
            // If either communication direction failed, return an error
            // indicating we can't talk to core.

            Err(Error::MissingPeer(String::from(
                "core didn't accept request",
            )))
        }
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
    pub async fn add_rw_device<T: Into<Value> + TryFrom<Value>>(
        &self, name: Base, units: Option<&str>, max_history: Option<usize>,
    ) -> Result<(ReportReading<T>, RxDeviceSetting, Option<T>)> {
        let (tx, rx) = oneshot::channel();
        let result = self
            .req_chan
            .send(Request::AddReadWriteDevice {
                driver_name: self.driver_name.clone(),
                dev_name: Name::build(self.prefix.clone(), name),
                dev_units: units.map(String::from),
                max_history,
                rpy_chan: tx,
            })
            .await;

        if result.is_ok() {
            match rx.await {
                Ok(Ok((rr, rs, prev))) => Ok((
                    Box::new(move |a| rr(a.into())),
                    rs,
                    prev.and_then(|v| T::try_from(v).ok()),
                )),
                Ok(Err(e)) => Err(e),
                Err(_) => Err(Error::MissingPeer(String::from(
                    "core didn't reply to request",
                ))),
            }
        } else {
            Err(Error::MissingPeer(String::from(
                "core didn't accept request",
            )))
        }
    }
}

pub type DriverType = Box<dyn API>;

/// All drivers implement the `driver::API` trait.
///
/// The `API` trait defines methods that are expected to be available
/// from a driver instance. By supporting this API, the framework can
/// create driver instances and monitor them as they run.

pub trait API: Send {
    /// Creates an instance of the driver.
    ///
    /// `cfg` contains the driver parameters, as specified in the
    /// `drmem.toml` configuration file. It is a `toml::Table` type so
    /// the keys for the parameter names are strings and the
    /// associated data are `toml::Value` types. This method should
    /// validate the parameters and convert them into forms useful to
    /// the driver. By convention, if any errors are found in the
    /// configuration, this method should return `Error::BadConfig`.
    ///
    /// `drc` is the send handle to a device request channel. The
    /// driver should store this handle and use it to communicate with
    /// the framework. Its typical use is to register devices with the
    /// framework, which is usually done in this method. As other
    /// request types are added, they can be used while the driver is
    /// running.
    ///
    /// `max_history` is specified in the configuration file. It is a
    /// hint as to the maximum number of data point to save for each
    /// of the devices created by this driver. A backend can choose to
    /// interpret this in its own way. For instance, the simple
    /// backend can only ever save one data point. Redis will take
    /// this as a hint and will choose the most efficient way to prune
    /// the history. That means, if more than the limit is present,
    /// redis won't prune the history to less than the limit. However
    /// there may be more than the limit -- it just won't grow without
    /// bound.

    fn create_instance(
        cfg: DriverConfig, drc: RequestChan, max_history: Option<usize>,
    ) -> Pin<Box<dyn Future<Output = Result<DriverType>> + Send + 'static>>
    where
        Self: Sized;

    /// Runs the instance of the driver.
    ///
    /// Since drivers provide access to hardware, this method should
    /// never return unless something severe occurs. All drivers are
    /// monitored by a task and if a driver panics or returns an error
    /// from this method, it gets reported in the log and then, after
    /// a short delay, the driver is restarted.

    fn run<'a>(
        &'a mut self,
    ) -> Pin<Box<dyn Future<Output = Infallible> + Send + 'a>>;
}

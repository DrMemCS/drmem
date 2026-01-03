//! Defines types and interfaces that drivers use to interact with the
//! core of DrMem.

use crate::types::{device, Error};
use std::future::Future;
use std::{convert::Infallible, sync::Arc};
use tokio::sync::{mpsc, oneshot};
use toml::value;

use super::Result;

/// Represents the type used to specify the name of a driver.
pub type Name = Arc<str>;

/// Represents how configuration information is given to a driver.
/// Since each driver can have vastly different requirements, the
/// config structure needs to be as general as possible. A
/// `DriverConfig` type is a map with `String` keys and `toml::Value`
/// values.
pub type DriverConfig = value::Table;

pub mod classes;
mod ro_device;
mod rw_device;
mod shared_rw_device;

pub use ro_device::{ReadOnlyDevice, ReportReading};
pub use rw_device::{
    ReadWriteDevice, RxDeviceSetting, SettingReply, SettingRequest,
    TxDeviceSetting,
};
pub use shared_rw_device::SharedReadWriteDevice;

/// Defines the requests that can be sent to core. Drivers don't use
/// this type directly. They are indirectly used by `RequestChan`.
pub enum Request {
    /// Registers a read-only device with core.
    ///
    /// The reply is a pair where the first element is a channel to
    /// report updated values of the device. The second element, if
    /// not `None`, is the last saved value of the device.
    AddReadonlyDevice {
        driver_name: Name,
        dev_name: device::Name,
        dev_units: Option<String>,
        max_history: Option<usize>,
        rpy_chan: oneshot::Sender<Result<ReportReading>>,
    },

    /// Registers a writable device with core.
    ///
    /// The reply is a 3-tuple where the first element is a channel to
    /// report updated values of the device. The second element is a
    /// stream that yileds incoming settings to the device. The last
    /// element, if not `None`, is the last saved value of the device.
    AddReadWriteDevice {
        driver_name: Name,
        dev_name: device::Name,
        dev_units: Option<String>,
        max_history: Option<usize>,
        rpy_chan: oneshot::Sender<
            Result<(ReportReading, RxDeviceSetting, Option<device::Value>)>,
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
    driver_name: Name,
    prefix: device::Path,
    req_chan: mpsc::Sender<Request>,
}

impl RequestChan {
    pub fn new(
        driver_name: Name,
        prefix: &device::Path,
        req_chan: &mpsc::Sender<Request>,
    ) -> Self {
        RequestChan {
            driver_name,
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
    pub async fn add_ro_device<T: device::ReadCompat>(
        &self,
        name: device::Base,
        units: Option<&str>,
        max_history: Option<usize>,
    ) -> super::Result<ReadOnlyDevice<T>> {
        // Create a location for the reply.

        let (tx, rx) = oneshot::channel();

        // Send a request to Core to register the given name.

        let result = self
            .req_chan
            .send(Request::AddReadonlyDevice {
                driver_name: self.driver_name.clone(),
                dev_name: device::Name::build(self.prefix.clone(), name),
                dev_units: units.map(String::from),
                max_history,
                rpy_chan: tx,
            })
            .await;

        // If the request was sent successfully and we successfully
        // received a reply, process the payload.

        if result.is_ok() {
            if let Ok(v) = rx.await {
                return v.map(ReadOnlyDevice::new);
            }
        }

        Err(Error::MissingPeer(String::from(
            "can't communicate with core",
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
    pub async fn add_rw_device<T: device::ReadWriteCompat>(
        &self,
        name: device::Base,
        units: Option<&str>,
        max_history: Option<usize>,
    ) -> Result<ReadWriteDevice<T>> {
        let (tx, rx) = oneshot::channel();
        let result = self
            .req_chan
            .send(Request::AddReadWriteDevice {
                driver_name: self.driver_name.clone(),
                dev_name: device::Name::build(self.prefix.clone(), name),
                dev_units: units.map(String::from),
                max_history,
                rpy_chan: tx,
            })
            .await;

        if result.is_ok() {
            if let Ok(v) = rx.await {
                return v.map(|(rr, rs, prev)| {
                    ReadWriteDevice::new(
                        rr,
                        rs,
                        prev.and_then(|v| T::try_from(v).ok()),
                    )
                });
            }
        }

        Err(Error::MissingPeer(String::from(
            "can't communicate with core",
        )))
    }

    /// Registers a device, with the framework, that is read-write but
    /// which can also change state via external means. WiFi LED
    /// bulbs, for instance, can be controlled by DrMem but can also
    /// be adjusted by a person in the room or an app or another
    /// automation agent, like Google Home.
    ///
    /// `name` is the last section of the full device name. Typically
    /// a driver will register several devices, each representing a
    /// portion of the hardware being controlled. All devices for a
    /// given driver instance will have the same prefix; the `name`
    /// parameter is appended to it.
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
    pub async fn add_shared_rw_device<T: device::ReadWriteCompat>(
        &self,
        name: device::Base,
        units: Option<&str>,
        override_duration: Option<tokio::time::Duration>,
        max_history: Option<usize>,
    ) -> Result<SharedReadWriteDevice<T>> {
        let (tx, rx) = oneshot::channel();
        let result = self
            .req_chan
            .send(Request::AddReadWriteDevice {
                driver_name: self.driver_name.clone(),
                dev_name: device::Name::build(self.prefix.clone(), name),
                dev_units: units.map(String::from),
                max_history,
                rpy_chan: tx,
            })
            .await;

        if result.is_ok() {
            if let Ok(v) = rx.await {
                return v.map(|(rr, rs, prev)| {
                    SharedReadWriteDevice::new(
                        rr,
                        rs,
                        prev.and_then(|v| T::try_from(v).ok()),
                        override_duration,
                    )
                });
            }
        }

        Err(Error::MissingPeer(String::from(
            "can't communicate with core",
        )))
    }
}

pub trait ResettableState {
    fn reset_state(&mut self) {}
}

/// A trait which manages details about driver registration.
///
/// All drivers will implement a type, or use one of the predefined
/// types, that registers the set of needed devices.
///
/// The only function in this trait is one to register the device(s)
/// with core and return the set of handles.
pub trait Registrator: ResettableState + Sized + Send {
    fn register_devices<'a>(
        drc: &'a mut RequestChan,
        cfg: &DriverConfig,
        max_history: Option<usize>,
    ) -> impl Future<Output = Result<Self>> + Send + 'a;
}

/// All drivers implement the `driver::API` trait.
///
/// The `API` trait defines methods that are expected to be available
/// from a driver instance. By supporting this API, the framework can
/// create driver instances and monitor them as they run.
pub trait API: Send + Sync {
    type HardwareType: Registrator;

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
    /// `drc` is a communication channel with which the driver makes
    /// requests to the core. Its typical use is to register devices
    /// with the framework, which is usually done in this method. As
    /// other request types are added, they can be used while the
    /// driver is running.
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
        cfg: &DriverConfig,
    ) -> impl Future<Output = Result<Box<Self>>> + Send + '_;

    /// Runs the instance of the driver.
    ///
    /// Since drivers provide access to hardware, this method should
    /// never return unless something severe occurs and, in that case,
    /// it should use `panic!()`. All drivers are monitored by a task
    /// and if a driver panics or returns an error from this method,
    /// it gets reported in the log and then, after a short delay, the
    /// driver is restarted.
    fn run<'a>(
        &'a mut self,
        devices: &'a mut Self::HardwareType,
    ) -> impl Future<Output = Infallible> + Send + 'a;
}

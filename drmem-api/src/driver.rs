//! Defines types and interfaces that drivers use to interact with the
//! core of DrMem.

use crate::types::{device, Error};
use std::future::Future;
use std::{convert::Infallible, pin::Pin, sync::Arc};
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio_stream::{wrappers::ReceiverStream, Stream, StreamExt};
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

/// This type represents the data that is transferred in the
/// communication channel. It simplifies the next two types.
pub type SettingRequest =
    (device::Value, oneshot::Sender<Result<device::Value>>);

/// Used by client APIs to send setting requests to a driver.
pub type TxDeviceSetting = mpsc::Sender<SettingRequest>;

/// Used by a driver to receive settings from a client.
pub type RxDeviceSetting = mpsc::Receiver<SettingRequest>;

/// A closure type that defines how a driver replies to a setting
/// request. It can return `Ok()` to show what value was actually used
/// or `Err()` to indicate the setting failed.
pub type SettingReply<T> = Box<dyn FnOnce(Result<T>) + Send>;

/// The driver is given a stream that yields setting requests. If the
/// driver uses a type that can be converted to and from a
/// `device::Value`, this stream will automatically reject settings
/// that aren't of the correct type and pass on converted values.
pub type SettingStream<T> =
    Pin<Box<dyn Stream<Item = (T, SettingReply<T>)> + Send + Sync>>;

/// A function that drivers use to report updated values of a device.
pub type ReportReading<T> =
    Box<dyn Fn(T) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

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
        rpy_chan: oneshot::Sender<
            Result<(ReportReading<device::Value>, Option<device::Value>)>,
        >,
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
            Result<(
                ReportReading<device::Value>,
                RxDeviceSetting,
                Option<device::Value>,
            )>,
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
    pub async fn add_ro_device<
        T: Into<device::Value> + TryFrom<device::Value>,
    >(
        &self,
        name: device::Base,
        units: Option<&str>,
        max_history: Option<usize>,
    ) -> super::Result<(ReportReading<T>, Option<T>)> {
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
                return v.map(|(rr, prev)| {
                    (
                        Box::new(move |a: T| rr(a.into())) as ReportReading<T>,
                        prev.and_then(|v| T::try_from(v).ok()),
                    )
                });
            }
        }

        Err(Error::MissingPeer(String::from(
            "can't communicate with core",
        )))
    }

    // Creates a stream of incoming settings. Since settings are
    // provided as `device::Value` types, we try to map them to the
    // desired type. If the conversion can't be done, an error is
    // automatically sent back to the client and the message isn't
    // forwarded to the driver. Otherwise the converted value is
    // yielded.

    fn create_setting_stream<
        T: TryFrom<device::Value> + Into<device::Value>,
    >(
        rx: RxDeviceSetting,
    ) -> SettingStream<T> {
        Box::pin(ReceiverStream::new(rx).filter_map(|(v, tx_rpy)| {
            match T::try_from(v) {
                Ok(v) => {
                    let f: SettingReply<T> = Box::new(|v: Result<T>| {
                        let _ = tx_rpy.send(v.map(T::into));
                    });

                    Some((v, f))
                }
                Err(_) => {
                    let _ = tx_rpy.send(Err(Error::TypeError));

                    None
                }
            }
        }))
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
    pub async fn add_rw_device<
        T: Into<device::Value> + TryFrom<device::Value>,
    >(
        &self,
        name: device::Base,
        units: Option<&str>,
        max_history: Option<usize>,
    ) -> Result<(ReportReading<T>, SettingStream<T>, Option<T>)> {
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
                    (
                        Box::new(move |a: T| rr(a.into())) as ReportReading<T>,
                        RequestChan::create_setting_stream(rs),
                        prev.and_then(|v| T::try_from(v).ok()),
                    )
                });
            }
        }

        Err(Error::MissingPeer(String::from(
            "can't communicate with core",
        )))
    }
}

/// Defines a boxed type that supports the `driver::API` trait.

pub type DriverType<T> = Box<dyn API<DeviceSet = <T as API>::DeviceSet>>;

/// All drivers implement the `driver::API` trait.
///
/// The `API` trait defines methods that are expected to be available
/// from a driver instance. By supporting this API, the framework can
/// create driver instances and monitor them as they run.

pub trait API: Send {
    type DeviceSet: Send + Sync;

    fn register_devices(
        drc: RequestChan,
        cfg: &DriverConfig,
        max_history: Option<usize>,
    ) -> Pin<Box<dyn Future<Output = Result<Self::DeviceSet>> + Send>>;

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
    ) -> Pin<Box<dyn Future<Output = Result<Box<Self>>> + Send>>
    where
        Self: Sized;

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
        devices: Arc<Mutex<Self::DeviceSet>>,
    ) -> Pin<Box<dyn Future<Output = Infallible> + Send + 'a>>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::{mpsc, oneshot};

    #[tokio::test]
    async fn test_setting_stream() {
        // Build communication channels, including wrapping the
        // receive handle in a `SettingStream`.

        let (tx, rx) = mpsc::channel(20);
        let mut s: SettingStream<bool> = RequestChan::create_setting_stream(rx);
        let (os_tx, os_rx) = oneshot::channel();

        // Assert we can send to an active channel.

        assert_eq!(tx.send((true.into(), os_tx)).await.unwrap(), ());

        // Assert there's an item in the stream and that it's been
        // converted to a `bool` type.

        let (v, f) = s.next().await.unwrap();

        assert_eq!(v, true);

        // Send back the reply -- changing it to `false`. Verify the
        // received reply is also `false`.

        f(Ok(false));

        assert_eq!(os_rx.await.unwrap().unwrap(), false.into());

        // Now try to send the wrong type to the channel. The stream
        // should reject the bad settings and return an error. This
        // means calling `.next()` will block. To avoid our tests from
        // blocking forever, we drop the `mpsc::Send` handle so the
        // stream reports end-of-stream. We can then check to see if
        // our reply was an error.

        let (os_tx, os_rx) = oneshot::channel();

        assert_eq!(tx.send(((1.0).into(), os_tx)).await.unwrap(), ());

        std::mem::drop(tx);

        assert!(s.next().await.is_none());
        assert!(os_rx.await.unwrap().is_err());
    }
}

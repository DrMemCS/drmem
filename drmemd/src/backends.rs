use chrono::{DateTime, Utc};
use drmem_api::{client, device, driver, Result};
use futures::Future;

// Defines the trait that a back-end needs to implement to provide
// storage for -- and access to -- the state of each driver's devices.

pub trait Store {
    type Reporter: driver::Reporter;

    // Called when a read-only device is to be registered with the
    // back-end.
    //
    // When `drmemd` begins, it starts up the set of drivers specified
    // in the configuration file. As these drivers initialize, they'll
    // register the devices they need. For read-only devices, this
    // method will be called.
    //
    // - `driver` is the name of the driver. The framework will
    //   guarantee that this parameter is consistent for all devices
    //   defined by a driver.
    // - `name` is the full name of the device.
    // - `units` is an optional value which specifies the engineering
    //   units returned by the device.
    // - `max_history` is a hint as to how large an archive the user
    //   specifies should be used for this device.
    //
    // On success, this function returns a pair. The first element is
    // a closure the driver uses to report updates. The second element
    // is an optional value representing the last value of the device,
    // as saved in the back-end.

    fn register_read_only_device<'a>(
        &'a mut self,
        driver: &'a str,
        name: &'a device::Name,
        units: Option<&'a String>,
        max_history: Option<usize>,
    ) -> impl Future<Output = Result<Self::Reporter>> + Send + 'a;

    // Called when a read-write device is to be registered with the
    // back-end.
    //
    // When `drmemd` begins, it starts up the set of drivers specified
    // in the configuration file. As these drivers initialize, they'll
    // register the devices they need. For read-write devices, this
    // method will be called.
    //
    // - `driver` is the name of the driver. The framework will
    //   guarantee that this parameter is consistent for all devices
    //   defined by a driver.
    // - `name` is the full name of the device.
    // - `units` is an optional value which specifies the engineering
    //   units returned by the device.
    // - `max_history` is a hint as to how large an archive the user
    //   specifies should be used for this device.
    //
    // On success, this function returns a 3-tuple. The first element
    // is a closure the driver uses to report updates. The second
    // element is a handle with which the driver will receive setting
    // requests. The third element is an optional value representing
    // the last value of the device, as saved in the back-end.

    fn register_read_write_device<'a>(
        &'a mut self,
        driver: &'a str,
        name: &'a device::Name,
        units: Option<&'a String>,
        max_history: Option<usize>,
    ) -> impl Future<
        Output = Result<(
            Self::Reporter,
            driver::RxDeviceSetting,
            Option<device::Value>,
        )>,
    > + Send
           + 'a;

    // Called when information from a device is requested.
    //
    // On success, this method should return an array of
    // `client::DevInfoReply` data. If a `pattern` is specified, only
    // device names matching the pattern should be returned. The
    // grammar of the pattern is the one used by Redis (to be
    // consistent across back-ends.)

    fn get_device_info<'a>(
        &'a mut self,
        pattern: Option<&'a str>,
    ) -> impl Future<Output = Result<Vec<client::DevInfoReply>>> + Send + 'a;

    // Sends a request to a driver to set its device to the specified
    // value.

    fn set_device(
        &self,
        name: device::Name,
        value: device::Value,
    ) -> impl Future<Output = Result<device::Value>> + Send + '_;

    // Obtains the `mpsc::Sender<>` handle associated with the
    // specified device. This handle can be used to send settings to
    // the device. If 'own` is set to `true`, the requester will be
    // the only one that can send settings to the device. NOTE: `own`
    // is currently unsupported and should always be set to 'false'.
    // When it gets supported, requesters can decide whether they
    // should set it to true.

    fn get_setting_chan(
        &self,
        name: device::Name,
        own: bool,
    ) -> impl Future<Output = Result<driver::TxDeviceSetting>> + Send + '_;

    // Creates a stream that yields values of a device as it updates.

    fn monitor_device(
        &mut self,
        name: device::Name,
        start: Option<DateTime<Utc>>,
        end: Option<DateTime<Utc>>,
    ) -> impl Future<Output = Result<device::DataStream<device::Reading>>> + Send + '_;
}

#[cfg(feature = "simple-backend")]
pub mod simple;
#[cfg(feature = "simple-backend")]
pub use simple as store;
#[cfg(feature = "simple-backend")]
pub use store::SimpleStore as Instance;

#[cfg(feature = "redis-backend")]
pub mod redis;
#[cfg(feature = "redis-backend")]
pub use redis as store;
#[cfg(feature = "redis-backend")]
pub use store::RedisStore as Instance;

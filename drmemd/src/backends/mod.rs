use async_trait::async_trait;
use chrono::{DateTime, Utc};
use drmem_api::{client, device, driver, Result};

// Defines the trait that a back-end needs to implement to provide
// storage for -- and access to -- the state of each driver's devices.

#[async_trait]
pub trait Store {
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

    async fn register_read_only_device(
        &mut self,
        driver: &str,
        name: &device::Name,
        units: Option<&String>,
        max_history: Option<usize>,
    ) -> Result<(driver::ReportReading, Option<device::Value>)>;

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

    async fn register_read_write_device(
        &mut self,
        driver: &str,
        name: &device::Name,
        units: Option<&String>,
        max_history: Option<usize>,
    ) -> Result<(
        driver::ReportReading,
        driver::RxDeviceSetting,
        Option<device::Value>,
    )>;

    // Called when information from a device is requested.
    //
    // On success, this method should return an array of
    // `client::DevInfoReply` data. If a `pattern` is specified, only
    // device names matching the pattern should be returned. The
    // grammar of the pattern is the one used by Redis (to be
    // consistent across back-ends.)

    async fn get_device_info(
        &mut self,
        pattern: Option<&str>,
    ) -> Result<Vec<client::DevInfoReply>>;

    // Sends a request to a driver to set its device to the specified
    // value.

    async fn set_device(
        &self,
        name: device::Name,
        value: device::Value,
    ) -> Result<device::Value>;

    // Obtains the `mpsc::Sender<>` handle associated with the
    // specified device. This handle can be used to send settings to
    // the device. If 'own` is set to `true`, the requester will be
    // the only one that can send settings to the device. NOTE: `own`
    // is currently unsupported and should always be set to 'false'.
    // When it gets supported, requesters can decide whether they
    // should set it to true.

    async fn get_setting_chan(
        &self,
        name: device::Name,
        own: bool,
    ) -> Result<driver::TxDeviceSetting>;

    // Creates a stream that yields values of a device as it updates.

    async fn monitor_device(
        &mut self,
        name: device::Name,
        start: Option<DateTime<Utc>>,
        end: Option<DateTime<Utc>>,
    ) -> Result<device::DataStream<device::Reading>>;
}

#[cfg(feature = "simple-backend")]
pub mod simple;
#[cfg(feature = "simple-backend")]
pub use simple as store;

#[cfg(feature = "redis-backend")]
pub mod redis;
#[cfg(feature = "redis-backend")]
pub use redis as store;

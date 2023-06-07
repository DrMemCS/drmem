//! This crate is used by various, internal tasks of `drmemd`.
//!
//! The interfaces and types defined in this crate are useful for
//! those wishing to write a new back-end storage module or a driver
//! for the `drmemd` executable.

use async_trait::async_trait;
use chrono::*;

mod types;

// Pull types down to the `drmem-api` namespace.

pub use types::device;
pub use types::Error;

/// A specialization of `std::result::Result<>` where the error value
/// is `types::Error`.

pub type Result<T> = std::result::Result<T, Error>;

/// Defines the trait that a back-end needs to implement to provide
/// storage for -- and access to -- the state of each driver's
/// devices.

#[async_trait]
pub trait Store {
    /// Called when a read-only device is to be registered with the
    /// back-end.
    ///
    /// When `drmemd` begins, it starts up the set of drivers
    /// specified in the configuration file. As these drivers
    /// initialize, they'll register the devices they need. For
    /// read-only devices, this method will be called.
    ///
    /// - `driver` is the name of the driver. The framework will
    ///   guarantee that this parameter is consistent for all devices
    ///   defined by a driver.
    /// - `name` is the full name of the device.
    /// - `units` is an optional value which specifies the engineering
    ///   units returned by the device.
    /// - `max_history` is a hint as to how large an archive the user
    ///   specifies should be used for this device.
    ///
    /// On success, this function returns a pair. The first element is
    /// a closure the driver uses to report updates. The second
    /// element is an optional value representing the last value of
    /// the device, as saved in the back-end.

    async fn register_read_only_device(
        &mut self,
        driver: &str,
        name: &types::device::Name,
        units: &Option<String>,
        max_history: &Option<usize>,
    ) -> Result<(
        driver::ReportReading<types::device::Value>,
        Option<types::device::Value>,
    )>;

    /// Called when a read-write device is to be registered with the
    /// back-end.
    ///
    /// When `drmemd` begins, it starts up the set of drivers
    /// specified in the configuration file. As these drivers
    /// initialize, they'll register the devices they need. For
    /// read-write devices, this method will be called.
    ///
    /// - `driver` is the name of the driver. The framework will
    ///   guarantee that this parameter is consistent for all devices
    ///   defined by a driver.
    /// - `name` is the full name of the device.
    /// - `units` is an optional value which specifies the engineering
    ///   units returned by the device.
    /// - `max_history` is a hint as to how large an archive the user
    ///   specifies should be used for this device.
    ///
    /// On success, this function returns a 3-tuple. The first element
    /// is a closure the driver uses to report updates. The second
    /// element is a handle with which the driver will receive setting
    /// requests. The third element is an optional value representing
    /// the last value of the device, as saved in the back-end.

    async fn register_read_write_device(
        &mut self,
        driver: &str,
        name: &types::device::Name,
        units: &Option<String>,
        max_history: &Option<usize>,
    ) -> Result<(
        driver::ReportReading<types::device::Value>,
        driver::RxDeviceSetting,
        Option<types::device::Value>,
    )>;

    /// Called when information from a device is requested.
    ///
    /// On success, this method should return an array of
    /// `client::DevInfoReply` data. If a `pattern` is specified, only
    /// device names matching the pattern should be returned. The
    /// grammar of the pattern is the one used by Redis (to be
    /// consistent across back-ends.)

    async fn get_device_info(
        &mut self,
        pattern: &Option<String>,
    ) -> Result<Vec<client::DevInfoReply>>;

    /// Sends a request to a driver to set its device to the specified
    /// value.

    async fn set_device(
        &self,
        name: types::device::Name,
        value: types::device::Value,
    ) -> Result<types::device::Value>;

    /// Obtains the `mpsc::Sender<>` handle associated with the
    /// specified device. This handle can be used to send settings to
    /// the device. If 'own` is set to `true`, the requester will be
    /// the only one that can send settings to the device. NOTE: `own`
    /// is currently unsupported and should always be set to
    /// 'false'. When it gets supported, requesters can decide whether
    /// they should set it to true.

    async fn get_setting_chan(
        &self,
        name: types::device::Name,
        own: bool,
    ) -> Result<driver::TxDeviceSetting>;

    /// Creates a stream that yields values of a device as it updates.

    async fn monitor_device(
        &mut self,
        name: types::device::Name,
        start: Option<DateTime<Utc>>,
        end: Option<DateTime<Utc>>,
    ) -> Result<types::device::DataStream<types::device::Reading>>;
}

pub mod client;
pub mod driver;

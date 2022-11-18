use async_trait::async_trait;

pub mod types;

/// A `Result` type where the error value is a value from
/// `drmem_api::types::Error`.

pub type Result<T> = std::result::Result<T, types::Error>;

/// The `DbContext` trait defines the API that a back-end needs to
/// implement to provide storage for -- and access to -- the state of
/// each driver's devices.

#[async_trait]
pub trait Store {
    /// Used by a driver to define a read-only device. `name`
    /// specifies the final segment of the device name (the path
    /// portion of the device name is specified in the driver's
    /// configuration.) On success, the function returns a closure
    /// which can be used to report device updates.

    async fn register_read_only_device(
        &mut self, driver: &str, name: &types::device::Name,
        units: &Option<String>,
    ) -> Result<(driver::ReportReading, Option<types::device::Value>)>;

    /// Used by a driver to define a read-write device. `name`
    /// specifies the final segment of the device name (the path
    /// portion of the device name is specified in the driver's
    /// configuration.) On success, the function retrns a 3-tuple. The
    /// first element is a closure which the driver uses to report new
    /// values of the device. The second element is an
    /// `mpsc::Receiver<>` handle which the driver monitors for
    /// incoming settings. The last item is the last value reported
    /// for the device. If it's a new device or the backend doesn't
    /// have a persistent store, then `None` is provided.

    async fn register_read_write_device(
        &mut self, driver: &str, name: &types::device::Name,
        units: &Option<String>,
    ) -> Result<(
        driver::ReportReading,
        driver::RxDeviceSetting,
        Option<types::device::Value>,
    )>;

    async fn get_device_info(
        &mut self, pattern: &Option<String>,
    ) -> Result<Vec<client::DevInfoReply>>;

    async fn set_device(
        &self, name: types::device::Name, value: types::device::Value,
    ) -> Result<types::device::Value>;

    async fn monitor_device(
        &mut self, name: types::device::Name,
    ) -> Result<types::device::DataStream<types::device::Reading>>;
}

pub mod client;
pub mod driver;

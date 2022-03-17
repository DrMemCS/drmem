use async_trait::async_trait;
use drmem_types::{device::Value, Error};

pub mod device;

/// A `Result` type where the error value is a value from
/// `drmem_api::types::Error`.

pub type Result<T> = std::result::Result<T, Error>;

/// The `DbContext` trait defines the API that a back-end needs to
/// implement to provide storage for -- and access to -- the state of
/// each driver's devices.

#[async_trait]
pub trait DbContext {
    /// Used by a driver to define a readable device. `name` specifies
    /// the final segment of the device name (the prefix is determined
    /// by the driver's name.) `summary` should be a one-line
    /// description of the device. `units` is an optional units
    /// field. Some devices (like boolean or string devices) don't
    /// require engineering units.
    async fn define_device<T: Into<Value> + Send>(
        &mut self, name: &str, summary: &str, units: Option<String>,
    ) -> Result<device::Device<T>>;

    /// Allows a driver to write values, associated with devices, to
    /// the database. The `values` array indicate which devices should
    /// be updated.
    ///
    /// If multiple devices change simultaneously (e.g. a device's
    /// value is computed from other devices), a driver is strongly
    /// recommended to make a single call with all the affected
    /// devices. Each call to this function makes an atomic change to
    /// the database so if all devices are changed in a single call,
    /// clients will see a consistent change.
    async fn write_values(&mut self, values: &[(String, Value)]) -> Result<()>;
}

pub mod client {
    use tokio::sync::{mpsc, oneshot};

    enum Request {
        Ping(oneshot::Sender<()>),
        GetSummary(oneshot::Sender<&'static str>),
        GetDescription(oneshot::Sender<&'static str>),
    }

    pub struct RequestChan(mpsc::Sender<Request>);
}

pub mod driver;

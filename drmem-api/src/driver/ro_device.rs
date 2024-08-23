use crate::device;
use std::future::Future;
use std::pin::Pin;

/// A function that drivers use to report updated values of a device.
pub type ReportReading = Box<
    dyn Fn(device::Value) -> Pin<Box<dyn Future<Output = ()> + Send>>
        + Send
        + Sync,
>;

/// Represents a read-only device that uses a specified type for its
/// reading. Any type that can be converted to a `device::Value` is
/// acceptable.
pub struct ReadOnlyDevice<T: Into<device::Value> + Clone> {
    report_chan: ReportReading,
    prev_val: Option<T>,
}

impl<T> ReadOnlyDevice<T>
where
    T: Into<device::Value> + Clone,
{
    /// Returns a new `ReadOnlyDevice` type.
    pub fn new(report_chan: ReportReading, prev_val: Option<T>) -> Self {
        ReadOnlyDevice {
            report_chan,
            prev_val,
        }
    }

    /// Saves a new value, returned by the device, to the backend
    /// storage.
    pub async fn report_update(&mut self, value: T) {
        self.prev_val = Some(value.clone());
        (self.report_chan)(value.into()).await
    }

    /// Gets the last value of the device. If DrMem is built with
    /// persistent storage, this value will be initialized with the
    /// last value saved to storage.
    pub fn get_last(&self) -> Option<&T> {
        self.prev_val.as_ref()
    }
}

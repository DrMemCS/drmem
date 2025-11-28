use crate::device;
use std::future::Future;
use std::marker::PhantomData;
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
pub struct ReadOnlyDevice<T: device::ReadCompat> {
    report_chan: ReportReading,
    phantom: PhantomData<T>,
}

impl<T> ReadOnlyDevice<T>
where
    T: device::ReadCompat,
{
    /// Returns a new `ReadOnlyDevice` type.
    pub fn new(report_chan: ReportReading) -> Self {
        ReadOnlyDevice {
            report_chan,
            phantom: PhantomData,
        }
    }

    /// Saves a new value, returned by the device, to the backend
    /// storage.
    pub fn report_update(
        &mut self,
        value: T,
    ) -> impl Future<Output = ()> + use<'_, T> {
        (self.report_chan)(value.into())
    }
}

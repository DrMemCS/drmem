use crate::{device, driver::Reporter};
use std::future::Future;
use std::marker::PhantomData;

/// Represents a read-only device that uses a specified type for its
/// reading. Any type that can be converted to a `device::Value` is
/// acceptable.
pub struct ReadOnlyDevice<T: device::ReadCompat, R: Reporter> {
    reporter: R,
    phantom: PhantomData<T>,
}

impl<T, R> ReadOnlyDevice<T, R>
where
    T: device::ReadCompat,
    R: Reporter,
{
    /// Returns a new `ReadOnlyDevice` type.
    pub fn new(reporter: R) -> Self {
        ReadOnlyDevice {
            reporter,
            phantom: PhantomData,
        }
    }

    /// Saves a new value, returned by the device, to the backend
    /// storage.
    pub fn report_update(
        &mut self,
        value: T,
    ) -> impl Future<Output = ()> + use<'_, T, R> {
        self.reporter.report_value(value.into())
    }
}

use crate::{device, driver::ReportReading, Error, Result};
use std::{future::Future, pin::Pin};
use tokio::sync::{mpsc, oneshot};
use tokio_stream::{wrappers::ReceiverStream, Stream, StreamExt};

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

pub type SettingTransaction<T> = (T, SettingReply<T>);

/// The driver is given a stream that yields setting requests. If the
/// driver uses a type that can be converted to and from a
/// `device::Value`, this stream will automatically reject settings
/// that aren't of the correct type and pass on converted values.
pub type SettingStream<T> =
    Pin<Box<dyn Stream<Item = SettingTransaction<T>> + Send + Sync>>;

// Creates a stream of incoming settings. Since settings are provided
// as `device::Value` types, we try to map them to the desired
// type. If the conversion can't be done, an error is automatically
// sent back to the client and the message isn't forwarded to the
// driver. Otherwise the converted value is yielded.

pub fn create_setting_stream<T>(rx: RxDeviceSetting) -> SettingStream<T>
where
    T: device::ReadWriteCompat,
{
    Box::pin(ReceiverStream::new(rx).filter_map(
        |(v, tx_rpy)| match T::try_from(v) {
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
        },
    ))
}

pub struct ReadWriteDevice<T: device::ReadWriteCompat> {
    report_chan: ReportReading,
    set_stream: SettingStream<T>,
    prev_val: Option<T>,
}

impl<T> ReadWriteDevice<T>
where
    T: device::ReadWriteCompat,
{
    pub fn new(
        report_chan: ReportReading,
        setting_chan: RxDeviceSetting,
        prev_val: Option<T>,
    ) -> Self {
        ReadWriteDevice {
            report_chan,
            set_stream: create_setting_stream(setting_chan),
            prev_val,
        }
    }

    /// Saves a new value, returned by the device, to the backend
    /// storage.
    pub fn report_update(
        &mut self,
        value: T,
    ) -> impl Future<Output = ()> + use<'_, T> {
        self.prev_val = Some(value.clone());
        (self.report_chan)(value.into())
    }

    /// Gets the last value of the device. If DrMem is built with
    /// persistent storage, this value will be initialized with the
    /// last value saved to storage.
    pub fn get_last(&self) -> Option<&T> {
        self.prev_val.as_ref()
    }

    pub fn next_setting(
        &mut self,
    ) -> impl Future<Output = Option<SettingTransaction<T>>> + use<'_, T> {
        self.set_stream.next()
    }
}

impl<T> super::ResettableState for ReadWriteDevice<T> where
    T: device::ReadWriteCompat
{
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
        let mut s: SettingStream<bool> = create_setting_stream(rx);
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

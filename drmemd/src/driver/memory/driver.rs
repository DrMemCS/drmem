use super::{config, device};
use drmem_api::{driver, Result};
use std::convert::Infallible;

pub struct Instance;

impl Instance {
    pub const NAME: &'static str = "memory";

    pub const SUMMARY: &'static str = "An area in memory to set values.";

    pub const DESCRIPTION: &'static str = include_str!("drv_memory.md");

    /// Creates a new `Instance` instance.
    pub fn new() -> Instance {
        Instance {}
    }
}

impl driver::API for Instance {
    type Config = config::Params;
    type HardwareType = device::Set;

    async fn create_instance(_cfg: &Self::Config) -> Result<Box<Self>> {
        Ok(Box::new(Instance::new()))
    }

    async fn run(&mut self, devices: &mut Self::HardwareType) -> Infallible {
        loop {
            let (idx, val) = devices.get_next().await;

            devices.set[idx].0.report_update(val).await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::device::TypeChecker;
    use drmem_api::{
        device,
        driver::{ReadWriteDevice, TxDeviceSetting},
    };
    use tokio::sync::mpsc;

    // Builds a type that acts like a settable device.

    fn build_device() -> (
        TxDeviceSetting,
        mpsc::Receiver<device::Value>,
        (ReadWriteDevice<device::Value>, TypeChecker),
    ) {
        let (tx_sets, rx_sets) = mpsc::channel(20);
        let (tx_reports, rx_reports) = mpsc::channel(20);

        (
            tx_sets,
            rx_reports,
            (
                ReadWriteDevice::<device::Value>::new(
                    Box::new(move |v| {
                        tx_reports.try_send(v).expect("couldn't report value");
                        Box::pin(async {})
                    }),
                    rx_sets,
                    None,
                ),
                |_| true,
            ),
        )
    }

    #[test]
    fn test_custom_future() {
        use super::device::Set;
        use futures::Future;
        use noop_waker::noop_waker;
        use std::task::{Context, Poll};
        use tokio::sync::oneshot;

        // If there's no memory devices, then the future should pend
        // forever.

        {
            let mut dev = Set { set: vec![] };
            let fut = std::pin::pin!(dev.get_next());
            let waker = noop_waker();
            let mut context = Context::from_waker(&waker);

            assert_eq!(fut.poll(&mut context), Poll::Pending);
        }

        // Now add a single memory device.

        {
            let (setting, _, device) = build_device();
            let mut dev = Set { set: vec![device] };
            let waker = noop_waker();
            let mut context = Context::from_waker(&waker);

            // Before we push any values, the Future should pend.

            {
                let fut = std::pin::pin!(dev.get_next());

                assert_eq!(fut.poll(&mut context), Poll::Pending);
            }

            // Now we send a `true` value. It should echo it back and
            // a future request will pend.

            let (tx_reply, _) = oneshot::channel();

            setting
                .try_send((device::Value::Bool(true), tx_reply))
                .unwrap();

            {
                let fut = std::pin::pin!(dev.get_next());

                assert_eq!(
                    fut.poll(&mut context),
                    Poll::Ready((0, device::Value::Bool(true)))
                );
            }

            {
                let fut = std::pin::pin!(dev.get_next());

                assert_eq!(fut.poll(&mut context), Poll::Pending);
            }

            // Now send two values and make sure they're both
            // returned.

            {
                let (tx_reply, _) = oneshot::channel();

                setting
                    .try_send((device::Value::Bool(true), tx_reply))
                    .unwrap();
            }

            {
                let (tx_reply, _) = oneshot::channel();

                setting
                    .try_send((device::Value::Flt(1.0), tx_reply))
                    .unwrap();
            }

            {
                let fut = std::pin::pin!(dev.get_next());

                assert_eq!(
                    fut.poll(&mut context),
                    Poll::Ready((0, device::Value::Bool(true)))
                );
            }

            {
                let fut = std::pin::pin!(dev.get_next());

                assert_eq!(
                    fut.poll(&mut context),
                    Poll::Ready((0, device::Value::Flt(1.0)))
                );
            }

            {
                let fut = std::pin::pin!(dev.get_next());

                assert_eq!(fut.poll(&mut context), Poll::Pending);
            }

            // Send two values, but send the second after we've read
            // the first.

            {
                let (tx_reply, _) = oneshot::channel();

                setting
                    .try_send((device::Value::Bool(true), tx_reply))
                    .unwrap();
            }

            {
                let fut = std::pin::pin!(dev.get_next());

                assert_eq!(
                    fut.poll(&mut context),
                    Poll::Ready((0, device::Value::Bool(true)))
                );
            }

            {
                let (tx_reply, _) = oneshot::channel();

                setting
                    .try_send((device::Value::Flt(1.0), tx_reply))
                    .unwrap();
            }

            {
                let fut = std::pin::pin!(dev.get_next());

                assert_eq!(
                    fut.poll(&mut context),
                    Poll::Ready((0, device::Value::Flt(1.0)))
                );
            }

            {
                let fut = std::pin::pin!(dev.get_next());

                assert_eq!(fut.poll(&mut context), Poll::Pending);
            }
        }

        // This section tests when we have two memory devices. We are
        // going to assume it scales to more than two.

        {
            let (setting_a, _, device_a) = build_device();
            let (setting_b, _, device_b) = build_device();
            let mut dev = Set {
                set: vec![device_a, device_b],
            };
            let waker = noop_waker();
            let mut context = Context::from_waker(&waker);

            // Nothing inserted, should Pend.

            {
                let fut = std::pin::pin!(dev.get_next());

                assert_eq!(fut.poll(&mut context), Poll::Pending);
            }

            // Set first device. Should return it, then pend.

            {
                let (tx_reply, _) = oneshot::channel();

                setting_a
                    .try_send((device::Value::Flt(1.0), tx_reply))
                    .unwrap();
            }

            {
                let fut = std::pin::pin!(dev.get_next());

                assert_eq!(
                    fut.poll(&mut context),
                    Poll::Ready((0, device::Value::Flt(1.0)))
                );
            }

            {
                let fut = std::pin::pin!(dev.get_next());

                assert_eq!(fut.poll(&mut context), Poll::Pending);
            }

            // Set second device. Should return it, then pend.

            {
                let (tx_reply, _) = oneshot::channel();

                setting_b
                    .try_send((device::Value::Flt(2.0), tx_reply))
                    .unwrap();
            }

            {
                let fut = std::pin::pin!(dev.get_next());

                assert_eq!(
                    fut.poll(&mut context),
                    Poll::Ready((1, device::Value::Flt(2.0)))
                );
            }

            {
                let fut = std::pin::pin!(dev.get_next());

                assert_eq!(fut.poll(&mut context), Poll::Pending);
            }

            // Set each device. Should read first, first, and second,
            // second.

            {
                let (tx_reply, _) = oneshot::channel();

                setting_a
                    .try_send((device::Value::Bool(true), tx_reply))
                    .unwrap();
            }

            {
                let (tx_reply, _) = oneshot::channel();

                setting_b
                    .try_send((device::Value::Flt(1.0), tx_reply))
                    .unwrap();
            }

            {
                let fut = std::pin::pin!(dev.get_next());

                assert_eq!(
                    fut.poll(&mut context),
                    Poll::Ready((0, device::Value::Bool(true)))
                );
            }

            {
                let fut = std::pin::pin!(dev.get_next());

                assert_eq!(
                    fut.poll(&mut context),
                    Poll::Ready((1, device::Value::Flt(1.0)))
                );
            }

            {
                let fut = std::pin::pin!(dev.get_next());

                assert_eq!(fut.poll(&mut context), Poll::Pending);
            }

            // Set each device twice. Interleave the settings and make
            // sure they come out in the correct order.

            {
                let (tx_reply, _) = oneshot::channel();

                setting_b
                    .try_send((device::Value::Flt(1.0), tx_reply))
                    .unwrap();
            }

            {
                let (tx_reply, _) = oneshot::channel();

                setting_a
                    .try_send((device::Value::Bool(true), tx_reply))
                    .unwrap();
            }

            {
                let (tx_reply, _) = oneshot::channel();

                setting_b
                    .try_send((device::Value::Flt(5.0), tx_reply))
                    .unwrap();
            }

            {
                let fut = std::pin::pin!(dev.get_next());

                assert_eq!(
                    fut.poll(&mut context),
                    Poll::Ready((0, device::Value::Bool(true)))
                );
            }

            {
                let fut = std::pin::pin!(dev.get_next());

                assert_eq!(
                    fut.poll(&mut context),
                    Poll::Ready((1, device::Value::Flt(1.0)))
                );
            }

            {
                let (tx_reply, _) = oneshot::channel();

                setting_a
                    .try_send((device::Value::Bool(false), tx_reply))
                    .unwrap();
            }

            {
                let fut = std::pin::pin!(dev.get_next());

                assert_eq!(
                    fut.poll(&mut context),
                    Poll::Ready((0, device::Value::Bool(false)))
                );
            }

            {
                let fut = std::pin::pin!(dev.get_next());

                assert_eq!(
                    fut.poll(&mut context),
                    Poll::Ready((1, device::Value::Flt(5.0)))
                );
            }

            {
                let fut = std::pin::pin!(dev.get_next());

                assert_eq!(fut.poll(&mut context), Poll::Pending);
            }
        }
    }
}

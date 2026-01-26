use drmem_api::{
    device,
    driver::{self, ResettableState},
    Error, Result,
};
use std::{convert::Infallible, future::Future};

// Defines the signature if a function that validates a
// `device::Value`'s type.

type TypeChecker = fn(&device::Value) -> bool;

// Returns a function that returns `true` when passed a value of the
// same type as `val`.

fn get_validator(val: &device::Value) -> TypeChecker {
    use device::Value;

    match val {
        Value::Bool(_) => |v| matches!(v, Value::Bool(_)),
        Value::Int(_) => |v| matches!(v, Value::Int(_)),
        Value::Flt(_) => |v| matches!(v, Value::Flt(_)),
        Value::Str(_) => |v| matches!(v, Value::Str(_)),
        Value::Color(_) => |v| matches!(v, Value::Color(_)),
    }
}

// Holds the set of memory devices used by an instance of the memory
// driver. Each entry has the device handle and a cooresponding
// function which is used to make sure incoming settings are of the
// correct type.

pub struct Devices {
    set: Vec<(driver::ReadWriteDevice<device::Value>, TypeChecker)>,
}

impl Devices {
    pub fn get_next(
        &mut self,
    ) -> impl Future<Output = (usize, device::Value)> + use<'_> {
        use std::future::poll_fn;

        poll_fn(move |ctxt| {
            use std::task::Poll;

            // Loop through all the devices. Get the index, the device
            // channel, and the function to verify any incoming
            // setting on that channel.

            for (idx, (dev, is_good)) in self.set.iter_mut().enumerate() {
                // Now that we have a device to look at, we enter a
                // loop. The loop is necessary because we need to
                // leave the stream in a state primed to wake us up
                // which only happens when it returns Poll::Pending.

                loop {
                    // Get a future that gets the next value from the
                    // stream. We "pin" it, since that's a requirement
                    // of futures.

                    let mut fut = std::pin::pin!(dev.next_setting());

                    // See if there's a value to read.

                    match fut.as_mut().poll(ctxt) {
                        // Got a value. Process it.
                        Poll::Ready(Some((val, reply))) => {
                            // Is the setting value of the correct
                            // type? If so, echo it back to the client
                            // (wrapped in Ok().) Return the idx of
                            // the device and the value so it can be
                            // processed by the caller.

                            if is_good(&val) {
                                reply.ok(val.clone());
                                return Poll::Ready((idx, val));
                            } else {
                                reply.err(Error::TypeError)
                            }
                        }

                        // Stream is empty. Break out the loop to
                        // check the next device.
                        Poll::Ready(None) | Poll::Pending => break,
                    }
                }
            }

            // If we exit the for-loop, then all streams were pending
            // (which also means they've all registered their wakers
            // so we'll poll again when a setting arrives.)

            Poll::Pending
        })
    }
}

mod config {
    use drmem_api::{device, driver, Error};

    #[derive(serde::Deserialize, Debug, PartialEq)]
    pub struct Entry {
        pub name: device::Base,
        pub initial: device::Value,
    }

    #[derive(serde::Deserialize, Debug, PartialEq)]
    pub struct InstanceConfig {
        pub vars: Vec<Entry>,
    }

    impl TryFrom<driver::DriverConfig> for InstanceConfig {
        type Error = Error;

        fn try_from(
            cfg: driver::DriverConfig,
        ) -> std::result::Result<Self, Self::Error> {
            cfg.parse_into()
        }
    }
}

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

impl driver::Registrator for Devices {
    type Config = config::InstanceConfig;

    async fn register_devices(
        core: &mut driver::RequestChan,
        cfg: &Self::Config,
        max_history: Option<usize>,
    ) -> Result<Self> {
        let mut devs = vec![];

        for e in cfg.vars.iter() {
            // This device is settable. Any setting is forwarded to
            // the backend.

            let mut entry: (
                driver::ReadWriteDevice<device::Value>,
                TypeChecker,
            ) = (
                core.add_rw_device(e.name.clone(), None, max_history)
                    .await?,
                get_validator(&e.initial),
            );

            // If the user configured an initial value and there was
            // no previous value or the previous value was of a
            // different type, immediately set it with the initial
            // value.

            if entry
                .0
                .get_last()
                .map(|v| !v.is_same_type(&e.initial))
                .unwrap_or(true)
            {
                entry.0.report_update(e.initial.clone()).await
            }

            // Add the entry to the driver's set of devices.

            devs.push(entry)
        }

        Ok(Devices { set: devs })
    }
}

impl driver::API for Instance {
    type Config = config::InstanceConfig;
    type HardwareType = Devices;

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

impl ResettableState for Devices {}

#[cfg(test)]
mod tests {
    use super::{config, TypeChecker};
    use drmem_api::{
        device,
        driver::{DriverConfig, ReadWriteDevice, TxDeviceSetting},
        Error, Result,
    };
    use tokio::sync::mpsc;

    #[test]
    fn test_validators() {
        use super::get_validator;

        {
            let f = get_validator(&device::Value::Bool(true));

            assert!(f(&device::Value::Bool(false)));
            assert!(!f(&device::Value::Int(10)));
            assert!(!f(&device::Value::Flt(20.0)));
            assert!(!f(&device::Value::Str("Hello".into())));
            assert!(!f(&device::Value::Color(palette::LinSrgba::new(
                0, 0, 0, 0
            ))));
        }
        {
            let f = get_validator(&device::Value::Int(5));

            assert!(!f(&device::Value::Bool(false)));
            assert!(f(&device::Value::Int(10)));
            assert!(!f(&device::Value::Flt(20.0)));
            assert!(!f(&device::Value::Str("Hello".into())));
            assert!(!f(&device::Value::Color(palette::LinSrgba::new(
                0, 0, 0, 0
            ))));
        }
        {
            let f = get_validator(&device::Value::Flt(2.0));

            assert!(!f(&device::Value::Bool(false)));
            assert!(!f(&device::Value::Int(10)));
            assert!(f(&device::Value::Flt(20.0)));
            assert!(!f(&device::Value::Str("Hello".into())));
            assert!(!f(&device::Value::Color(palette::LinSrgba::new(
                0, 0, 0, 0
            ))));
        }
        {
            let f = get_validator(&device::Value::Str("World".into()));

            assert!(!f(&device::Value::Bool(false)));
            assert!(!f(&device::Value::Int(10)));
            assert!(!f(&device::Value::Flt(20.0)));
            assert!(f(&device::Value::Str("Hello".into())));
            assert!(!f(&device::Value::Color(palette::LinSrgba::new(
                0, 0, 0, 0
            ))));
        }
        {
            let f = get_validator(&device::Value::Color(
                palette::LinSrgba::new(100, 100, 100, 100),
            ));

            assert!(!f(&device::Value::Bool(false)));
            assert!(!f(&device::Value::Int(10)));
            assert!(!f(&device::Value::Flt(20.0)));
            assert!(!f(&device::Value::Str("Hello".into())));
            assert!(f(&device::Value::Color(palette::LinSrgba::new(
                0, 0, 0, 0
            ))));
        }
    }

    fn mk_cfg(text: &str) -> Result<config::InstanceConfig> {
        Into::<DriverConfig>::into(
            toml::from_str::<toml::value::Table>(text)
                .map_err(|e| Error::ConfigError(format!("{}", e)))?,
        )
        .parse_into()
    }

    #[test]
    fn test_configuration() {
        use super::device;
        use toml::{Table, Value};

        // Test for an empty Map or a Map that doesn't have the "vars"
        // key or a map with "vars" whose value isn't a map or is a
        // map but is empty or has a value, but it's not an array. All
        // of these are errors.

        {
            assert!(mk_cfg("vars = [{initial = true}]").is_err());
            assert!(mk_cfg("vars = [{name = \"var\"}]").is_err());
            assert!(mk_cfg("vars = [{junk = \"var\"}]").is_err());
            assert!(mk_cfg("vars = [{name = \"var\", initial = true}]").is_ok());

            assert_eq!(
                mk_cfg(
                    "
vars = [{name = \"v1\", initial = true},
        {name = \"v2\", initial = 100}]"
                ),
                Ok(config::InstanceConfig {
                    vars: vec![
                        config::Entry {
                            name: "v1".try_into().unwrap(),
                            initial: device::Value::Bool(true)
                        },
                        config::Entry {
                            name: "v2".try_into().unwrap(),
                            initial: device::Value::Int(100)
                        }
                    ]
                })
            );
        }

        // Now make sure the config code creates a single memory
        // device correctly. We'll deal with sets later.

        {
            let mut test_set: Vec<(&'static str, Value, device::Value)> = vec![
                ("flag", Value::Boolean(true), device::Value::Bool(true)),
                ("int-val", Value::Integer(100), device::Value::Int(100)),
                ("flt-val", Value::Float(50.0), device::Value::Flt(50.0)),
                (
                    "str-val",
                    Value::String("Hello".into()),
                    device::Value::Str("Hello".into()),
                ),
                (
                    "clr-val",
                    Value::String("#ffffff".into()),
                    device::Value::Color(palette::LinSrgba::new(
                        255, 255, 255, 255,
                    )),
                ),
            ];

            for entry in test_set.drain(..) {
                let mut tbl = Table::new();
                let _ = tbl.insert("name".into(), entry.0.try_into().unwrap());
                let _ = tbl.insert("initial".into(), entry.1.clone());
                let mut map = Table::new();

                map.insert(
                    "vars".into(),
                    Value::Array(vec![Value::Table(tbl)]),
                );

                let result: config::InstanceConfig =
                    Into::<DriverConfig>::into(map).parse_into().unwrap();

                assert!(result.vars.len() == 1);
                assert_eq!(result.vars[0].name.to_string(), entry.0);
                assert_eq!(result.vars[0].initial, entry.2);
            }
        }
    }

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
        use super::Devices;
        use futures::Future;
        use noop_waker::noop_waker;
        use std::task::{Context, Poll};
        use tokio::sync::oneshot;

        // If there's no memory devices, then the future should pend
        // forever.

        {
            let mut dev = Devices { set: vec![] };
            let fut = std::pin::pin!(dev.get_next());
            let waker = noop_waker();
            let mut context = Context::from_waker(&waker);

            assert_eq!(fut.poll(&mut context), Poll::Pending);
        }

        // Now add a single memory device.

        {
            let (setting, _, device) = build_device();
            let mut dev = Devices { set: vec![device] };
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
            let mut dev = Devices {
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

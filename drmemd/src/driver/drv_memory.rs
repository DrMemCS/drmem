use drmem_api::{
    device,
    driver::{self, DriverConfig},
    Error, Result,
};
use std::{convert::Infallible, future::Future, pin::Pin, sync::Arc};
use tokio::sync::Mutex;

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
                                reply(Ok(val.clone()));
                                return Poll::Ready((idx, val));
                            } else {
                                reply(Err(Error::TypeError))
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

pub struct Instance;

impl Instance {
    pub const NAME: &'static str = "memory";

    pub const SUMMARY: &'static str = "An area in memory to set values.";

    pub const DESCRIPTION: &'static str = include_str!("drv_memory.md");

    /// Creates a new `Instance` instance.
    pub fn new() -> Instance {
        Instance {}
    }

    fn read_name(m: &toml::Table) -> Result<device::Base> {
        match m.get("name") {
            Some(toml::value::Value::String(name)) => {
                if let v @ Ok(_) = name.parse::<device::Base>() {
                    v
                } else {
                    Err(Error::ConfigError(format!(
                        "'{name}' isn't a proper, base name for a device"
                    )))
                }
            }
            Some(_) => Err(Error::ConfigError(String::from(
                "'name' config parameter should be a string",
            ))),
            None => Err(Error::ConfigError(String::from(
                "missing `name` parameter in `vars` entry",
            ))),
        }
    }

    fn read_initial(m: &toml::Table) -> Result<device::Value> {
        if let Some(val) = m.get("initial") {
            device::Value::try_from(val)
        } else {
            Err(Error::ConfigError(String::from(
                "missing `initial` parameter in `vars` entry",
            )))
        }
    }

    fn read_entries(
        v: &toml::value::Value,
    ) -> Result<(device::Base, device::Value)> {
        if let toml::Value::Table(m) = v {
            Ok((Self::read_name(m)?, Self::read_initial(m)?))
        } else {
            Err(Error::ConfigError(String::from(
                "`vars` contains an entry that isn't a map",
            )))
        }
    }

    // Gets the variables associated with the device from the configuration.

    fn get_cfg_vars(
        cfg: &DriverConfig,
    ) -> Result<Vec<(device::Base, device::Value)>> {
        use toml::value::Value;

        match cfg.get("vars") {
            Some(Value::Array(vars)) if !vars.is_empty() => {
                vars.iter().map(Self::read_entries).collect()
            }
            Some(_) => Err(Error::ConfigError(String::from(
                "'vars' config parameter should be an array of maps",
            ))),
            None => Err(Error::ConfigError(String::from(
                "missing 'vars' parameter in config",
            ))),
        }
    }
}

impl driver::API for Instance {
    type DeviceSet = Devices;

    fn register_devices(
        core: driver::RequestChan,
        cfg: &DriverConfig,
        max_history: Option<usize>,
    ) -> Pin<Box<dyn Future<Output = Result<Self::DeviceSet>> + Send>> {
        let vars = Self::get_cfg_vars(cfg);

        Box::pin(async move {
            let mut devs = vec![];

            for (name, init_val) in vars?.drain(..) {
                // This device is settable. Any setting is forwarded
                // to the backend.

                let mut entry: (
                    driver::ReadWriteDevice<device::Value>,
                    TypeChecker,
                ) = (
                    core.add_rw_device(name, None, max_history).await?,
                    get_validator(&init_val),
                );

                // If the user configured an initial value and there
                // was no previous value or the previous value was of
                // a different type, immediately set it with the
                // initial value.

                if entry
                    .0
                    .get_last()
                    .map(|v| !v.is_same_type(&init_val))
                    .unwrap_or(true)
                {
                    entry.0.report_update(init_val).await
                }

                // Add the entry to the driver's set of devices.

                devs.push(entry)
            }

            Ok(Devices { set: devs })
        })
    }

    fn create_instance(
        _cfg: &DriverConfig,
    ) -> Pin<Box<dyn Future<Output = Result<Box<Self>>> + Send>> {
        Box::pin(async move { Ok(Box::new(Instance::new())) })
    }

    fn run<'a>(
        &'a mut self,
        devices: Arc<Mutex<Self::DeviceSet>>,
    ) -> Pin<Box<dyn Future<Output = Infallible> + Send + 'a>> {
        Box::pin(async move {
            let mut devices = devices.lock().await;

            loop {
                let (idx, val) = devices.get_next().await;

                devices.set[idx].0.report_update(val).await
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::TypeChecker;
    use drmem_api::{
        device::Value,
        driver::{ReadWriteDevice, TxDeviceSetting},
    };
    use tokio::sync::mpsc;

    #[test]
    fn test_validators() {
        use super::get_validator;

        {
            let f = get_validator(&Value::Bool(true));

            assert!(f(&Value::Bool(false)));
            assert!(!f(&Value::Int(10)));
            assert!(!f(&Value::Flt(20.0)));
            assert!(!f(&Value::Str("Hello".into())));
            assert!(!f(&Value::Color(palette::LinSrgba::new(0, 0, 0, 0))));
        }
        {
            let f = get_validator(&Value::Int(5));

            assert!(!f(&Value::Bool(false)));
            assert!(f(&Value::Int(10)));
            assert!(!f(&Value::Flt(20.0)));
            assert!(!f(&Value::Str("Hello".into())));
            assert!(!f(&Value::Color(palette::LinSrgba::new(0, 0, 0, 0))));
        }
        {
            let f = get_validator(&Value::Flt(2.0));

            assert!(!f(&Value::Bool(false)));
            assert!(!f(&Value::Int(10)));
            assert!(f(&Value::Flt(20.0)));
            assert!(!f(&Value::Str("Hello".into())));
            assert!(!f(&Value::Color(palette::LinSrgba::new(0, 0, 0, 0))));
        }
        {
            let f = get_validator(&Value::Str("World".into()));

            assert!(!f(&Value::Bool(false)));
            assert!(!f(&Value::Int(10)));
            assert!(!f(&Value::Flt(20.0)));
            assert!(f(&Value::Str("Hello".into())));
            assert!(!f(&Value::Color(palette::LinSrgba::new(0, 0, 0, 0))));
        }
        {
            let f = get_validator(&Value::Color(palette::LinSrgba::new(
                100, 100, 100, 100,
            )));

            assert!(!f(&Value::Bool(false)));
            assert!(!f(&Value::Int(10)));
            assert!(!f(&Value::Flt(20.0)));
            assert!(!f(&Value::Str("Hello".into())));
            assert!(f(&Value::Color(palette::LinSrgba::new(0, 0, 0, 0))));
        }
    }

    #[test]
    fn test_configuration() {
        use super::device;
        use super::Instance;
        use toml::{map::Map, Table, Value};

        // Test for an empty Map or a Map that doesn't have the "vars"
        // key or a map with "vars" whose value isn't a map or is a
        // map but is empty or has a value, but it's not an array. All
        // of these are errors.

        {
            let mut map = Map::new();

            assert!(Instance::get_cfg_vars(&map).is_err());

            let _ = map.insert("junk".into(), Value::Boolean(true));

            assert!(Instance::get_cfg_vars(&map).is_err());

            let _ = map.insert("vars".into(), Value::Boolean(true));

            assert!(Instance::get_cfg_vars(&map).is_err());

            let _ = map.insert("vars".into(), Value::Table(Table::new()));

            assert!(Instance::get_cfg_vars(&map).is_err());

            let _ = map.insert("vars".into(), Value::Array(vec![]));

            assert!(Instance::get_cfg_vars(&map).is_err());
        }

        // Now make sure the config code creates a single memory
        // device correctly. We'll deal with sets later.

        {
            let mut map = Map::new();

            let test_set: &[(&'static str, Value, device::Value)] = &[
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

            for entry in &test_set[..] {
                let mut tbl = Table::new();
                let _ = tbl.insert("name".into(), entry.0.into());
                let _ = tbl.insert("initial".into(), entry.1.clone());
                let _ = map.insert(
                    "vars".into(),
                    Value::Array(vec![Value::Table(tbl)]),
                );

                let result = Instance::get_cfg_vars(&map).unwrap();

                assert!(result.len() == 1);
                assert_eq!(result[0].0.to_string(), entry.0);
                assert_eq!(result[0].1, entry.2);
            }
        }
    }

    // Builds a type that acts like a settable device.

    fn build_device() -> (
        TxDeviceSetting,
        mpsc::Receiver<Value>,
        (ReadWriteDevice<Value>, TypeChecker),
    ) {
        let (tx_sets, rx_sets) = mpsc::channel(20);
        let (tx_reports, rx_reports) = mpsc::channel(20);

        (
            tx_sets,
            rx_reports,
            (
                ReadWriteDevice::<Value>::new(
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

            setting.try_send((Value::Bool(true), tx_reply)).unwrap();

            {
                let fut = std::pin::pin!(dev.get_next());

                assert_eq!(
                    fut.poll(&mut context),
                    Poll::Ready((0, Value::Bool(true)))
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

                setting.try_send((Value::Bool(true), tx_reply)).unwrap();
            }

            {
                let (tx_reply, _) = oneshot::channel();

                setting.try_send((Value::Flt(1.0), tx_reply)).unwrap();
            }

            {
                let fut = std::pin::pin!(dev.get_next());

                assert_eq!(
                    fut.poll(&mut context),
                    Poll::Ready((0, Value::Bool(true)))
                );
            }

            {
                let fut = std::pin::pin!(dev.get_next());

                assert_eq!(
                    fut.poll(&mut context),
                    Poll::Ready((0, Value::Flt(1.0)))
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

                setting.try_send((Value::Bool(true), tx_reply)).unwrap();
            }

            {
                let fut = std::pin::pin!(dev.get_next());

                assert_eq!(
                    fut.poll(&mut context),
                    Poll::Ready((0, Value::Bool(true)))
                );
            }

            {
                let (tx_reply, _) = oneshot::channel();

                setting.try_send((Value::Flt(1.0), tx_reply)).unwrap();
            }

            {
                let fut = std::pin::pin!(dev.get_next());

                assert_eq!(
                    fut.poll(&mut context),
                    Poll::Ready((0, Value::Flt(1.0)))
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

                setting_a.try_send((Value::Flt(1.0), tx_reply)).unwrap();
            }

            {
                let fut = std::pin::pin!(dev.get_next());

                assert_eq!(
                    fut.poll(&mut context),
                    Poll::Ready((0, Value::Flt(1.0)))
                );
            }

            {
                let fut = std::pin::pin!(dev.get_next());

                assert_eq!(fut.poll(&mut context), Poll::Pending);
            }

            // Set second device. Should return it, then pend.

            {
                let (tx_reply, _) = oneshot::channel();

                setting_b.try_send((Value::Flt(2.0), tx_reply)).unwrap();
            }

            {
                let fut = std::pin::pin!(dev.get_next());

                assert_eq!(
                    fut.poll(&mut context),
                    Poll::Ready((1, Value::Flt(2.0)))
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

                setting_a.try_send((Value::Bool(true), tx_reply)).unwrap();
            }

            {
                let (tx_reply, _) = oneshot::channel();

                setting_b.try_send((Value::Flt(1.0), tx_reply)).unwrap();
            }

            {
                let fut = std::pin::pin!(dev.get_next());

                assert_eq!(
                    fut.poll(&mut context),
                    Poll::Ready((0, Value::Bool(true)))
                );
            }

            {
                let fut = std::pin::pin!(dev.get_next());

                assert_eq!(
                    fut.poll(&mut context),
                    Poll::Ready((1, Value::Flt(1.0)))
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

                setting_b.try_send((Value::Flt(1.0), tx_reply)).unwrap();
            }

            {
                let (tx_reply, _) = oneshot::channel();

                setting_a.try_send((Value::Bool(true), tx_reply)).unwrap();
            }

            {
                let (tx_reply, _) = oneshot::channel();

                setting_b.try_send((Value::Flt(5.0), tx_reply)).unwrap();
            }

            {
                let fut = std::pin::pin!(dev.get_next());

                assert_eq!(
                    fut.poll(&mut context),
                    Poll::Ready((0, Value::Bool(true)))
                );
            }

            {
                let fut = std::pin::pin!(dev.get_next());

                assert_eq!(
                    fut.poll(&mut context),
                    Poll::Ready((1, Value::Flt(1.0)))
                );
            }

            {
                let (tx_reply, _) = oneshot::channel();

                setting_a.try_send((Value::Bool(false), tx_reply)).unwrap();
            }

            {
                let fut = std::pin::pin!(dev.get_next());

                assert_eq!(
                    fut.poll(&mut context),
                    Poll::Ready((0, Value::Bool(false)))
                );
            }

            {
                let fut = std::pin::pin!(dev.get_next());

                assert_eq!(
                    fut.poll(&mut context),
                    Poll::Ready((1, Value::Flt(5.0)))
                );
            }

            {
                let fut = std::pin::pin!(dev.get_next());

                assert_eq!(fut.poll(&mut context), Poll::Pending);
            }
        }
    }
}

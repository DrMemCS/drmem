use drmem_api::{
    device,
    driver::{self, DriverConfig},
    Error, Result,
};
use std::{
    convert::Infallible, future::Future, ops::DerefMut, pin::Pin, sync::Arc,
};
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

impl Future for Devices {
    type Output = (usize, device::Value);

    // Making `Devices` a `Future` allows efficient monitoring of the
    // set of memory device handles. Ideally this future would return
    // the reference to the ReadWriteDevice channel but returning a
    // reference to a value in a future that's wrapping in a mutex is
    // beyond my abilities.
    //
    // Since this isn't a public `Future`, I can return the index to
    // the channel and have the outer algorithm use it properly.

    fn poll(
        mut self: Pin<&mut Self>,
        ctxt: &mut std::task::Context<'_>,
    ) -> std::task::Poll<<Self as futures::Future>::Output> {
        use std::task::Poll;

        // Loop through all the devices. Get the index, the device
        // channel, and the function to verify any incoming setting on
        // that channel.

        for (idx, (dev, is_good)) in self.set.iter_mut().enumerate() {
            // Now that we have a device to look at, we enter a
            // loop. The loop is necessary because we need to leave
            // the stream in a state primed to wake us up which only
            // happens when it returns Poll::Pending.

            loop {
                // Get a future that gets the next value from the
                // stream. We "pin" it, since that's a requirement of
                // futures.

                let mut fut = std::pin::pin!(dev.next_setting());

                // See if there's a value to read.

                match fut.as_mut().poll(ctxt) {
                    // Got a value. Process it.
                    Poll::Ready(Some((val, reply))) => {
                        // Is the setting value of the correct type?
                        // If so, echo it back to the client (wrapped
                        // in Ok().) Return the idx of the device and
                        // the value so it can be processed by the
                        // caller.

                        if is_good(&val) {
                            reply(Ok(val.clone()));
                            return Poll::Ready((idx, val));
                        } else {
                            reply(Err(Error::TypeError))
                        }
                    }

                    // Stream is empty. Break out the loop to check
                    // the next device.
                    Poll::Ready(None) | Poll::Pending => break,
                }
            }
        }

        // If we exit the for-loop, then all streams were pending
        // (which also means they've all registered their wakers so
        // we'll poll again when a setting arrives.)

        Poll::Pending
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
                    Err(Error::ConfigError(String::from(
                        "'name' isn't a proper, base name for a device",
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
        let fut = async move {
            // Build and return the future.

            Ok(Box::new(Instance::new()))
        };

        Box::pin(fut)
    }

    fn run<'a>(
        &'a mut self,
        devices: Arc<Mutex<Self::DeviceSet>>,
    ) -> Pin<Box<dyn Future<Output = Infallible> + Send + 'a>> {
        Box::pin(async move {
            let mut devices = devices.lock().await;

            loop {
                let (idx, val) = devices.deref_mut().await;

                devices.set[idx].0.report_update(val).await
            }
        })
    }
}

#[cfg(test)]
mod tests {}

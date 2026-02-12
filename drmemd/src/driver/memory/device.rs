use super::config;
use drmem_api::{
    device::{Path, Value},
    driver::{self, Reporter, ResettableState},
    Error, Result,
};
use std::future::Future;

// Defines the signature if a function that validates a
// `device::Value`'s type.

pub type TypeChecker = fn(&Value) -> bool;

// Returns a function that returns `true` when passed a value of the
// same type as `val`.

fn get_validator(val: &Value) -> TypeChecker {
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

pub struct Set<R: Reporter> {
    pub set: Vec<(driver::ReadWriteDevice<Value, R>, TypeChecker)>,
}

impl<R: Reporter> Set<R> {
    pub fn get_next(
        &mut self,
    ) -> impl Future<Output = (usize, Value)> + use<'_, R> {
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

impl<R: Reporter> driver::Registrator<R> for Set<R> {
    type Config = config::Params;

    async fn register_devices(
        core: &mut driver::RequestChan<R>,
        subpath: Option<&Path>,
        cfg: &Self::Config,
        max_history: Option<usize>,
    ) -> Result<Self> {
        let mut devs = vec![];

        for e in cfg.vars.iter() {
            // This device is settable. Any setting is forwarded to
            // the backend.

            let mut entry: (driver::ReadWriteDevice<Value, R>, TypeChecker) = (
                core.add_rw_device(e.name.clone(), subpath, None, max_history)
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

        Ok(Set { set: devs })
    }
}

impl<R: Reporter> ResettableState for Set<R> {}

#[cfg(test)]
mod test {
    use drmem_api::device::Value;

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
}

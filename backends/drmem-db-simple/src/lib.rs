//! Provides a simple, storage back-end for the DrMem control system.
//!
//! This is the simplest data-store available. It only saves the last
//! value for each device. It also doesn't provide persistent storage
//! for device meta-information so, after a restart, that information
//! is reset to its default state.
//!
//! This back-end is useful for installations that don't require
//! historical information but, instead, are doing real-time control
//! with current values.

use async_trait::async_trait;
use drmem_api::{
    driver::{ReportReading, RxDeviceSetting, TxDeviceSetting},
    types::{
        device::{Name, Value},
        Error,
    },
    Result, Store,
};
use drmem_config::backend;
use futures_util::future;
use std::collections::{hash_map, HashMap};
use std::sync::{Arc, Mutex};
use tokio::sync::{broadcast, mpsc};
use tracing::error;

const CHAN_SIZE: usize = 20;

struct DeviceInfo {
    owner: String,
    units: Option<String>,
    tx_reading: broadcast::Sender<Value>,
    tx_setting: Option<TxDeviceSetting>,
    last_reading: Arc<Mutex<Option<Value>>>,
}

impl DeviceInfo {
    pub fn create(
        owner: String, units: Option<String>,
        tx_setting: Option<TxDeviceSetting>,
    ) -> DeviceInfo {
        let (tx, _) = broadcast::channel(CHAN_SIZE);

        // Build the entry and insert it in the table.

        DeviceInfo {
            owner,
            units,
            tx_reading: tx,
            tx_setting,
            last_reading: Arc::new(Mutex::new(None)),
        }
    }
}

struct SimpleStore(HashMap<Name, DeviceInfo>);

pub async fn open(_cfg: &backend::Config) -> Result<impl Store> {
    Ok(SimpleStore(HashMap::new()))
}

// Builds the `ReportReading` function. Drivers will call specialized
// instances of this function to record the latest value of a device.

fn mk_report_func(di: &DeviceInfo, name: &Name) -> ReportReading {
    let tx = di.tx_reading.clone();
    let reading = di.last_reading.clone();
    let name = name.to_string();

    Box::new(move |v| {
        let _ = tx.send(v.clone());

        // If a lock is obtained, update the current value. The only
        // way a lock can fail is if it's "poisoned", which means
        // another thread panicked while holding the lock. This module
        // holds the only code that uses the mutex and all accesses
        // are short and infallible, so the error message shouldn't
        // ever get displayed.

        if let Ok(mut data) = reading.lock() {
            *data = Some(v)
        } else {
            error!("couldn't set current value of {}", &name)
        }
        Box::pin(future::ok(()))
    })
}

#[async_trait]
impl Store for SimpleStore {
    /// Handle read-only devices registration. This function creates
    /// an association between the device name and its associated
    /// resources. Since the driver is registering a read-only device,
    /// this function doesn't allocate a channel to provide settings.

    async fn register_read_only_device(
        &mut self, driver: &str, name: &Name, units: &Option<String>,
    ) -> Result<(ReportReading, Option<Value>)> {
        // Check to see if the device name already exists.

        match self.0.entry((*name).clone()) {
            // The device didn't exist. Create it and associate it
            // with the driver.
            hash_map::Entry::Vacant(e) => {
                // Build the entry and insert it in the table.

                let di = e.insert(DeviceInfo::create(
                    String::from(driver),
                    units.clone(),
                    None,
                ));

                // Create and return the closure that the driver will
                // use to report updates.

                Ok((mk_report_func(di, name), None))
            }

            // The device already exists. If it was created from a
            // previous instance of the driver, allow the registration
            // to succeed.
            hash_map::Entry::Occupied(e) => {
                let dev_info = e.get();

                if dev_info.owner == driver {
                    let func = mk_report_func(dev_info, name);
                    let guard = dev_info.last_reading.lock();

                    Ok((
                        func,
                        if let Ok(data) = guard {
                            data.clone()
                        } else {
                            None
                        },
                    ))
                } else {
                    Err(Error::InUse)
                }
            }
        }
    }

    /// Handle read-write devices registration. This function creates
    /// an association between the device name and its associated
    /// resources.

    async fn register_read_write_device(
        &mut self, driver: &str, name: &Name, units: &Option<String>,
    ) -> Result<(ReportReading, RxDeviceSetting, Option<Value>)> {
        // Check to see if the device name already exists.

        match self.0.entry((*name).clone()) {
            // The device didn't exist. Create it and associate it
            // with the driver.
            hash_map::Entry::Vacant(e) => {
                // Create a channel with which to send settings.

                let (tx_sets, rx_sets) = mpsc::channel(CHAN_SIZE);

                // Build the entry and insert it in the table.

                let di = e.insert(DeviceInfo::create(
                    String::from(driver),
                    units.clone(),
                    Some(tx_sets),
                ));

                // Create and return the closure that the driver will
                // use to report updates.

                Ok((mk_report_func(di, name), rx_sets, None))
            }

            // The device already exists. If it was created from a
            // previous instance of the driver, allow the registration
            // to succeed.
            hash_map::Entry::Occupied(mut e) => {
                let dev_info = e.get_mut();

                if dev_info.owner == driver {
                    // Create a channel with which to send settings.

                    let (tx_sets, rx_sets) = mpsc::channel(CHAN_SIZE);

                    dev_info.tx_setting = Some(tx_sets);

                    let func = mk_report_func(dev_info, name);
                    let guard = dev_info.last_reading.lock();

                    Ok((
                        func,
                        rx_sets,
                        if let Ok(data) = guard {
                            data.clone()
                        } else {
                            None
                        },
                    ))
                } else {
                    Err(Error::InUse)
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{mk_report_func, DeviceInfo, SimpleStore};
    use drmem_api::{
        types::device::{Name, Value},
        Store,
    };
    use std::collections::HashMap;
    use tokio::sync::mpsc::error::TryRecvError;

    #[tokio::test]
    async fn test_ro_registration() {
        let mut db = SimpleStore(HashMap::new());
        let name = "misc:junk".parse::<Name>().unwrap();

        // Register a device named "junk" and associate it with the
        // driver named "test". We don't define units for this device.

        if let Ok((_, None)) =
            db.register_read_only_device("test", &name, &None).await
        {
            // Make sure the device was defined and the setting
            // channel is `None`.

            assert!(db.0.get(&name).unwrap().tx_setting.is_none());

            // Create a receiving handle for device updates.

            let mut rx = db.0.get(&name).unwrap().tx_reading.subscribe();

            // Assert that re-registering this device with a different
            // driver name results in an error.

            assert!(db
                .register_read_only_device("test2", &name, &None)
                .await
                .is_err());

            // Assert that re-registering this device with the same
            // driver name is successful.

            if let Ok((f, None)) =
                db.register_read_only_device("test", &name, &None).await
            {
                // Also, verify that the device update channel wasn't
                // disrupted by sending a value and receiving it from
                // the receive handle we opened before re-registering.

                assert!(f(Value::Int(2)).await.is_ok());
                assert_eq!(rx.try_recv(), Ok(Value::Int(2)));
            } else {
                panic!("error registering read-only device from same driver")
            }
        } else {
            panic!("error registering read-only device on empty database")
        }
    }

    #[tokio::test]
    async fn test_rw_registration() {
        let mut db = SimpleStore(HashMap::new());
        let name = "misc:junk".parse::<Name>().unwrap();

        // Register a device named "junk" and associate it with the
        // driver named "test". We don't define units for this device.

        if let Ok((_, mut set_chan, None)) =
            db.register_read_write_device("test", &name, &None).await
        {
            // Make sure the device was defined and a setting channel
            // has been created.

            assert!(db.0.get(&name).unwrap().tx_setting.is_some());

            // Make sure the setting channel is valid.

            {
                let tx_set =
                    db.0.get(&name).unwrap().tx_setting.clone().unwrap();

                assert_eq!(tx_set.is_closed(), false);
                assert!(tx_set.send(Value::Int(2)).await.is_ok());
                assert_eq!(set_chan.try_recv(), Ok(Value::Int(2)));
            }

            // Create a receiving handle for device updates.

            let mut rx = db.0.get(&name).unwrap().tx_reading.subscribe();

            // Assert that re-registering this device with a different
            // driver name results in an error. Also verify that it
            // didn't affect the setting channel.

            assert!(db
                .register_read_only_device("test2", &name, &None)
                .await
                .is_err());
            assert_eq!(Err(TryRecvError::Empty), set_chan.try_recv());

            // Assert that re-registering this device with the same
            // driver name is successful.

            if let Ok((f, _, None)) =
                db.register_read_write_device("test", &name, &None).await
            {
                assert_eq!(
                    Err(TryRecvError::Disconnected),
                    set_chan.try_recv()
                );

                // Also, verify that the device update channel wasn't
                // disrupted by sending a value and receiving it from
                // the receive handle we opened before re-registering.

                assert!(f(Value::Int(2)).await.is_ok());
                assert_eq!(rx.try_recv(), Ok(Value::Int(2)));
            } else {
                panic!("error registering read-only device from same driver")
            }
        } else {
            panic!("error registering read-only device on empty database")
        }
    }

    #[tokio::test]
    async fn test_closure() {
        let di = DeviceInfo::create(String::from("test"), None, None);
        let name = "misc:junk".parse::<Name>().unwrap();
        let f = mk_report_func(&di, &name);

        assert_eq!(*di.last_reading.lock().unwrap(), None);
        assert!(f(Value::Int(1)).await.is_ok());
        assert_eq!(*di.last_reading.lock().unwrap(), Some(Value::Int(1)));

        {
            let mut rx = di.tx_reading.subscribe();

            assert!(f(Value::Int(2)).await.is_ok());
            assert_eq!(rx.try_recv(), Ok(Value::Int(2)));
            assert_eq!(*di.last_reading.lock().unwrap(), Some(Value::Int(2)));
        }

        assert!(f(Value::Int(3)).await.is_ok());
        assert_eq!(*di.last_reading.lock().unwrap(), Some(Value::Int(3)));

        {
            let mut rx1 = di.tx_reading.subscribe();
            let mut rx2 = di.tx_reading.subscribe();

            assert!(f(Value::Int(4)).await.is_ok());
            assert_eq!(rx1.try_recv(), Ok(Value::Int(4)));
            assert_eq!(rx2.try_recv(), Ok(Value::Int(4)));
            assert_eq!(*di.last_reading.lock().unwrap(), Some(Value::Int(4)));
        }
    }
}

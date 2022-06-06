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
    types::{device::Value, Error},
    Result, Store,
};
use drmem_config::backend;
use futures_util::future;
use std::collections::{hash_map, HashMap};
use tokio::sync::{broadcast, mpsc};

const CHAN_SIZE: usize = 20;

struct DeviceInfo {
    owner: String,
    _units: Option<String>,
    tx_reading: broadcast::Sender<Value>,
    tx_setting: Option<TxDeviceSetting>,
}

struct SimpleStore(HashMap<String, DeviceInfo>);

pub async fn open(_cfg: &backend::Config) -> Result<impl Store> {
    Ok(SimpleStore(HashMap::new()))
}

fn mk_report_func(tx: broadcast::Sender<Value>, _name: &str) -> ReportReading {
    Box::new(move |v| {
        let _ = tx.send(v);

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
        &mut self, driver: &str, name: &str, units: &Option<String>,
    ) -> Result<(ReportReading, Option<Value>)> {
        // Check to see if the device name already exists.

        match self.0.entry(String::from(name)) {
            // The device didn't exist. Create it and associate it
            // with the driver.
            hash_map::Entry::Vacant(e) => {
                // Create a broadcast channel. Slow clients will get
                // an error if they miss an update.

                let (tx, _) = broadcast::channel(CHAN_SIZE);

                // Build the entry and insert it in the table.

                let _ = e.insert(DeviceInfo {
                    owner: String::from(driver),
                    _units: units.clone(),
                    tx_reading: tx.clone(),
                    tx_setting: None,
                });

                // Create and return the closure that the driver will
                // use to report updates.

                Ok((mk_report_func(tx, name), None))
            }

            // The device already exists. If it was created from a
            // previous instance of the driver, allow the registration
            // to succeed.
            hash_map::Entry::Occupied(e) => {
                let dev_info = e.get();

                if dev_info.owner == driver {
                    Ok((
                        mk_report_func(dev_info.tx_reading.clone(), name),
                        None,
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
        &mut self, driver: &str, name: &str, units: &Option<String>,
    ) -> Result<(ReportReading, RxDeviceSetting, Option<Value>)> {
        // Check to see if the device name already exists.

        match self.0.entry(String::from(name)) {
            // The device didn't exist. Create it and associate it
            // with the driver.
            hash_map::Entry::Vacant(e) => {
                // Create a broadcast channel. Slow clients will get
                // an error if they miss an update.

                let (tx, _) = broadcast::channel(CHAN_SIZE);

                // Create a channel with which to send settings.

                let (tx_sets, rx_sets) = mpsc::channel(CHAN_SIZE);

                // Build the entry and insert it in the table.

                let _ = e.insert(DeviceInfo {
                    owner: String::from(driver),
                    _units: units.clone(),
                    tx_reading: tx.clone(),
                    tx_setting: Some(tx_sets),
                });

                // Create and return the closure that the driver will
                // use to report updates.

                Ok((mk_report_func(tx, name), rx_sets, None))
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

                    Ok((
                        mk_report_func(dev_info.tx_reading.clone(), name),
                        rx_sets,
                        None,
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
    use crate::{mk_report_func, SimpleStore, CHAN_SIZE};
    use drmem_api::{types::device::Value, Store};
    use std::collections::HashMap;
    use tokio::sync::broadcast;

    #[tokio::test]
    async fn test_ro_registration() {
        let mut db = SimpleStore(HashMap::new());

        // Register a device named "junk" and associate it with the
        // driver named "test". We don't define units for this device.

        if let Ok((_, None)) =
            db.register_read_only_device("test", "junk", &None).await
        {
            // Make sure the device was defined and the setting
            // channel is `None`.

            assert!(db.0.get("junk").unwrap().tx_setting.is_none());

            // Create a receiving handle for device updates.

            let mut rx = db.0.get("junk").unwrap().tx_reading.subscribe();

            // Assert that re-registering this device with a different
            // driver name results in an error.

            assert!(db
                .register_read_only_device("test2", "junk", &None)
                .await
                .is_err());

            // Assert that re-registering this device with the same
            // driver name is successful.

            if let Ok((f, None)) =
                db.register_read_only_device("test", "junk", &None).await
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
    async fn test_closure() {
        let (tx, rx) = broadcast::channel(CHAN_SIZE);

        std::mem::drop(rx);

        let f = mk_report_func(tx.clone(), "misc");

        assert!(f(Value::Int(1)).await.is_ok());

        {
            let mut rx = tx.subscribe();

            assert!(f(Value::Int(2)).await.is_ok());
            assert_eq!(rx.try_recv(), Ok(Value::Int(2)));
        }

        assert!(f(Value::Int(3)).await.is_ok());

        {
            let mut rx1 = tx.subscribe();
            let mut rx2 = tx.subscribe();

            assert!(f(Value::Int(4)).await.is_ok());
            assert_eq!(rx1.try_recv(), Ok(Value::Int(4)));
            assert_eq!(rx2.try_recv(), Ok(Value::Int(4)));
        }
    }
}

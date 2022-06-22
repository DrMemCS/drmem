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
    client,
    driver::{ReportReading, RxDeviceSetting, TxDeviceSetting},
    types::{device, Error},
    Result, Store,
};
use drmem_config::backend;
use std::collections::{hash_map, HashMap};
use std::{time, sync::{Arc, Mutex}};
use tokio::sync::{broadcast, mpsc, oneshot};
use tracing::error;

const CHAN_SIZE: usize = 20;

struct DeviceInfo {
    owner: String,
    units: Option<String>,
    tx_setting: Option<TxDeviceSetting>,
    reading: Arc<
        Mutex<(
            broadcast::Sender<device::Reading>,
            Option<device::Reading>,
            time::SystemTime,
        )>,
    >,
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
            tx_setting,
            reading: Arc::new(Mutex::new((tx, None, time::UNIX_EPOCH))),
        }
    }
}

struct SimpleStore(HashMap<device::Name, DeviceInfo>);

pub async fn open(_cfg: &backend::Config) -> Result<impl Store> {
    Ok(SimpleStore(HashMap::new()))
}

// Builds the `ReportReading` function. Drivers will call specialized
// instances of this function to record the latest value of a device.

fn mk_report_func(di: &DeviceInfo, name: &device::Name) -> ReportReading {
    let reading = di.reading.clone();
    let name = name.to_string();

    Box::new(move |v| {
	// Determine the timestamp *before* we take the mutex. The
	// timing shouldn't pay the price of waiting for the mutex so
	// we grab it right away.

        let mut ts = time::SystemTime::now();

        // If a lock is obtained, update the current value. The only
        // way a lock can fail is if it's "poisoned", which means
        // another thread panicked while holding the lock. This module
        // holds the only code that uses the mutex and all accesses
        // are short and infallible, so the error message shouldn't
        // ever get displayed.

        if let Ok(mut data) = reading.lock() {

	    // At this point, we have access to the previous
	    // timestamp. If the new timestamp is *before* the
	    // previous, then we fudge the timestamp to be 1 𝜇s later
	    // (DrMem doesn't allow data values to be inserted in
	    // random order.) If, somehow, the timestamp will exceed
	    // the range of the `SystemTime` type, the maxmimum
	    // timestamp will be used for this sample (as well as
	    // future samples.)

	    if ts <= data.2 {
		if let Some(nts) = data.2.checked_add(time::Duration::from_micros(1)) {
		    ts = nts
		} else {
		    ts = time::UNIX_EPOCH.checked_add(
			time::Duration::new(i64::MAX as u64, 999_999_999)
		    ).unwrap()
		}
	    }

	    let reading = device::Reading { ts, value: v.clone() };
            let _ = data.0.send(reading.clone());

	    // Update the device's state.

            data.1 = Some(reading);
	    data.2 = ts
        } else {
            error!("couldn't set current value of {}", &name)
        }
        Box::pin(async { () })
    })
}

#[async_trait]
impl Store for SimpleStore {
    /// Handle read-only devices registration. This function creates
    /// an association between the device name and its associated
    /// resources. Since the driver is registering a read-only device,
    /// this function doesn't allocate a channel to provide settings.

    async fn register_read_only_device(
        &mut self, driver: &str, name: &device::Name, units: &Option<String>,
    ) -> Result<(ReportReading, Option<device::Value>)> {
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
                    let guard = dev_info.reading.lock();

                    Ok((
                        func,
                        if let Ok(data) = guard {
                            data.1.as_ref().map(
                                |device::Reading { value, .. }| value.clone(),
                            )
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
        &mut self, driver: &str, name: &device::Name, units: &Option<String>,
    ) -> Result<(ReportReading, RxDeviceSetting, Option<device::Value>)> {
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
                    let guard = dev_info.reading.lock();

                    Ok((
                        func,
                        rx_sets,
                        if let Ok(data) = guard {
                            data.1.as_ref().map(
                                |device::Reading { value, .. }| value.clone(),
                            )
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

    async fn get_device_info(
        &self, pattern: &Option<String>,
    ) -> Result<Vec<client::DevInfoReply>> {
        let pred: Box<dyn FnMut(&(&device::Name, &DeviceInfo)) -> bool> =
            if let Some(pattern) = pattern {
                if let Ok(pattern) = pattern.parse::<device::Name>() {
                    Box::new(move |(k, _)| pattern == **k)
                } else {
                    Box::new(|_| false)
                }
            } else {
                Box::new(|_| true)
            };
        let res: Vec<client::DevInfoReply> = self
            .0
            .iter()
            .filter(pred)
            .map(|(k, v)| client::DevInfoReply {
                name: k.clone(),
                units: v.units.clone(),
                driver: v.owner.clone(),
            })
            .collect();

        Ok(res)
    }

    async fn set_device(
        &self, name: device::Name, value: device::Value,
    ) -> Result<device::Value> {
        if let Some(di) = self.0.get(&name) {
            if let Some(tx) = &di.tx_setting {
                let (tx_rpy, rx_rpy) = oneshot::channel();

                match tx.send((value, tx_rpy)).await {
                    Ok(()) => match rx_rpy.await {
                        Ok(reply) => reply,
                        Err(_) => Err(Error::MissingPeer(
                            "driver broke connection".to_string(),
                        )),
                    },
                    Err(_) => Err(Error::MissingPeer(
                        "driver is ignoring settings".to_string(),
                    )),
                }
            } else {
                Err(Error::OperationError)
            }
        } else {
            Err(Error::NotFound)
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{mk_report_func, DeviceInfo, SimpleStore};
    use drmem_api::{types::device, Store};
    use std::{time, collections::HashMap};
    use tokio::sync::{mpsc::error::TryRecvError, oneshot};

    #[test]
    fn test_timestamp() {
	assert!(
	    time::UNIX_EPOCH.checked_add(
		time::Duration::new(i64::MAX as u64, 999_999_999)
	    ).is_some()
	)
    }

    #[tokio::test]
    async fn test_ro_registration() {
        let mut db = SimpleStore(HashMap::new());
        let name = "misc:junk".parse::<device::Name>().unwrap();

        // Register a device named "junk" and associate it with the
        // driver named "test". We don't define units for this device.

        if let Ok((f, None)) =
            db.register_read_only_device("test", &name, &None).await
        {
            // Make sure the device was defined and the setting
            // channel is `None`.

            assert!(db.0.get(&name).unwrap().tx_setting.is_none());

            // Report a value.

            f(device::Value::Int(1)).await;

            // Create a receiving handle for device updates.

            let mut rx =
                db.0.get(&name)
                    .unwrap()
                    .reading
                    .lock()
                    .unwrap()
                    .0
                    .subscribe();

            // Assert that re-registering this device with a different
            // driver name results in an error.

            assert!(db
                .register_read_only_device("test2", &name, &None)
                .await
                .is_err());

            // Assert that re-registering this device with the same
            // driver name is successful.

            if let Ok((f, Some(device::Value::Int(1)))) =
                db.register_read_only_device("test", &name, &None).await
            {
                // Also, verify that the device update channel wasn't
                // disrupted by sending a value and receiving it from
                // the receive handle we opened before re-registering.

                f(device::Value::Int(2)).await;
                assert_eq!(rx.try_recv().unwrap().value, device::Value::Int(2));
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
        let name = "misc:junk".parse::<device::Name>().unwrap();

        // Register a device named "junk" and associate it with the
        // driver named "test". We don't define units for this device.

        if let Ok((f, mut set_chan, None)) =
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

                let (tx_os, _rx_os) = oneshot::channel();

                assert!(tx_set
                    .send((device::Value::Int(2), tx_os))
                    .await
                    .is_ok());
                assert_eq!(
                    set_chan.try_recv().unwrap().0,
                    device::Value::Int(2)
                );
            }

            // Report a value.

            f(device::Value::Int(1)).await;

            // Create a receiving handle for device updates.

            let mut rx =
                db.0.get(&name)
                    .unwrap()
                    .reading
                    .lock()
                    .unwrap()
                    .0
                    .subscribe();

            // Assert that re-registering this device with a different
            // driver name results in an error. Also verify that it
            // didn't affect the setting channel.

            assert!(db
                .register_read_only_device("test2", &name, &None)
                .await
                .is_err());
            assert_eq!(
                Err(TryRecvError::Empty),
                set_chan.try_recv().map(|_| ())
            );

            // Assert that re-registering this device with the same
            // driver name is successful.

            if let Ok((f, _, Some(device::Value::Int(1)))) =
                db.register_read_write_device("test", &name, &None).await
            {
                assert_eq!(
                    Err(TryRecvError::Disconnected),
                    set_chan.try_recv().map(|_| ())
                );

                // Also, verify that the device update channel wasn't
                // disrupted by sending a value and receiving it from
                // the receive handle we opened before re-registering.

                f(device::Value::Int(2)).await;
                assert_eq!(rx.try_recv().unwrap().value, device::Value::Int(2));
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
        let name = "misc:junk".parse::<device::Name>().unwrap();
        let f = mk_report_func(&di, &name);

        assert_eq!(di.reading.lock().unwrap().1, None);
        f(device::Value::Int(1)).await;
        assert_eq!(
            di.reading.lock().unwrap().1.as_ref().unwrap().value,
            device::Value::Int(1)
        );

        {
	    let ts1 = di.reading.lock().unwrap().1.as_ref().unwrap().ts;
            let mut rx = di.reading.lock().unwrap().0.subscribe();

            f(device::Value::Int(2)).await;
            assert_eq!(rx.try_recv().unwrap().value, device::Value::Int(2));
            assert_eq!(
                di.reading.lock().unwrap().1.as_ref().unwrap().value,
                device::Value::Int(2)
            );
	    assert!(ts1 < di.reading.lock().unwrap().1.as_ref().unwrap().ts);
        }

        f(device::Value::Int(3)).await;
        assert_eq!(
            di.reading.lock().unwrap().1.as_ref().unwrap().value,
            device::Value::Int(3)
        );

        {
            let mut rx1 = di.reading.lock().unwrap().0.subscribe();
            let mut rx2 = di.reading.lock().unwrap().0.subscribe();

            f(device::Value::Int(4)).await;
            assert_eq!(rx1.try_recv().unwrap().value, device::Value::Int(4));
            assert_eq!(rx2.try_recv().unwrap().value, device::Value::Int(4));
            assert_eq!(
                di.reading.lock().unwrap().1.as_ref().unwrap().value,
                device::Value::Int(4)
            );
        }
    }
}

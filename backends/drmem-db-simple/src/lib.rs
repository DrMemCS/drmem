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
use std::collections::{hash_map, HashMap};
use tokio::sync::{broadcast, mpsc};

struct DeviceInfo {
    _tx_reading: broadcast::Sender<Value>,
    _tx_setting: Option<TxDeviceSetting>,
}

struct SimpleStore(HashMap<String, DeviceInfo>);

pub async fn open() -> Result<impl Store> {
    Ok(SimpleStore(HashMap::new()))
}

#[async_trait]
impl Store for SimpleStore {
    /// Handle read-only devices registration. This function creates
    /// an association between the device name and its associated
    /// resources. Since the driver is registering a read-only device,
    /// this function doesn't allocate a channel to provide settings.

    async fn register_read_only_device(
        &mut self, name: &str,
    ) -> Result<ReportReading> {
        // Check to see if the device name already exists. It is does,
        // we return an `InUse` error. Otherwise we hang onto the
        // location in which we can write the entry.

        if let hash_map::Entry::Vacant(e) = self.0.entry(String::from(name)) {
            // Create a broadcast channel. The simple backend doesn't
            // keep a history so we set the depth to 1. Slow clients
            // will get an error if they miss an update.

            let (tx, _) = broadcast::channel(1);

            // Build the entry and insert it in the table.

            let _ = e.insert(DeviceInfo {
                _tx_reading: tx.clone(),
                _tx_setting: None,
            });

            // Create and return the closure that the driver will use
            // to report updates.

            Ok(Box::new(move |v| {
                let _ = tx.send(v);
                Ok(())
            }))
        } else {
            Err(Error::InUse)
        }
    }

    /// Handle read-write devices registration. This function creates
    /// an association between the device name and its associated
    /// resources.

    async fn register_read_write_device(
        &mut self, name: &str,
    ) -> Result<(ReportReading, RxDeviceSetting, Option<Value>)> {
        // Check to see if the device name already exists. It is does,
        // we return an `InUse` error. Otherwise we hang onto the
        // location in which we can write the entry.

        if let hash_map::Entry::Vacant(e) = self.0.entry(String::from(name)) {
            // Create a broadcast channel. The simple backend doesn't
            // keep a history so we set the depth to 1. Slow clients
            // will get an error if they miss an update.

            let (tx, _) = broadcast::channel(1);

            // Create a channel with which to send settings.

            let (tx_sets, rx_sets) = mpsc::channel(20);

            // Build the entry and insert it in the table.

            let _ = e.insert(DeviceInfo {
                _tx_reading: tx.clone(),
                _tx_setting: Some(tx_sets),
            });

            // Create and return the closure that the driver will use
            // to report updates.

            Ok((
                Box::new(move |v| {
                    let _ = tx.send(v);
                    Ok(())
                }),
                rx_sets,
                None,
            ))
        } else {
            Err(Error::InUse)
        }
    }
}

#[cfg(test)]
mod tests {}

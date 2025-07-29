//! Define classes of devices.
//!
//! This module provides a set of types that define a consistent set
//! of devices for classes of hardware devices. For instance, drivers
//! should use the `ColorLight` type if it controls color, LED
//! bulbs. This type will device the set of DrMem devices that are
//! expected from every color LED bulb.
use super::{
    ro_device::ReadOnlyDevice, rw_device::ReadWriteDevice, DriverConfig,
    Registrator, RequestChan, Result,
};
use std::future::Future;

// Define a "marker" trait for registering switches.

pub struct Switch;

impl Registrator for Switch {
    type DeviceSet = ReadWriteDevice<bool>;

    fn register_devices<'a>(
        drc: &'a mut RequestChan,
        _cfg: &DriverConfig,
        max_history: Option<usize>,
    ) -> impl Future<Output = Result<Self::DeviceSet>> + Send + 'a {
        async move {
            drc.add_rw_device::<bool>("state".parse()?, None, max_history)
                .await
        }
    }
}

pub struct Dimmer;

pub struct DimmerSet {
    pub error: ReadOnlyDevice<bool>,
    pub brightness: ReadWriteDevice<f64>,
    pub indicator: ReadWriteDevice<bool>,
}

impl Registrator for Dimmer {
    type DeviceSet = DimmerSet;

    fn register_devices<'a>(
        drc: &'a mut RequestChan,
        _cfg: &DriverConfig,
        max_history: Option<usize>,
    ) -> impl Future<Output = Result<Self::DeviceSet>> + Send + 'a {
        let nm_state = "error".parse().unwrap();
        let nm_brightness = "brightness".parse().unwrap();
        let nm_indicator = "indicator".parse().unwrap();

        async move {
            Ok(DimmerSet {
                error: drc
                    .add_ro_device::<bool>(nm_state, None, max_history)
                    .await?,
                brightness: drc
                    .add_rw_device::<f64>(nm_brightness, Some("%"), max_history)
                    .await?,
                indicator: drc
                    .add_rw_device::<bool>(nm_indicator, None, max_history)
                    .await?,
            })
        }
    }
}

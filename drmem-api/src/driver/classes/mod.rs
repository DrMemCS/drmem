//! Define classes of devices.
//!
//! This module provides a set of types that define a consistent set
//! of devices for classes of hardware devices. For instance, drivers
//! should use the `ColorLight` type if it controls color, LED
//! bulbs. This type will device the set of DrMem devices that are
//! expected from every color LED bulb.
use super::{
    rw_device::ReadWriteDevice, DriverConfig, Registrator, RequestChan, Result,
};
use std::future::Future;

pub struct Switch;

impl Registrator for Switch {
    type DeviceSet = ReadWriteDevice<bool>;

    fn register_devices(
        drc: RequestChan,
        _cfg: &DriverConfig,
        max_history: Option<usize>,
    ) -> impl Future<Output = Result<Self::DeviceSet>> + Send {
        async move {
            drc.add_rw_device::<bool>("state".parse()?, None, max_history)
                .await
        }
    }
}

pub struct Dimmer;

pub struct DimmerSet {
    pub state: ReadWriteDevice<bool>,
    pub brightness: ReadWriteDevice<f64>,
}

impl Registrator for Dimmer {
    type DeviceSet = DimmerSet;

    fn register_devices(
        drc: RequestChan,
        _cfg: &DriverConfig,
        max_history: Option<usize>,
    ) -> impl Future<Output = Result<Self::DeviceSet>> + Send {
        let nm_state = "state".parse();
        let nm_brightness = "brightness".parse();

        async move {
            Ok(DimmerSet {
                state: drc
                    .add_rw_device::<bool>(nm_state?, None, max_history)
                    .await?,
                brightness: drc
                    .add_rw_device::<f64>(
                        nm_brightness?,
                        Some("%"),
                        max_history,
                    )
                    .await?,
            })
        }
    }
}

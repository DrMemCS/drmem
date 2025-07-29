use crate::driver::{
    ro_device::ReadOnlyDevice, rw_device::ReadWriteDevice, DriverConfig,
    Registrator, RequestChan, Result,
};
use std::future::Future;

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
        let nm_error = "error".parse();
        let nm_brightness = "brightness".parse();
        let nm_indicator = "indicator".parse();

        async move {
            // Report any errors before creating any device channels.

            let nm_error = nm_error?;
            let nm_brightness = nm_brightness?;
            let nm_indicator = nm_indicator?;

            // Build the set of channels.

            Ok(DimmerSet {
                error: drc
                    .add_ro_device::<bool>(nm_error, None, max_history)
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

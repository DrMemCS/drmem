/// Trait-based device handling for Hue devices
use super::{constants, payload};
use drmem_api::driver::{Reporter, ResettableState, classes};
use tracing::debug;

/// Common interface for all Hue device types
pub trait HueDevice<R: Reporter> {
    /// Returns the resource type for this device ("light" or "grouped_light")
    fn resource_type(&self) -> &'static str;

    /// Wait for the next setting change and return a command if one is ready
    fn next_setting(
        &mut self,
    ) -> impl std::future::Future<Output = Option<payload::LightCommand>> + Send;

    /// Apply an update from the bridge to the device
    fn apply_update(
        &mut self,
        update: &payload::ResourceData,
    ) -> impl std::future::Future<Output = ()> + Send;

    /// Reset the device state (called when driver restarts)
    fn reset(&mut self);
}

/// Wrapper for Switch devices
pub struct SwitchDevice<R: Reporter> {
    pub inner: classes::Switch<R>,
}

impl<R: Reporter> HueDevice<R> for SwitchDevice<R> {
    fn resource_type(&self) -> &'static str {
        constants::LIGHT_RESOURCE
    }

    async fn next_setting(&mut self) -> Option<payload::LightCommand> {
        loop {
            tokio::select! {
                // Check state device
                opt_txn = self.inner.state.next_setting() => {
                    if let Some((val, reply)) = opt_txn {
                        debug!("switch state setting ready: {}", val);
                        if let Some(r) = reply {
                            r.ok(val);
                        }
                        return Some(payload::LightCommand {
                            on: Some(payload::On { on: val }),
                            dimming: None,
                            color: None,
                        });
                    }
                }

                // Check indicator device (drain but don't send command)
                opt_txn = self.inner.indicator.next_setting() => {
                    if let Some((val, reply)) = opt_txn {
                        if let Some(r) = reply {
                            r.ok(val);
                        }
                        return None;
                    }
                }
            }
        }
    }

    async fn apply_update(&mut self, update: &payload::ResourceData) {
        if let Some(on) = &update.on {
            debug!("switch: reporting state update: {}", on.on);
            self.inner.state.report_update(on.on).await;
        }
    }

    fn reset(&mut self) {
        self.inner.state.reset_state();
        self.inner.indicator.reset_state();
    }
}

/// Wrapper for Dimmer/Bulb devices
pub struct DimmerDevice<R: Reporter> {
    pub inner: classes::Dimmer<R>,
}

impl<R: Reporter> HueDevice<R> for DimmerDevice<R> {
    fn resource_type(&self) -> &'static str {
        constants::LIGHT_RESOURCE
    }

    async fn next_setting(&mut self) -> Option<payload::LightCommand> {
        loop {
            tokio::select! {
                // Check brightness device
                opt_txn = self.inner.brightness.next_setting() => {
                    if let Some((val, reply)) = opt_txn {
                        let val = val.clamp(0.0, 100.0).round();
                        debug!("dimmer brightness setting ready: {}", val);

                        if let Some(r) = reply {
                            r.ok(val);
                        }

                        let cmd = if val == 0.0 {
                            payload::LightCommand {
                                on: Some(payload::On { on: false }),
                                dimming: None,
                                color: None,
                            }
                        } else {
                            payload::LightCommand {
                                on: Some(payload::On { on: true }),
                                dimming: Some(payload::Dimming {
                                    brightness: val as f32,
                                }),
                                color: None,
                            }
                        };
                        return Some(cmd);
                    }
                }

                // Check indicator device (drain but don't send command)
                opt_txn = self.inner.indicator.next_setting() => {
                    if let Some((val, reply)) = opt_txn {
                        if let Some(r) = reply {
                            r.ok(val);
                        }
                        return None;
                    }
                }
            }
        }
    }

    async fn apply_update(&mut self, update: &payload::ResourceData) {
        if let Some(on) = &update.on {
            if !on.on {
                debug!("dimmer: reporting brightness update: 0");
                self.inner.brightness.report_update(0.0).await;
            } else if let Some(dim) = &update.dimming {
                let brightness = (dim.brightness as f64).round();

                debug!("dimmer: reporting brightness update: {}", brightness);
                self.inner.brightness.report_update(brightness).await;
            } else {
                // Use 100% if brightness is missing when device is on
                debug!("dimmer: reporting brightness update: 100 (default)");
                self.inner.brightness.report_update(100.0).await;
            }
        } else if let Some(dim) = &update.dimming {
            let brightness = (dim.brightness as f64).round();

            debug!("dimmer: reporting brightness update: {}", brightness);
            self.inner.brightness.report_update(brightness).await;
        }
    }

    fn reset(&mut self) {
        self.inner.brightness.reset_state();
        self.inner.indicator.reset_state();
    }
}

/// Wrapper for ColorBulb devices
pub struct ColorBulbDevice<R: Reporter> {
    pub inner: classes::ColorBulb<R>,
    resource_type: &'static str,
}

impl<R: Reporter> ColorBulbDevice<R> {
    pub fn new(inner: classes::ColorBulb<R>, is_group: bool) -> Self {
        Self {
            inner,
            resource_type: if is_group {
                constants::GROUPED_LIGHT_RESOURCE
            } else {
                constants::LIGHT_RESOURCE
            },
        }
    }
}

impl<R: Reporter> HueDevice<R> for ColorBulbDevice<R> {
    fn resource_type(&self) -> &'static str {
        self.resource_type
    }

    async fn next_setting(&mut self) -> Option<payload::LightCommand> {
        loop {
            tokio::select! {
                // Check brightness device
                opt_txn = self.inner.brightness.next_setting() => {
                    if let Some((val, reply)) = opt_txn {
                        let val = val.clamp(0.0, 100.0).round();
                        debug!("colorbulb brightness setting ready: {}", val);

                        if let Some(r) = reply {
                            r.ok(val);
                        }

                        let cmd = if val == 0.0 {
                            payload::LightCommand {
                                on: Some(payload::On { on: false }),
                                dimming: None,
                                color: None,
                            }
                        } else {
                            payload::LightCommand {
                                on: Some(payload::On { on: true }),
                                dimming: Some(payload::Dimming {
                                    brightness: val as f32,
                                }),
                                color: None,
                            }
                        };
                        return Some(cmd);
                    }
                }

                // Check color device
                opt_txn = self.inner.color.next_setting() => {
                    if let Some((val, reply)) = opt_txn {
                        debug!("colorbulb color setting ready: {:?}", val);

                        if let Some(r) = reply {
                            r.ok(val.clone());
                        }

                        let (x, y) = super::color::rgba_to_cie_xy(&val);

                        return Some(payload::LightCommand {
                            on: Some(payload::On { on: true }),
                            dimming: None,
                            color: Some(payload::Color {
                                xy: Some(payload::XyCoordinates { x, y }),
                            }),
                        });
                    }
                }
            }
        }
    }

    async fn apply_update(&mut self, update: &payload::ResourceData) {
        // Handle brightness updates
        if let Some(on) = &update.on {
            if !on.on {
                debug!("colorbulb: reporting brightness update: 0");
                self.inner.brightness.report_update(0.0).await;
            } else if let Some(dim) = &update.dimming {
                let brightness = (dim.brightness as f64).round();

                debug!(
                    "colorbulb: reporting brightness update: {}",
                    brightness
                );
                self.inner.brightness.report_update(brightness).await;
            } else {
                // Use 100% if brightness is missing when device is on
                debug!("colorbulb: reporting brightness update: 100 (default)");
                self.inner.brightness.report_update(100.0).await;
            }
        } else if let Some(dim) = &update.dimming {
            let brightness = (dim.brightness as f64).round();

            debug!("colorbulb: reporting brightness update: {}", brightness);
            self.inner.brightness.report_update(brightness).await;
        }

        // Handle color updates
        if let Some(color) = &update.color.as_ref().and_then(|c| c.xy.as_ref())
        {
            let rgba = super::color::cie_xy_to_rgba(color.x, color.y);

            debug!("colorbulb: reporting color update: {:?}", rgba);
            self.inner.color.report_update(rgba).await;
        }
    }

    fn reset(&mut self) {
        self.inner.brightness.reset_state();
        self.inner.color.reset_state();
    }
}

/// Type-erased device wrapper
pub enum DeviceWrapper<R: Reporter> {
    Switch(SwitchDevice<R>),
    Dimmer(DimmerDevice<R>),
    ColorBulb(ColorBulbDevice<R>),
}

impl<R: Reporter> DeviceWrapper<R> {
    pub fn resource_type(&self) -> &'static str {
        match self {
            Self::Switch(d) => d.resource_type(),
            Self::Dimmer(d) => d.resource_type(),
            Self::ColorBulb(d) => d.resource_type(),
        }
    }

    pub async fn next_setting(&mut self) -> Option<payload::LightCommand> {
        match self {
            Self::Switch(d) => d.next_setting().await,
            Self::Dimmer(d) => d.next_setting().await,
            Self::ColorBulb(d) => d.next_setting().await,
        }
    }

    pub async fn apply_update(&mut self, update: &payload::ResourceData) {
        match self {
            Self::Switch(d) => d.apply_update(update).await,
            Self::Dimmer(d) => d.apply_update(update).await,
            Self::ColorBulb(d) => d.apply_update(update).await,
        }
    }

    pub fn reset(&mut self) {
        match self {
            Self::Switch(d) => d.reset(),
            Self::Dimmer(d) => d.reset(),
            Self::ColorBulb(d) => d.reset(),
        }
    }
}

/// Trait-based device handling for Hue devices
use super::{color, constants, payload};
use drmem_api::{
    Result,
    device::{ColorType, Path},
    driver::{
        OverrideConfig, Registrator, Reporter, RequestChan, ResettableState,
        classes,
    },
};
use palette::LinSrgba;
use tracing::debug;

/// Common interface for all Hue device types
pub trait HueDevice<R: Reporter> {
    /// Returns the resource type for this device ("light" or "grouped_light")
    fn resource_type(&self) -> &'static str;

    /// Wait for the next setting change and return a command if one is ready
    async fn next_setting(&mut self) -> Option<payload::LightCommand>;

    /// Apply an update from the bridge to the device
    async fn apply_update(&mut self, update: &payload::ResourceData) -> ();

    /// Reset the device state (called when driver restarts)
    fn reset(&mut self);
}

/// Wrapper for Switch devices
pub struct SwitchDevice<R: Reporter>(pub classes::Switch<R>);

impl<R: Reporter> HueDevice<R> for SwitchDevice<R> {
    fn resource_type(&self) -> &'static str {
        constants::LIGHT_RESOURCE
    }

    async fn next_setting(&mut self) -> Option<payload::LightCommand> {
        self.0.next_setting().await.map(|val| {
            debug!("switch state setting ready: {}", val.state);
            payload::LightCommand {
                on: Some(payload::On { on: val.state }),
                dimming: None,
                color: None,
            }
        })
    }

    async fn apply_update(&mut self, update: &payload::ResourceData) {
        if let Some(on) = &update.on {
            debug!("switch: reporting state update: {}", on.on);
            self.0
                .report_update(classes::SwitchProperty { state: on.on })
                .await;
        }
    }

    fn reset(&mut self) {
        self.0.reset_state()
    }
}

/// Wrapper for Dimmer/Bulb devices
pub struct DimmerDevice<R: Reporter> {
    pub dimmer: classes::Dimmer<R>,
}

impl<R: Reporter> ResettableState for DimmerDevice<R> {}

impl<R: Reporter> Registrator<R> for DimmerDevice<R> {
    type Config = OverrideConfig;

    async fn register_devices(
        drc: &mut RequestChan<R>,
        subpath: Option<&Path>,
        cfg: &Self::Config,
        max_history: Option<usize>,
    ) -> Result<Self> {
        Ok(DimmerDevice {
            dimmer: classes::Dimmer::register_devices(
                drc,
                subpath,
                cfg,
                max_history,
            )
            .await?,
        })
    }
}

impl<R: Reporter> HueDevice<R> for DimmerDevice<R> {
    fn resource_type(&self) -> &'static str {
        constants::LIGHT_RESOURCE
    }

    async fn next_setting(&mut self) -> Option<payload::LightCommand> {
        if let Some(brightness) = self.dimmer.next_setting().await {
            let cmd = if brightness.brightness == 0.0 {
                payload::LightCommand {
                    on: Some(payload::On { on: false }),
                    dimming: None,
                    color: None,
                }
            } else {
                payload::LightCommand {
                    on: Some(payload::On { on: true }),
                    dimming: Some(payload::Dimming {
                        brightness: brightness.brightness as f32,
                    }),
                    color: None,
                }
            };
            Some(cmd)
        } else {
            Some(payload::LightCommand {
                on: None,
                dimming: None,
                color: None,
            })
        }
    }

    async fn apply_update(&mut self, update: &payload::ResourceData) {
        let brightness = match (&update.on, &update.dimming) {
            (Some(payload::On { on: false }), _) => 0.0,
            (Some(payload::On { on: true }), None) => 100.0,
            (_, Some(dim)) => (dim.brightness as f64).round(),
            (None, None) => return,
        };

        debug!("dimmer: brightness update: {}", brightness);
        self.dimmer
            .report_update(classes::DimmerProperty { brightness })
            .await;
    }

    fn reset(&mut self) {
        self.dimmer.reset_state();
    }
}

/// Wrapper for ColorBulb devices
pub struct ColorBulbDevice<R: Reporter> {
    pub inner: classes::ColorBulb<R>,
    resource_type: &'static str,
    /// Stores the last XY coordinates sent to the bridge to avoid
    /// round-off errors when comparing RGB <-> XY conversions
    last_xy: Option<(f32, f32)>,
    /// The last color reported to DrMem, used as the baseline when
    /// merging partial bridge updates (on/dimming/color can each
    /// arrive independently).
    last_color: ColorType,
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
            last_xy: None,
            last_color: ColorType::Rgba {
                color: LinSrgba::new(255, 255, 255, 0),
            },
        }
    }
}

impl<R: Reporter> HueDevice<R> for ColorBulbDevice<R> {
    fn resource_type(&self) -> &'static str {
        self.resource_type
    }

    async fn next_setting(&mut self) -> Option<payload::LightCommand> {
        let (val, reply) = self.inner.color.next_setting().await?;

        debug!("colorbulb color setting ready: {:?}", val);

        if let Some(r) = reply {
            r.ok(val.clone());
        }

        let bridge = color::color_to_bridge(&val);

        // Store the XY coordinates we're sending to the bridge, and the
        // setting itself so `apply_update` knows which `ColorType`
        // variant (`Rgba` or `Ccta`) to preserve when the bridge just
        // echoes back the xy we sent.
        self.last_xy = Some(bridge.xy);
        self.last_color = val;

        Some(payload::LightCommand {
            on: Some(payload::On { on: bridge.on }),
            dimming: bridge.on.then_some(payload::Dimming {
                brightness: bridge.brightness,
            }),
            color: Some(payload::Color {
                xy: Some(payload::XyCoordinates {
                    x: bridge.xy.0,
                    y: bridge.xy.1,
                }),
            }),
        })
    }

    async fn apply_update(&mut self, update: &payload::ResourceData) {
        let on = update.on.as_ref().map(|on| on.on);
        let brightness = update.dimming.as_ref().map(|dim| dim.brightness);

        // Ignore XY coordinates that just echo what we last sent, to
        // avoid round-off errors from the RGB <-> XY conversion.
        let xy = update
            .color
            .as_ref()
            .and_then(|c| c.xy.as_ref())
            .map(|xy| (xy.x, xy.y))
            .filter(|&(x, y)| {
                !self.last_xy.is_some_and(|(last_x, last_y)| {
                    (last_x - x).abs() < 0.001 && (last_y - y).abs() < 0.001
                })
            });

        if on.is_none() && brightness.is_none() && xy.is_none() {
            return;
        }

        let merged =
            color::merge_bridge_update(&self.last_color, on, brightness, xy);

        debug!("colorbulb: reporting color update: {:?}", merged);

        if let Some(xy) = xy {
            self.last_xy = Some(xy);
        }
        self.last_color = merged.clone();
        self.inner.color.report_update(merged).await;
    }

    fn reset(&mut self) {
        self.inner.color.reset_state();
        self.last_xy = None;
        self.last_color = ColorType::Rgba {
            color: LinSrgba::new(255, 255, 255, 0),
        };
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

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
            .filter(|&xy| {
                !self
                    .last_xy
                    .is_some_and(|last_xy| color::xy_is_close(last_xy, xy))
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

#[cfg(test)]
mod tests {
    use super::*;
    use drmem_api::{
        device::Value,
        driver::{OverridableDevice, ReadOnlyDevice, TxDeviceSetting},
    };
    use tokio::{
        sync::{mpsc, oneshot},
        time::{Duration, advance, timeout},
    };

    // Same as the envelope in `Set::from_devcfg`.
    const ENVELOPE: Duration = Duration::from_secs(30);
    const POLL_INTERVAL: Duration = Duration::from_secs(5);

    struct MockReporter(mpsc::Sender<Value>);

    impl Reporter for MockReporter {
        async fn report_value(&mut self, v: Value) {
            self.0.send(v).await.unwrap()
        }
    }

    // Models how the bridge stores a light: brightness in 254 steps (reported
    // as a percentage) and xy to 4 decimal places, so what it reports back is
    // never bit-for-bit what was sent.
    struct FakeBridge {
        on: bool,
        level: u8,
        xy: (f32, f32),
    }

    impl FakeBridge {
        fn apply(&mut self, cmd: &payload::LightCommand) {
            let round4 = |v: f32| (v * 10_000.0).round() / 10_000.0;

            if let Some(on) = &cmd.on {
                self.on = on.on;
            }
            if let Some(dim) = &cmd.dimming {
                self.level = (dim.brightness / 100.0 * 254.0)
                    .round()
                    .clamp(1.0, 254.0) as u8;
            }
            if let Some(xy) = cmd.color.as_ref().and_then(|c| c.xy.as_ref()) {
                self.xy = (round4(xy.x), round4(xy.y));
            }
        }

        fn poll(&self) -> payload::ResourceData {
            payload::ResourceData {
                id: "test".into(),
                on: Some(payload::On { on: self.on }),
                dimming: Some(payload::Dimming {
                    brightness: (self.level as f32 / 254.0 * 10_000.0).round()
                        / 100.0,
                }),
                color: Some(payload::Color {
                    xy: Some(payload::XyCoordinates {
                        x: self.xy.0,
                        y: self.xy.1,
                    }),
                }),
            }
        }
    }

    // A `ColorBulbDevice` wired to a `FakeBridge` the way the driver's
    // device loop wires it to a real one.
    struct Rig {
        tx_set: TxDeviceSetting,
        rx_rdg: mpsc::Receiver<Value>,
        dev: ColorBulbDevice<MockReporter>,
        bridge: FakeBridge,
    }

    impl Rig {
        async fn new() -> Self {
            let (rdg_tx, rx_rdg) = mpsc::channel(20);
            let (tx_set, rx_set) = mpsc::channel(20);
            let mut rig = Rig {
                tx_set,
                rx_rdg,
                dev: ColorBulbDevice::new(
                    classes::ColorBulb {
                        error: ReadOnlyDevice::new(MockReporter(
                            mpsc::channel(1).0,
                        )),
                        color: OverridableDevice::new(
                            MockReporter(rdg_tx),
                            rx_set,
                            None,
                            None,
                            Some(ENVELOPE),
                        ),
                    },
                    false,
                ),
                bridge: FakeBridge {
                    on: false,
                    level: 254,
                    xy: (0.5, 0.4),
                },
            };

            // The driver's first poll reports the light's initial state.
            rig.poll().await;
            while rig.rx_rdg.try_recv().is_ok() {}
            rig
        }

        async fn poll(&mut self) {
            let update = self.bridge.poll();

            self.dev.apply_update(&update).await;
        }

        // Sends `setting` to the device and feeds the bridge's polled state
        // back until the envelope has elapsed. A faithful bridge should
        // leave the device settled: one command, one backend report, no
        // reassertions and no override.
        async fn set(
            &mut self,
            setting: &ColorType,
        ) -> std::result::Result<(), String> {
            let (os_tx, _os_rx) = oneshot::channel();

            self.tx_set
                .send((setting.clone().into(), os_tx))
                .await
                .unwrap();

            let cmd = timeout(Duration::ZERO, self.dev.next_setting())
                .await
                .map_err(|_| "no command was issued")?
                .ok_or("setting stream closed")?;

            self.bridge.apply(&cmd);

            for _ in 0..3 {
                self.poll().await;

                if timeout(Duration::ZERO, self.dev.next_setting())
                    .await
                    .is_ok()
                {
                    return Err("setting was re-sent after a poll".into());
                }
                advance(POLL_INTERVAL).await;
            }

            if timeout(ENVELOPE * 2, self.dev.next_setting()).await.is_ok() {
                return Err("setting was re-sent after the envelope".into());
            }

            let mut seen = vec![];

            while let Ok(v) = self.rx_rdg.try_recv() {
                seen.push(v);
            }

            if seen != [Value::Color(setting.clone())] {
                return Err(format!("backend saw {seen:?}"));
            }
            Ok(())
        }
    }

    fn rgba(r: u8, g: u8, b: u8, a: u8) -> ColorType {
        ColorType::Rgba {
            color: LinSrgba::new(r, g, b, a),
        }
    }

    fn assert_no_failures(failures: Vec<String>) {
        assert!(failures.is_empty(), "{failures:#?}");
    }

    #[tokio::test(start_paused = true)]
    async fn test_rgba_brightness_settles() {
        let mut failures = vec![];

        for alpha in 0..=255 {
            if let Err(e) =
                Rig::new().await.set(&rgba(200, 100, 50, alpha)).await
            {
                failures.push(format!("alpha {alpha}: {e}"));
            }
        }
        assert_no_failures(failures);
    }

    #[tokio::test(start_paused = true)]
    async fn test_rgba_colors_settle() {
        let mut failures = vec![];

        for r in (0..=255).step_by(51) {
            for g in (0..=255).step_by(51) {
                for b in (0..=255).step_by(51) {
                    if let Err(e) =
                        Rig::new().await.set(&rgba(r, g, b, 255)).await
                    {
                        failures.push(format!("rgb({r},{g},{b}): {e}"));
                    }
                }
            }
        }
        assert_no_failures(failures);
    }

    #[tokio::test(start_paused = true)]
    async fn test_ccta_settles() {
        let mut failures = vec![];

        for a in 0..=255 {
            if let Err(e) = Rig::new()
                .await
                .set(&ColorType::Ccta { kelvin: 2700, a })
                .await
            {
                failures.push(format!("2700K alpha {a}: {e}"));
            }
        }
        for kelvin in (2000..=6500).step_by(500) {
            if let Err(e) = Rig::new()
                .await
                .set(&ColorType::Ccta { kelvin, a: 255 })
                .await
            {
                failures.push(format!("{kelvin}K: {e}"));
            }
        }
        assert_no_failures(failures);
    }

    #[tokio::test(start_paused = true)]
    async fn test_successive_settings_settle() {
        let mut rig = Rig::new().await;

        for setting in [
            rgba(255, 0, 0, 255),
            rgba(255, 0, 0, 127),
            ColorType::Ccta {
                kelvin: 2700,
                a: 200,
            },
            rgba(0, 0, 255, 0),
            rgba(10, 200, 30, 255),
            ColorType::Ccta {
                kelvin: 6500,
                a: 255,
            },
        ] {
            if let Err(e) = rig.set(&setting).await {
                panic!("{setting:?}: {e}");
            }
        }
    }
}

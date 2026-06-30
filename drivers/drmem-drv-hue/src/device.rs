use super::{config, driver::payload};
use drmem_api::{
    Result,
    device::Path,
    driver::{Registrator, Reporter, ResettableState, classes},
};
use palette::{IntoColor, LinSrgb, Yxy};
use std::{
    collections::HashMap,
    sync::Arc,
    task::{Context, Poll},
};
use tokio::time::Duration;

// Each type of device has a specific set of channels. In order to
// allow a map of different types, we wrap it in an enum.
pub enum DeviceSet<R: Reporter> {
    Switch(classes::Switch<R>),
    Bulb(classes::Dimmer<R>),
    ColorBulb(classes::ColorBulb<R>),
    Group(classes::ColorBulb<R>),
}

// The set of devices for an instance of this driver is held in a map
// which maps the device ID to its set of device channels.
pub struct Set<R: Reporter> {
    pub map: HashMap<Arc<str>, DeviceSet<R>>,
}

impl<R: Reporter> Set<R> {
    pub async fn from_devcfg(
        drc: &mut drmem_api::driver::RequestChan<R>,
        cfg: &config::DeviceConfig,
        max_history: Option<usize>,
    ) -> Result<(Arc<str>, DeviceSet<R>)> {
        let tmo = cfg.override_timeout.map(|v| Duration::from_secs(v * 60));

        Ok((
            cfg.id.clone(),
            match cfg.r#type {
                config::DevCfgType::Switch => DeviceSet::Switch(
                    classes::Switch::register_devices(
                        drc,
                        Some(cfg.subpath.as_ref()),
                        &tmo,
                        max_history,
                    )
                    .await?,
                ),
                config::DevCfgType::Dimmer | config::DevCfgType::Bulb => {
                    DeviceSet::Bulb(
                        classes::Dimmer::register_devices(
                            drc,
                            Some(cfg.subpath.as_ref()),
                            &tmo,
                            max_history,
                        )
                        .await?,
                    )
                }
                config::DevCfgType::ColorBulb => DeviceSet::ColorBulb(
                    classes::ColorBulb::register_devices(
                        drc,
                        Some(cfg.subpath.as_ref()),
                        &tmo,
                        max_history,
                    )
                    .await?,
                ),
                config::DevCfgType::Group => DeviceSet::Group(
                    classes::ColorBulb::register_devices(
                        drc,
                        Some(cfg.subpath.as_ref()),
                        &tmo,
                        max_history,
                    )
                    .await?,
                ),
            },
        ))
    }

    // Helper function to process the futures in a `Dimmer` type.

    fn process_bulb(
        id: Arc<str>,
        rtype: &'static str,
        dimmer: &mut classes::Dimmer<R>,
        cx: &mut Context,
    ) -> Poll<(Arc<str>, &'static str, Option<payload::LightCommand>)> {
        // Handle any incoming brightness settings.

        if let Poll::Ready(Some((val, reply))) =
            std::pin::pin!(dimmer.brightness.next_setting()).poll(cx)
        {
            let val = val.clamp(0.0, 100.0).round();

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
            return Poll::Ready((id, rtype, Some(cmd)));
        }

        // Hue devices don't have an indicator but we still have to
        // drain the channel. This sends a reply to the client, but
        // doesn't return a command to act upon.

        if let Poll::Ready(Some((val, reply))) =
            std::pin::pin!(dimmer.indicator.next_setting()).poll(cx)
        {
            if let Some(r) = reply {
                r.ok(val);
            }
            return Poll::Ready((id, rtype, None));
        }

        Poll::Pending
    }

    // Helper function to process the futures in a `ColorBulb` type.

    fn process_colorbulb(
        id: Arc<str>,
        rtype: &'static str,
        cb: &mut classes::ColorBulb<R>,
        cx: &mut Context,
    ) -> Poll<(Arc<str>, &'static str, Option<payload::LightCommand>)> {
        if let Poll::Ready(Some((val, reply))) =
            std::pin::pin!(cb.brightness.next_setting()).poll(cx)
        {
            let val = val.clamp(0.0, 100.0).round();

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

            return Poll::Ready((id, rtype, Some(cmd)));
        }

        if let Poll::Ready(Some((val, reply))) =
            std::pin::pin!(cb.color.next_setting()).poll(cx)
        {
            if let Some(r) = reply {
                r.ok(val.clone());
            }

            let rgb = LinSrgb::new(
                val.red as f32 / 255.0,
                val.green as f32 / 255.0,
                val.blue as f32 / 255.0,
            );
            let yxy: Yxy = rgb.into_color();

            return Poll::Ready((
                id,
                rtype,
                Some(payload::LightCommand {
                    on: Some(payload::On { on: true }),
                    dimming: None,
                    color: Some(payload::Color {
                        xy: Some(payload::XyCoordinates { x: yxy.x, y: yxy.y }),
                    }),
                }),
            ));
        }
        Poll::Pending
    }

    // Helper function to process the futures in a `Switch` type.

    fn process_switch(
        id: Arc<str>,
        rtype: &'static str,
        switch: &mut classes::Switch<R>,
        cx: &mut Context,
    ) -> Poll<(Arc<str>, &'static str, Option<payload::LightCommand>)> {
        // If the `state` device has a setting, return it.

        if let Poll::Ready(Some((val, reply))) =
            std::pin::pin!(switch.state.next_setting()).poll(cx)
        {
            if let Some(r) = reply {
                r.ok(val);
            }
            return Poll::Ready((
                id,
                rtype,
                Some(payload::LightCommand {
                    on: Some(payload::On { on: val }),
                    dimming: None,
                    color: None,
                }),
            ));
        }

        // If the `indicator` device has a setting, return it.

        if let Poll::Ready(Some((val, reply))) =
            std::pin::pin!(switch.indicator.next_setting()).poll(cx)
        {
            if let Some(r) = reply {
                r.ok(val);
            }
            return Poll::Ready((id, rtype, None));
        }

        // No settings. Return `Pending`.

        Poll::Pending
    }

    pub fn next_setting(
        &mut self,
    ) -> impl Future<Output = (Arc<str>, &'static str, Option<payload::LightCommand>)>
    {
        use std::future::poll_fn;

        poll_fn(move |mut cx| {
            for (id, dev) in self.map.iter_mut() {
                let rtype = match dev {
                    DeviceSet::Group(_) => "grouped_light",
                    _ => "light",
                };
                let id = id.clone();

                let res = match dev {
                    DeviceSet::Switch(switch) => {
                        Set::process_switch(id, rtype, switch, &mut cx)
                    }

                    DeviceSet::Bulb(dimmer) => {
                        Set::process_bulb(id, rtype, dimmer, &mut cx)
                    }

                    DeviceSet::ColorBulb(cb) | DeviceSet::Group(cb) => {
                        Set::process_colorbulb(id, rtype, cb, &mut cx)
                    }
                };

                if let Poll::Ready(_) = res {
                    return res;
                }
            }
            Poll::Pending
        })
    }
}

impl<R: Reporter> Registrator<R> for Set<R> {
    type Config = config::Params;

    async fn register_devices(
        drc: &mut drmem_api::driver::RequestChan<R>,
        _subpath: Option<&Path>,
        cfg: &Self::Config,
        max_history: Option<usize>,
    ) -> Result<Self> {
        let mut map = HashMap::new();

        for dcfg in &cfg.devices {
            match Set::from_devcfg(drc, dcfg, max_history).await {
                Ok((k, v)) => {
                    let _ = map.insert(k, v);
                }
                Err(e) => return Err(e),
            }
        }
        Ok(Set { map })
    }
}

impl<R: Reporter> ResettableState for Set<R> {
    fn reset_state(&mut self) {
        for (_, entry) in self.map.iter_mut() {
            match entry {
                DeviceSet::Switch(dev) => {
                    dev.state.reset_state();
                    dev.indicator.reset_state();
                }
                DeviceSet::Bulb(dev) => {
                    dev.brightness.reset_state();
                    dev.indicator.reset_state();
                }
                DeviceSet::ColorBulb(dev) => {
                    dev.brightness.reset_state();
                    dev.color.reset_state();
                }
                DeviceSet::Group(dev) => {
                    dev.brightness.reset_state();
                    dev.color.reset_state();
                }
            }
        }
    }
}

/// Defines a settable device that is "shared" with the outside world.
///
/// Some devices are purely controlled by DrMem. However, there are many
/// commercial devices that are intended to be controlled by users outside of
/// DrMem. LED WiFi light bulbs are one, obvious example. For devices that can
/// be controlled outside of DrMem, we need a way to cooperatively control
/// them. That's what `OverridableDevice`s do.
///
/// A driver that uses this type of device must do these steps in
/// their main loop:
///
/// - periodically poll the hardware and report the value using
///   `.report_update()`
/// - call `.next_setting()` to get the next incoming setting
/// - after setting the hardware to a new value, a poll should
///   immediately be done followed by a `.report_update()`
///
/// `OverridableDevice`s implement a simple state machine to know how to handle
/// incoming settings. Some hardware (e.g. a Philips Hue bridge) can optimistic-
/// ally confirm a setting before the physical device actually has it applied,
/// and later revert to the stale, real reading if the device never became
/// reachable. To tolerate this, a device can be given an "envelope": a settle
/// period, started when a setting is applied, during which polled readings are
/// compared against the setting but don't yet commit to `Synced` or
/// `Overridden`. Mismatches during the envelope reassert the setting and extend
/// the envelope; when it finally elapses, the last polled reading decides the
/// outcome.
use crate::{
    device,
    driver::{rw_device, Reporter, RxDeviceSetting, SettingResponder},
};
use tokio_stream::StreamExt;
use tracing::{debug, info, instrument, warn};

pub type SettingTransaction<T> = (T, Option<SettingResponder<T>>);

/// Bundles the two independent timeouts with which an `OverridableDevice` can
/// be configured.
#[derive(Clone, Copy, Debug, Default)]
pub struct OverrideConfig {
    /// How long the device stays `Overridden` before DrMem re-asserts its last
    /// setting. `None` means DrMem never re-asserts on its own.
    pub override_duration: Option<tokio::time::Duration>,
    /// How long to wait, after applying a setting, before trusting a polled
    /// reading enough to leave the "applying" state. `None` disables the
    /// envelope, matching a polled reading immediately.
    pub envelope: Option<tokio::time::Duration>,
}

// Describes the states that the device goes through as it receives
// settings and polled readings.

enum State<T: device::ReadWriteCompat> {
    Unknown,
    UnknownTrans {
        value: T,
        report: Option<SettingResponder<T>>,
    },
    Synced {
        value: T,
    },
    SyncedTrans {
        value: T,
    },

    // A setting is being applied to the hardware. `last_seen` is the most
    // recent polled reading observed since `setting` was (re)issued; it starts
    // out equal to `setting` (optimistic) until a `report_update()` says
    // otherwise. `deadline` only matters when an envelope is configured; it's
    // when we stop waiting for corroborating readings and commit to an outcome.
    // `needs_reaffirm` is set when a client redundantly re-sends the value
    // we're already applying; it tells `.next_setting()` to re-report that
    // value to the backend without bothering the driver again.
    Applying {
        setting: T,
        last_seen: T,
        deadline: tokio::time::Instant,
        needs_reaffirm: bool,
    },

    // Same as `Applying`, but there's a value that still needs to be handed to
    // the driver via `.next_setting()` -- either a brand new target
    // (`needs_report` is `true`, since the backend hasn't seen it yet) or a
    // reassertion of the current setting after a mismatched reading
    // (`needs_report` is `false`, since the backend already reflects this
    // target from when it was first applied).
    ApplyingTrans {
        setting: T,
        last_seen: T,
        deadline: tokio::time::Instant,
        to_send: SettingTransaction<T>,
        needs_report: bool,
    },

    Overridden {
        setting: T,
        r#override: T,
        deadline: tokio::time::Instant,
    },
}

impl<T: device::ReadWriteCompat> std::fmt::Debug for State<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            State::Unknown => write!(f, "<<Unknown>>"),
            State::UnknownTrans { .. } => {
                write!(f, "<<UnknownTrans>>")
            }
            State::Synced { .. } => write!(f, "<<Synced>>"),
            State::SyncedTrans { .. } => {
                write!(f, "<<SyncedTrans>>")
            }
            State::Applying { .. } => write!(f, "<<Applying>>",),
            State::ApplyingTrans { .. } => write!(f, "<<ApplyingTrans>>",),
            State::Overridden { .. } => write!(f, "<<Overridden>>",),
        }
    }
}

pub struct OverridableDevice<T: device::ReadWriteCompat, R: Reporter> {
    state: State<T>,
    override_duration: Option<tokio::time::Duration>,
    envelope: tokio::time::Duration,
    reporter: R,
    set_stream: rw_device::SettingStream<T>,
}

impl<T, R> OverridableDevice<T, R>
where
    T: device::ReadWriteCompat + std::fmt::Debug,
    R: Reporter,
{
    pub fn new(
        reporter: R,
        setting_chan: RxDeviceSetting,
        desired_value: Option<T>,
        override_duration: Option<tokio::time::Duration>,
        envelope: Option<tokio::time::Duration>,
    ) -> Self {
        let state = desired_value
            .map(|value| State::ApplyingTrans {
                setting: value.clone(),
                last_seen: value.clone(),
                deadline: tokio::time::Instant::now()
                    + envelope.unwrap_or_default(),
                to_send: (value, None),
                needs_report: true,
            })
            .unwrap_or(State::Unknown);

        debug!("initial state: {:?}", state);
        OverridableDevice {
            state,
            reporter,
            set_stream: rw_device::create_setting_stream(setting_chan),
            override_duration,
            envelope: envelope.unwrap_or_default(),
        }
    }

    /// Saves a new value, returned by the device, to the backend
    /// storage. This only writes values that have changed.
    ///
    /// This method is cancel-safe: every branch either performs no
    /// `.await` or mutates `self.state` only after its single
    /// `.await` resolves, so a cancelled call can be retried without
    /// losing or duplicating a report.
    #[inline(never)]
    #[instrument(skip(self), fields(state = ?self.state))]
    pub async fn report_update(&mut self, new_value: T) {
        debug!("waiting for the next report in state {:?}", self.state);

        let next_deadline = self
            .override_duration
            .map(|duration| tokio::time::Instant::now() + duration)
            .unwrap_or_else(tokio::time::Instant::now);
        let next_envelope = tokio::time::Instant::now() + self.envelope;

        match &mut self.state {
            State::Unknown => {
                self.reporter.report_value(new_value.clone().into()).await;

                // If we are in the unknown state and we get a polled
                // reading, we switch to the synced state. If a
                // setting comes it, it can further modify the state.

                debug!("entering Synced state with value {:?}", &new_value);
                self.state = State::Synced { value: new_value }
            }

            // If we're in this state, then we are in the process of
            // applying a setting from the Unknown state. We ignore
            // this spurious polled value so we can complete the
            // setting.
            //
            // This situation will probably never happen.
            State::UnknownTrans { .. } => {
                warn!("in UnknownTrans state ... ignoring reading");
            }

            State::Overridden {
                r#override: value,
                setting,
                deadline: _,
            } => {
                // The settings are currently overridden. If the value
                // is the same as the saved setting, we're back in
                // sync. If it's different than the last overridden
                // value, report it, refresh the absolute timer, and
                // save the new reading.

                if setting == &new_value {
                    self.reporter.report_value(new_value.clone().into()).await;
                    debug!("entering Synced state with value {:?}", &new_value);
                    self.state = State::Synced {
                        value: setting.clone(),
                    };
                } else if value != &new_value {
                    self.reporter.report_value(new_value.clone().into()).await;
                    debug!(
                        "reset Overridden state with new value {:?}",
                        &new_value
                    );
                    self.state = State::Overridden {
                        deadline: next_deadline,
                        setting: value.clone(),
                        r#override: new_value,
                    };
                }
            }

            State::Synced { value } => {
                // If the value is different from the previously
                // polled value, then we go into the overridden state.

                if value != &new_value {
                    self.reporter.report_value(new_value.clone().into()).await;
                    debug!("value differs from previous, transitioning to Overridden");
                    self.state = State::Overridden {
                        deadline: next_deadline,
                        setting: value.clone(),
                        r#override: new_value,
                    };
                }
            }

            // We're applying a setting and nothing is pending
            // delivery to the driver. A matching reading is
            // remembered but, if an envelope is configured, doesn't
            // commit to `Synced` until the envelope elapses (that
            // decision is made in `.next_setting()`). A mismatch
            // always reasserts the setting.
            State::Applying {
                setting, last_seen, ..
            } => {
                if setting == &new_value {
                    *last_seen = new_value;
                } else {
                    let setting = setting.clone();

                    warn!("reasserting setting, via ApplyingTrans");
                    self.state = State::ApplyingTrans {
                        setting: setting.clone(),
                        last_seen: new_value,
                        deadline: next_envelope,
                        to_send: (setting, None),
                        needs_report: false,
                    };
                }
            }

            // Same as `Applying`, but a value is still waiting to be
            // handed to the driver. A matching reading means that
            // value is no longer needed (the hardware is already
            // there), so the pending client is ack'd and we collapse
            // back to a settled state. A mismatch just extends the
            // envelope; the pending delivery is untouched.
            State::ApplyingTrans {
                setting,
                last_seen,
                deadline,
                to_send,
                ..
            } => {
                *last_seen = new_value.clone();

                if setting == &new_value {
                    let setting = setting.clone();
                    let old_deadline = *deadline;

                    if let Some(resp) = to_send.1.take() {
                        resp.ok(new_value);
                    }

                    info!("has envelope, transitioning to Applying");
                    self.state = State::Applying {
                        last_seen: setting.clone(),
                        setting,
                        deadline: old_deadline,
                        needs_reaffirm: false,
                    };
                } else {
                    *deadline = next_envelope;
                }
            }

            State::SyncedTrans { value } => {
                if value != &new_value {
                    self.reporter.report_value(new_value.clone().into()).await;
                    info!("value differs from previous, transitioning to Overridden");
                    self.state = State::Overridden {
                        deadline: next_deadline,
                        setting: value.clone(),
                        r#override: new_value,
                    };
                }
            }
        }
    }

    /// Gets the last value of the device. If DrMem is built with
    /// persistent storage, this value will be initialized with the
    /// last value saved to storage.
    #[inline(never)]
    pub fn get_last(&self) -> Option<&T> {
        match &self.state {
            State::Unknown => None,
            State::UnknownTrans { value, .. }
            | State::Synced { value }
            | State::SyncedTrans { value }
            | State::Applying { setting: value, .. }
            | State::ApplyingTrans { setting: value, .. } => Some(value),
            State::Overridden { r#override, .. } => Some(r#override),
        }
    }

    /// Waits for the next setting to arrive.
    ///
    /// This method is cancel-safe.
    #[inline(never)]
    #[instrument(skip(self), fields(state = ?self.state))]
    pub async fn next_setting(&mut self) -> Option<SettingTransaction<T>> {
        debug!("waiting for the next setting in state {:?}", self.state);
        let result = loop {
            match &mut self.state {
                // At this point, we have no known state. If a setting comes in,
                // we're going to assume it's different from the hardware's
                // state so we switch to Applying (via UnknownTrans).
                State::Unknown => {
                    let reply = self.set_stream.next().await?;

                    debug!(
                        "received new setting, transitioning to UnknownTrans"
                    );
                    self.state = State::UnknownTrans {
                        value: reply.0,
                        report: Some(reply.1),
                    };
                }

                // This is a transition state between Unknown and Applying.
                // This was needed to break up the Unknown state so that each
                // state has one future to await. This makes the function
                // "cancel safe".
                State::UnknownTrans { value, report } => {
                    self.reporter.report_value(value.clone().into()).await;

                    let value = value.clone();
                    let deadline = tokio::time::Instant::now() + self.envelope;
                    let report = report.take();

                    debug!("transitioning to Applying");
                    self.state = State::Applying {
                        setting: value.clone(),
                        last_seen: value.clone(),
                        deadline,
                        needs_reaffirm: false,
                    };
                    break Some((value, report));
                }

                State::Synced { value } => {
                    let reply = self.set_stream.next().await?;

                    self.state = if reply.0 != *value {
                        debug!("value differs from previous, transitioning to ApplyingTrans");
                        State::ApplyingTrans {
                            setting: reply.0.clone(),
                            last_seen: value.clone(),
                            deadline: tokio::time::Instant::now()
                                + self.envelope,
                            to_send: (reply.0, Some(reply.1)),
                            needs_report: true,
                        }
                    } else {
                        reply.1.ok(reply.0.clone());
                        debug!("value matches previous, transitioning to SyncedTrans");
                        State::SyncedTrans { value: reply.0 }
                    };
                }

                State::SyncedTrans { value } => {
                    self.reporter.report_value(value.clone().into()).await;
                    debug!("transitioning from SyncedTrans to Synced");
                    self.state = State::Synced {
                        value: value.clone(),
                    };
                }

                // A setting is being applied and there's nothing
                // pending delivery to the driver.
                State::Applying {
                    setting,
                    last_seen,
                    deadline,
                    needs_reaffirm,
                } => {
                    // The setting needs to be reasserted.
                    // Re-report it to the backend (without bothering the
                    // driver again) before doing anything else.
                    if *needs_reaffirm {
                        self.reporter
                            .report_value(setting.clone().into())
                            .await;
                        *needs_reaffirm = false;
                    }

                    let now = tokio::time::Instant::now();

                    if *deadline <= now {
                        if last_seen == setting {
                            debug!("deadline reached and last_seen matches setting, transitioning to Synced");
                            self.state = State::Synced {
                                value: setting.clone(),
                            };
                        } else {
                            // The backend never saw this reading while we were
                            // waiting out the envelope, so report it now or
                            // clients are stuck seeing the stale setting.
                            self.reporter
                                .report_value(last_seen.clone().into())
                                .await;
                            debug!("deadline reached and last_seen differs from setting, transitioning to Overridden");
                            self.state = State::Overridden {
                                setting: setting.clone(),
                                r#override: last_seen.clone(),
                                deadline: now
                                    + self
                                        .override_duration
                                        .unwrap_or_default(),
                            };
                        }
                        continue;
                    }

                    let delay = deadline.saturating_duration_since(now);

                    // Wait for a setting, or for the envelope to
                    // elapse so we can decide the outcome.

                    tokio::select! {
                        reply = self.set_stream.next() => {
                            match reply {
                                Some(r) => {
                                    if r.0 != *setting {
                                        debug!("new setting differs from current, transitioning to ApplyingTrans");
                                        self.state = State::ApplyingTrans {
                                            setting: r.0.clone(),
                                            last_seen: last_seen.clone(),
                                            deadline: tokio::time::Instant::now() + self.envelope,
                                            to_send: (r.0, Some(r.1)),
                                            needs_report: true,
                                        };
                                    } else {
                                        r.1.ok(r.0.clone());
                                        *needs_reaffirm = true;
                                    }
                                }
                                None => break None,
                            }
                        }
                        _ = tokio::time::sleep(delay) => {
                            if last_seen == setting {
                                debug!("TIMEOUT : last seen matches setting, transitioning to Synced");
                                self.state = State::Synced { value: setting.clone() };
                            } else {
                                // Same as above: report the real
                                // reading before committing to
                                // `Overridden`.
                                self.reporter.report_value(last_seen.clone().into()).await;
                                info!("TIMEOUT : last seen differs from setting, transitioning to Overridden");
                                self.state = State::Overridden {
                                    setting: setting.clone(),
                                    r#override: last_seen.clone(),
                                    deadline: tokio::time::Instant::now() + self.override_duration.unwrap_or_default(),
                                };
                            }
                        }
                    }
                }

                // Same as `Applying`, but a value still needs to be
                // handed to the driver -- either a brand new target
                // (reported here, since the backend hasn't seen it
                // yet) or a reassertion after a mismatch (never
                // reported; the backend already reflects this
                // target).
                State::ApplyingTrans {
                    setting,
                    last_seen,
                    deadline,
                    to_send,
                    needs_report,
                } => {
                    if *needs_report {
                        self.reporter
                            .report_value(to_send.0.clone().into())
                            .await;
                    }

                    let setting = setting.clone();
                    let last_seen = last_seen.clone();
                    let deadline = *deadline;
                    let result = (to_send.0.clone(), to_send.1.take());

                    debug!(
                        "transitioning to Applying with setting {:?}",
                        setting
                    );
                    self.state = State::Applying {
                        setting,
                        last_seen,
                        deadline,
                        needs_reaffirm: false,
                    };
                    break Some(result);
                }

                State::Overridden {
                    deadline,
                    setting,
                    r#override,
                } => {
                    // Being in the overridden state is a little more
                    // complicated. It has an optional timeout for when
                    // the override should switch back to the last
                    // setting.
                    if self.override_duration.is_some() {
                        let now = tokio::time::Instant::now();

                        if *deadline <= now {
                            self.state = if setting != r#override {
                                debug!("override deadline reached, transitioning to ApplyingTrans");
                                State::ApplyingTrans {
                                    setting: setting.clone(),
                                    last_seen: r#override.clone(),
                                    deadline: now + self.envelope,
                                    to_send: (setting.clone(), None),
                                    needs_report: true,
                                }
                            } else {
                                debug!("override deadline reached, transitioning to Synced");
                                State::Synced {
                                    value: r#override.clone(),
                                }
                            };
                            continue;
                        }

                        let delay = deadline.saturating_duration_since(now);

                        // Wait for a setting or for when the override
                        // timeout occurs.
                        #[rustfmt::skip]
                        tokio::select! {
                            reply = self.set_stream.next() => {
                                match reply {
                                    Some(r) => {
                                        // Save the new setting. It
                                        // doesn't get forwarded to
                                        // the driver because we don't
                                        // want the hardware state to
                                        // be changed. Instead it gets
                                        // stored.
                                        //
                                        // XXX: There is an issue here
                                        // in that, if a driver can
                                        // reject a setting's value,
                                        // we're not allowing that to
                                        // happen until when the
                                        // setting is applied later.
                                        // At that time, any error is
                                        // simply dropped.

                                        *setting = r.0.clone();
                                        r.1.ok(r.0);
                                    }
                                    None => break None
                                }
                            }
                            _ = tokio::time::sleep(delay) => {
                                // The timeout has occurred so we have
                                // to cancel the override. If the
                                // setting is the same as the
                                // override, we go into the `Synced`
                                // state. If they're different, then
                                // treat it as a new setting and
                                // reassert it (protected by a fresh
                                // envelope, same as any other new
                                // setting).

                                self.state = if setting != r#override {
                                    debug!("override deadline reached, transitioning to ApplyingTrans");
                                    State::ApplyingTrans {
                                        setting: setting.clone(),
                                        last_seen: r#override.clone(),
                                        deadline: tokio::time::Instant::now()
                                            + self.envelope,
                                        to_send: (setting.clone(), None),
                                        needs_report: true,
                                    }
                                } else {
                                    debug!("override deadline reached, transitioning to Synced");
                                    State::Synced {
                                        value: r#override.clone()
                                    }
                                }

                            }
                        }
                    } else {
                        let reply = self.set_stream.next().await?;

                        *setting = reply.0.clone();
                        reply.1.ok(reply.0)
                    }
                }
            }
        };
        result
    }
}

impl<T, R> super::ResettableState for OverridableDevice<T, R>
where
    T: device::ReadWriteCompat,
    R: Reporter,
{
    fn reset_state(&mut self) {
        self.state = State::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{device, driver::TxDeviceSetting};
    use noop_waker::noop_waker;
    use std::{
        future::Future,
        task::{Context, Poll},
    };
    use tokio::{
        sync::{mpsc, oneshot},
        time::{timeout, Duration},
    };

    struct MockReporter(mpsc::Sender<device::Value>);

    impl Reporter for MockReporter {
        async fn report_value(&mut self, v: device::Value) {
            self.0.send(v).await.unwrap()
        }
    }

    // Helper function that creates a `OverridableDevice`.

    fn mk_device<T: device::ReadWriteCompat + std::fmt::Debug>(
        init: Option<T>,
        tmo: Option<Duration>,
        envelope: Option<Duration>,
    ) -> (
        TxDeviceSetting,
        mpsc::Receiver<device::Value>,
        OverridableDevice<T, MockReporter>,
    ) {
        let (rrtx, rrrx) = mpsc::channel(20);
        let (srtx, srrx) = mpsc::channel(20);

        (
            srtx,
            rrrx,
            OverridableDevice::new(
                MockReporter(rrtx),
                srrx,
                init,
                tmo,
                envelope,
            ),
        )
    }

    #[tokio::test]
    async fn test_override_deadline_is_absolute() {
        let (tx_set, mut rx_rdg, mut dev) =
            mk_device::<bool>(Some(true), Some(Duration::from_secs(60)), None);
        let deadline = tokio::time::Instant::now() + Duration::from_millis(25);

        dev.state = State::Overridden {
            setting: true,
            r#override: false,
            deadline,
        };

        let start = tokio::time::Instant::now();
        assert!(matches!(
            timeout(Duration::from_millis(200), dev.next_setting()).await,
            Ok(Some((true, None)))
        ));
        assert!(matches!(
            timeout(Duration::from_secs(0), rx_rdg.recv()).await,
            Ok(Some(device::Value::Bool(true)))
        ));
        assert!(
            tokio::time::Instant::now().duration_since(start)
                >= Duration::from_millis(25)
        );

        std::mem::drop(tx_set);
    }

    #[tokio::test]
    async fn test_initialized_shared_device() {
        let (tx_set, mut rx_rdg, mut sh_dev) =
            mk_device::<i32>(Some(1), None, None);

        // A initialized shared device should assert the initial value.

        {
            // Nothing should be in the queue of messages going to the
            // backend.

            assert!(timeout(Duration::from_secs(0), rx_rdg.recv())
                .await
                .is_err());

            // Asking for the next setting should return the initial
            // value, but no acknowledgement function.

            assert!(matches!(
                timeout(Duration::from_secs(0), sh_dev.next_setting()).await,
                Ok(Some((1, None)))
            ));

            // The value should also go to the backend so it looks
            // like a setting.

            assert!(matches!(
                timeout(Duration::from_secs(0), rx_rdg.recv()).await,
                Ok(Some(device::Value::Int(1)))
            ));
        }

        // Set the reading as 1 to close out the setting transaction.

        {
            // Simulate that polling the hardware returned the setting
            // that we want.

            assert!(matches!(sh_dev.report_update(1).await, ()));

            // The hardware state matches the software state. But no
            // new messages should go to the backend since we already
            // reported the value.

            assert!(timeout(Duration::from_secs(0), rx_rdg.recv())
                .await
                .is_err());

            // Nothing new is coming in, so we should see a `Pending`
            // (since we're in an async function, we test for
            // `Pending` by setting a 0 duration timeout.)

            assert!(timeout(Duration::from_secs(0), sh_dev.next_setting())
                .await
                .is_err());

            // And, still, nothing should have been reported.

            assert!(timeout(Duration::from_secs(0), rx_rdg.recv())
                .await
                .is_err());
        }

        // Set the "polled" reading to 2, which puts us in override
        // mode.

        {
            assert!(matches!(sh_dev.report_update(2).await, ()));

            // Since we're in override mode, the value needs to be
            // automatically reported.

            assert!(matches!(
                timeout(Duration::from_secs(0), rx_rdg.recv()).await,
                Ok(Some(device::Value::Int(2)))
            ));

            // Looking for a setting should result in Pending.

            assert!(timeout(Duration::from_secs(0), sh_dev.next_setting())
                .await
                .is_err());

            // Nothing further should have been reported.

            assert!(timeout(Duration::from_secs(0), rx_rdg.recv())
                .await
                .is_err());
        }

        // Send a setting of 1. Since we're in override mode, it is
        // simply saved until we leave override mode.

        {
            let (os_tx, mut os_rx) = oneshot::channel();

            assert!(matches!(tx_set.send((1.into(), os_tx)).await, Ok(())));

            // Looking for a setting should result in Pending.

            assert!(timeout(Duration::from_secs(0), sh_dev.next_setting())
                .await
                .is_err());

            // Client should get a reply.

            assert_eq!(os_rx.try_recv(), Ok(Ok(device::Value::Int(1))));

            // Nothing further should have been reported.

            assert!(timeout(Duration::from_secs(0), rx_rdg.recv())
                .await
                .is_err());
        }

        // Now force a timeout to see if the new setting is reasserted.

        {
            // Force the override to expire immediately.
            sh_dev.override_duration = Some(Duration::ZERO);

            if let State::Overridden { .. } = &sh_dev.state {
                sh_dev.state = State::Overridden {
                    setting: 1,
                    r#override: 2,
                    deadline: tokio::time::Instant::now(),
                };
            } else {
                panic!(
                    "in wrong state: {:?}",
                    std::mem::discriminant(&sh_dev.state)
                );
            }

            // The previous setting (true) should be returned.

            assert!(matches!(
                timeout(Duration::from_secs(0), sh_dev.next_setting()).await,
                Ok(Some((1, None)))
            ));

            // The backend should receive the new setting, too.

            assert!(matches!(
                timeout(Duration::from_secs(0), rx_rdg.recv()).await,
                Ok(Some(device::Value::Int(1)))
            ));
        }

        std::mem::drop(tx_set)
    }

    #[test]
    fn test_uninitialized_shared_device() {
        let (tx_set, mut rx_rdg, mut sh_dev) =
            mk_device::<i32>(None, None, None);
        let waker = noop_waker();
        let mut context = Context::from_waker(&waker);

        {
            let fut = sh_dev.next_setting();

            tokio::pin!(fut);
            assert!(fut.poll(&mut context).is_pending());
        }

        // Send a setting of 1.

        {
            let (os_tx, mut os_rx) = oneshot::channel();

            assert!(matches!(tx_set.blocking_send((1.into(), os_tx)), Ok(_)));

            // `.next_setting()` should announce the new setting and
            // provide a function to send the reply.

            assert!(matches!(
                {
                    let fut = sh_dev.next_setting();

                    tokio::pin!(fut);
                    fut.poll(&mut context)
                },
                Poll::Ready(Some((1, Some(_))))
            ));

            // The client shouldn't get a reply from the call. It
            // *should* return an Empty error, but due to lifetimes in
            // these unit tests, it returns Closed. Both errors
            // indicate the function didn't send a reply.

            assert!(os_rx.try_recv().is_err());

            // Since we have a new setting, it should have been
            // reported to the backend.

            assert_eq!(rx_rdg.try_recv(), Ok(device::Value::Int(1)));
        }

        // Send another setting of 1. The client will get a reply, but
        // the function should return `Pending`.

        {
            let (os_tx, mut os_rx) = oneshot::channel();

            assert!(matches!(tx_set.blocking_send((1.into(), os_tx)), Ok(_)));

            // Client won't see another setting.

            assert!(matches!(
                {
                    let fut = sh_dev.next_setting();

                    tokio::pin!(fut);
                    fut.poll(&mut context)
                },
                Poll::Pending
            ));

            // Client sees a setting.

            assert_eq!(os_rx.try_recv(), Ok(Ok(device::Value::Int(1))));

            // The backend should see the setting attempt.

            assert_eq!(rx_rdg.try_recv(), Ok(device::Value::Int(1)));
        }

        // Send a setting of 2. The function will return the new
        // setting.

        {
            let (os_tx, mut os_rx) = oneshot::channel();

            assert!(matches!(tx_set.blocking_send((2.into(), os_tx)), Ok(_)));

            // Driver should see the setting and have a function to
            // reply to the client.

            assert!(matches!(
                {
                    let fut = sh_dev.next_setting();

                    tokio::pin!(fut);
                    fut.poll(&mut context)
                },
                Poll::Ready(Some((2, Some(_))))
            ));

            // Client shouldn't get a reply from the shared
            // device. It's up to the driver to do that.

            assert!(os_rx.try_recv().is_err());

            // Since we have a new setting, it should have been
            // reported.

            assert_eq!(rx_rdg.try_recv(), Ok(device::Value::Int(2)));
        }

        // Register the current value as 3. Since this isn't the
        // setting value, the next setting should be reported as 2.

        {
            {
                let fut = sh_dev.report_update(3);

                tokio::pin!(fut);
                assert!(matches!(fut.poll(&mut context), Poll::Ready(_)));
            }

            // Make sure `.report_update()` didn't report a new value
            // which didn't match the setting.

            assert_eq!(
                rx_rdg.try_recv(),
                Err(mpsc::error::TryRecvError::Empty)
            );

            assert!(matches!(
                {
                    let fut = sh_dev.next_setting();

                    tokio::pin!(fut);
                    fut.poll(&mut context)
                },
                Poll::Ready(Some((2, None)))
            ));

            // Even though we're re-reporting the setting, it
            // shouldn't be forwarded to the backend storage -- we
            // still need to sync the hardware with the setting.

            assert_eq!(
                rx_rdg.try_recv(),
                Err(mpsc::error::TryRecvError::Empty)
            );
        }

        // Register the current value as 2. Since this matches the
        // setting, this closes out the setting transaction.

        {
            {
                let fut = sh_dev.report_update(2);

                tokio::pin!(fut);
                assert!(matches!(fut.poll(&mut context), Poll::Ready(_)));
            }

            // The value was already reported.

            assert_eq!(
                rx_rdg.try_recv(),
                Err(mpsc::error::TryRecvError::Empty)
            );

            // Nothing new is coming in, so we should see a `Pending`.

            assert!(matches!(
                {
                    let fut = sh_dev.next_setting();

                    tokio::pin!(fut);
                    fut.poll(&mut context)
                },
                Poll::Pending
            ));

            // And nothing should have been reported.

            assert_eq!(
                rx_rdg.try_recv(),
                Err(mpsc::error::TryRecvError::Empty)
            );
        }

        // Now register the polled reading as 3. Since this is
        // different from the setting, we should go into override
        // mode.

        {
            {
                let fut = sh_dev.report_update(3);

                tokio::pin!(fut);
                assert!(matches!(fut.poll(&mut context), Poll::Ready(_)));
            }

            // Since we're in override mode, the value needs to be
            // automatically reported.

            assert_eq!(rx_rdg.try_recv(), Ok(device::Value::Int(3)));

            // Looking for a setting should result in Pending.

            assert!(matches!(
                {
                    let fut = sh_dev.next_setting();

                    tokio::pin!(fut);
                    fut.poll(&mut context)
                },
                Poll::Pending
            ));

            // Nothing further should have been reported.

            assert_eq!(
                rx_rdg.try_recv(),
                Err(mpsc::error::TryRecvError::Empty)
            );
        }

        // Now it reads the hardware as 2, which matches the current
        // setting. It should be reported but no new setting should
        // appear.

        {
            {
                let fut = sh_dev.report_update(2);

                tokio::pin!(fut);
                assert!(matches!(fut.poll(&mut context), Poll::Ready(_)));
            }

            assert_eq!(rx_rdg.try_recv(), Ok(device::Value::Int(2)));

            // Looking for a setting should result in a Pending.

            assert!(matches!(
                {
                    let fut = sh_dev.next_setting();

                    tokio::pin!(fut);
                    fut.poll(&mut context)
                },
                Poll::Pending
            ));

            assert_eq!(
                rx_rdg.try_recv(),
                Err(mpsc::error::TryRecvError::Empty)
            );
        }

        // Now a new setting (2). It matches the synced state so it
        // should get reported to the backend and the client should
        // get a reply. The driver should get a pending.

        {
            let (os_tx, mut os_rx) = oneshot::channel();

            assert!(matches!(tx_set.blocking_send((2.into(), os_tx)), Ok(_)));
            assert!(matches!(
                {
                    let fut = sh_dev.next_setting();

                    tokio::pin!(fut);
                    fut.poll(&mut context)
                },
                Poll::Pending
            ));

            // Client should get a success reply.

            assert!(matches!(os_rx.try_recv(), Ok(Ok(device::Value::Int(2)))));

            // Since we have a new setting, it should have been
            // reported.

            assert_eq!(rx_rdg.try_recv(), Ok(device::Value::Int(2)));
        }

        // Now a new setting (7) will get reported and returned, etc.

        {
            let (os_tx, mut os_rx) = oneshot::channel();

            assert!(matches!(tx_set.blocking_send((7.into(), os_tx)), Ok(_)));
            assert!(matches!(
                {
                    let fut = sh_dev.next_setting();

                    tokio::pin!(fut);
                    fut.poll(&mut context)
                },
                Poll::Ready(Some((7, Some(_))))
            ));

            // Client shouldn't get a reply from the shared
            // device. It's up to the driver to do that.

            assert!(os_rx.try_recv().is_err());

            // Since we have a new setting, it should have been
            // reported.

            assert_eq!(rx_rdg.try_recv(), Ok(device::Value::Int(7)));
        }
    }

    #[tokio::test]
    async fn test_timed_overrides() {
        let (tx_set, mut rx_rdg, mut sh_dev) =
            mk_device::<bool>(Some(true), Some(Duration::from_secs(60)), None);

        // Was created with an initial value, so that should be
        // presented as the first setting.

        {
            assert!(timeout(Duration::from_secs(0), rx_rdg.recv())
                .await
                .is_err());
            assert!(matches!(
                timeout(Duration::from_secs(0), sh_dev.next_setting()).await,
                Ok(Some((true, None)))
            ));
            assert!(matches!(
                timeout(Duration::from_secs(0), rx_rdg.recv()).await,
                Ok(Some(device::Value::Bool(true)))
            ));
        }

        // Report the hardware as false. Since that's not the desired
        // setting, it should reassert the setting.

        {
            assert!(matches!(sh_dev.report_update(false).await, ()));

            // Make sure `.report_update()` didn't report a new value
            // which didn't match the setting.

            assert!(timeout(Duration::from_secs(0), rx_rdg.recv())
                .await
                .is_err());

            assert!(matches!(
                timeout(Duration::from_secs(0), sh_dev.next_setting()).await,
                Ok(Some((true, None)))
            ));

            // Even though we're re-reporting the setting, it
            // shouldn't be forwarded to the backend storage -- we
            // still need to sync the hardware with the setting.

            assert!(timeout(Duration::from_secs(0), rx_rdg.recv())
                .await
                .is_err());
        }

        // Now report the hardware as 'true', which will close-out the
        // setting "transaction".

        {
            assert!(matches!(sh_dev.report_update(true).await, ()));

            // The value was already reported.

            assert!(timeout(Duration::from_secs(0), rx_rdg.recv())
                .await
                .is_err());

            // Nothing new is coming in, so we should see a `Pending`.

            assert!(timeout(Duration::from_secs(0), sh_dev.next_setting())
                .await
                .is_err());

            // And nothing should have been reported.

            assert!(timeout(Duration::from_secs(0), rx_rdg.recv())
                .await
                .is_err());
        }

        // Now the reading is 'false' which should put us in override
        // mode.

        {
            assert!(matches!(sh_dev.report_update(false).await, ()));

            // Since we're in override mode, the value needs to be
            // automatically reported.

            assert!(matches!(
                timeout(Duration::from_secs(0), rx_rdg.recv()).await,
                Ok(Some(device::Value::Bool(false)))
            ));

            // Looking for a setting should result in Pending.

            assert!(timeout(Duration::from_secs(0), sh_dev.next_setting())
                .await
                .is_err());

            // Nothing further should have been reported.

            assert!(timeout(Duration::from_secs(0), rx_rdg.recv())
                .await
                .is_err());
        }

        // Now force a timeout to see if the setting is reasserted.

        {
            // Force the override to expire immediately.
            sh_dev.override_duration = Some(Duration::ZERO);

            if let State::Overridden { .. } = &sh_dev.state {
                sh_dev.state = State::Overridden {
                    setting: true,
                    r#override: false,
                    deadline: tokio::time::Instant::now(),
                };
            } else {
                panic!(
                    "in wrong state: {:?}",
                    std::mem::discriminant(&sh_dev.state)
                );
            }

            // The previous setting (true) should be returned.

            assert!(matches!(
                timeout(Duration::from_secs(0), sh_dev.next_setting()).await,
                Ok(Some((true, None)))
            ));

            // The backend should receive the new setting, too.

            assert!(matches!(
                timeout(Duration::from_secs(0), rx_rdg.recv()).await,
                Ok(Some(device::Value::Bool(true)))
            ));
        }

        std::mem::drop(tx_set)
    }

    #[tokio::test]
    async fn test_envelope_holds_after_immediate_match() {
        let (tx_set, mut rx_rdg, mut sh_dev) = mk_device::<bool>(
            Some(true),
            None,
            Some(Duration::from_millis(100)),
        );

        // The initial desired value is delivered to the driver and
        // reported to the backend, same as without an envelope.
        assert!(matches!(
            timeout(Duration::from_secs(0), sh_dev.next_setting()).await,
            Ok(Some((true, None)))
        ));
        assert!(matches!(
            timeout(Duration::from_secs(0), rx_rdg.recv()).await,
            Ok(Some(device::Value::Bool(true)))
        ));

        // A matching poll arrives right away. With the bridge-staleness
        // bug this feature fixes, this is exactly the situation that
        // must NOT be trusted immediately.
        sh_dev.report_update(true).await;

        // Nothing new should be reported, and the device must not have
        // committed to `Synced` yet.
        assert!(timeout(Duration::from_secs(0), rx_rdg.recv())
            .await
            .is_err());
        assert!(matches!(&sh_dev.state, State::Applying { .. }));

        // `.next_setting()` must stay pending -- the envelope is still
        // open and no client setting has arrived.
        assert!(timeout(Duration::from_millis(20), sh_dev.next_setting())
            .await
            .is_err());

        std::mem::drop(tx_set);
    }

    #[tokio::test]
    async fn test_envelope_reassert_on_mismatch() {
        let (tx_set, mut rx_rdg, mut sh_dev) = mk_device::<bool>(
            Some(true),
            None,
            Some(Duration::from_millis(200)),
        );

        assert!(matches!(
            timeout(Duration::from_secs(0), sh_dev.next_setting()).await,
            Ok(Some((true, None)))
        ));
        assert!(matches!(
            timeout(Duration::from_secs(0), rx_rdg.recv()).await,
            Ok(Some(device::Value::Bool(true)))
        ));

        // A mismatched poll arrives. It must be remembered, but never
        // pushed to the backend.
        sh_dev.report_update(false).await;
        assert!(timeout(Duration::from_secs(0), rx_rdg.recv())
            .await
            .is_err());

        // The reassert is handed straight back to the driver (no
        // responder, since it's not a fresh client request) with no
        // extra backend report.
        assert!(matches!(
            timeout(Duration::from_secs(0), sh_dev.next_setting()).await,
            Ok(Some((true, None)))
        ));
        assert!(timeout(Duration::from_secs(0), rx_rdg.recv())
            .await
            .is_err());

        std::mem::drop(tx_set);
    }

    #[tokio::test]
    async fn test_envelope_reassert_extends_deadline() {
        let (tx_set, mut rx_rdg, mut sh_dev) = mk_device::<bool>(
            Some(true),
            None,
            Some(Duration::from_millis(150)),
        );

        assert!(matches!(
            timeout(Duration::from_secs(0), sh_dev.next_setting()).await,
            Ok(Some((true, None)))
        ));
        assert!(matches!(
            timeout(Duration::from_secs(0), rx_rdg.recv()).await,
            Ok(Some(device::Value::Bool(true)))
        ));

        // Wait most of the way through the original envelope, then
        // cause a mismatch. The deadline must be pushed out by a
        // fresh envelope measured from now, not from the start.
        tokio::time::sleep(Duration::from_millis(100)).await;
        sh_dev.report_update(false).await;
        assert!(matches!(
            timeout(Duration::from_secs(0), sh_dev.next_setting()).await,
            Ok(Some((true, None)))
        ));

        // At the *original* deadline (~50ms from here) the device must
        // still be waiting -- proving the mismatch reset the timer.
        assert!(timeout(Duration::from_millis(60), sh_dev.next_setting())
            .await
            .is_err());

        // Let the hardware settle on the right value so the extended
        // envelope can resolve cleanly once it elapses.
        sh_dev.report_update(true).await;
        assert!(timeout(Duration::from_millis(200), sh_dev.next_setting())
            .await
            .is_err());
        assert!(matches!(&sh_dev.state, State::Synced { value: true }));

        std::mem::drop(tx_set);
    }

    #[tokio::test]
    async fn test_envelope_expiry_resolves_to_synced() {
        let (tx_set, _rx_rdg, mut sh_dev) =
            mk_device::<bool>(None, None, Some(Duration::from_secs(60)));

        sh_dev.state = State::Applying {
            setting: true,
            last_seen: true,
            deadline: tokio::time::Instant::now(),
            needs_reaffirm: false,
        };

        // Deadline already elapsed and the last reading matches, so
        // this must resolve to `Synced` (and then just wait for a
        // setting that never comes).
        assert!(timeout(Duration::from_millis(50), sh_dev.next_setting())
            .await
            .is_err());
        assert!(matches!(&sh_dev.state, State::Synced { value: true }));

        std::mem::drop(tx_set);
    }

    #[tokio::test]
    async fn test_envelope_expiry_resolves_to_overridden() {
        let (tx_set, _rx_rdg, mut sh_dev) = mk_device::<bool>(
            None,
            Some(Duration::from_secs(60)),
            Some(Duration::from_secs(60)),
        );

        sh_dev.state = State::Applying {
            setting: true,
            last_seen: false,
            deadline: tokio::time::Instant::now(),
            needs_reaffirm: false,
        };

        // Deadline already elapsed and the last reading still doesn't
        // match, so we must give up and call it an override.
        assert!(timeout(Duration::from_millis(50), sh_dev.next_setting())
            .await
            .is_err());
        assert!(matches!(
            &sh_dev.state,
            State::Overridden {
                setting: true,
                r#override: false,
                ..
            }
        ));

        std::mem::drop(tx_set);
    }

    #[tokio::test]
    async fn test_envelope_expiry_to_overridden_reports_value() {
        let (tx_set, mut rx_rdg, mut sh_dev) = mk_device::<bool>(
            None,
            Some(Duration::from_secs(60)),
            Some(Duration::from_secs(60)),
        );

        sh_dev.state = State::Applying {
            setting: true,
            last_seen: false,
            deadline: tokio::time::Instant::now(),
            needs_reaffirm: false,
        };

        // Deadline already elapsed and the last reading still doesn't
        // match, so we give up and call it an override. The real
        // (mismatched) value must be reported to the backend right
        // then -- otherwise clients are stuck seeing the stale
        // setting forever, since a later poll of the *same* value
        // never triggers `report_update()`'s own Overridden-mismatch
        // report.
        assert!(timeout(Duration::from_millis(50), sh_dev.next_setting())
            .await
            .is_err());
        assert!(matches!(
            &sh_dev.state,
            State::Overridden {
                setting: true,
                r#override: false,
                ..
            }
        ));
        assert!(matches!(
            timeout(Duration::from_secs(0), rx_rdg.recv()).await,
            Ok(Some(device::Value::Bool(false)))
        ));

        std::mem::drop(tx_set);
    }

    #[tokio::test]
    async fn test_envelope_expiry_via_sleep_reports_value() {
        let (tx_set, mut rx_rdg, mut sh_dev) = mk_device::<bool>(
            None,
            Some(Duration::from_secs(60)),
            Some(Duration::from_millis(30)),
        );

        sh_dev.state = State::Applying {
            setting: true,
            last_seen: false,
            deadline: tokio::time::Instant::now() + Duration::from_millis(30),
            needs_reaffirm: false,
        };

        // Same as above, but let the envelope elapse naturally through
        // the `tokio::select!` sleep arm instead of the "already
        // expired" fast path.
        assert!(timeout(Duration::from_millis(200), sh_dev.next_setting())
            .await
            .is_err());
        assert!(matches!(
            &sh_dev.state,
            State::Overridden {
                setting: true,
                r#override: false,
                ..
            }
        ));
        assert!(matches!(
            timeout(Duration::from_secs(0), rx_rdg.recv()).await,
            Ok(Some(device::Value::Bool(false)))
        ));

        std::mem::drop(tx_set);
    }

    #[tokio::test]
    async fn test_envelope_multiple_reasserts_then_settles() {
        let (tx_set, mut rx_rdg, mut sh_dev) = mk_device::<bool>(
            Some(true),
            None,
            Some(Duration::from_millis(80)),
        );

        assert!(matches!(
            timeout(Duration::from_secs(0), sh_dev.next_setting()).await,
            Ok(Some((true, None)))
        ));
        assert!(matches!(
            timeout(Duration::from_secs(0), rx_rdg.recv()).await,
            Ok(Some(device::Value::Bool(true)))
        ));

        // Several mismatch/reassert cycles in a row. Each one extends
        // the envelope and none of them touch the backend.
        for _ in 0..3 {
            sh_dev.report_update(false).await;
            assert!(matches!(
                timeout(Duration::from_secs(0), sh_dev.next_setting()).await,
                Ok(Some((true, None)))
            ));
            assert!(timeout(Duration::from_secs(0), rx_rdg.recv())
                .await
                .is_err());
        }

        // Finally the hardware settles on the right value. Once the
        // full (most recent) envelope elapses with no further
        // mismatch, we commit to `Synced`.
        sh_dev.report_update(true).await;
        assert!(timeout(Duration::from_millis(300), sh_dev.next_setting())
            .await
            .is_err());
        assert!(matches!(&sh_dev.state, State::Synced { value: true }));

        std::mem::drop(tx_set);
    }

    #[tokio::test]
    async fn test_new_setting_arrives_mid_envelope() {
        let (tx_set, mut rx_rdg, mut sh_dev) =
            mk_device::<i32>(Some(1), None, Some(Duration::from_millis(200)));

        assert!(matches!(
            timeout(Duration::from_secs(0), sh_dev.next_setting()).await,
            Ok(Some((1, None)))
        ));
        assert!(matches!(
            timeout(Duration::from_secs(0), rx_rdg.recv()).await,
            Ok(Some(device::Value::Int(1)))
        ));

        // A client sends a different setting (2) while we're still
        // waiting out the envelope for 1.
        let (os_tx, mut os_rx) = oneshot::channel();
        assert!(matches!(tx_set.send((2.into(), os_tx)).await, Ok(())));

        // The driver is handed the new target, along with the
        // responder -- it's the driver's job to ack the client once
        // the hardware is actually set.
        assert!(matches!(
            timeout(Duration::from_secs(0), sh_dev.next_setting()).await,
            Ok(Some((2, Some(_))))
        ));
        assert!(os_rx.try_recv().is_err());
        assert!(matches!(
            timeout(Duration::from_secs(0), rx_rdg.recv()).await,
            Ok(Some(device::Value::Int(2)))
        ));
        assert!(matches!(sh_dev.get_last(), Some(2)));

        std::mem::drop(tx_set);
    }

    #[tokio::test]
    async fn test_duplicate_setting_mid_envelope_acks_immediately() {
        let (tx_set, mut rx_rdg, mut sh_dev) =
            mk_device::<i32>(Some(1), None, Some(Duration::from_millis(200)));

        assert!(matches!(
            timeout(Duration::from_secs(0), sh_dev.next_setting()).await,
            Ok(Some((1, None)))
        ));
        assert!(matches!(
            timeout(Duration::from_secs(0), rx_rdg.recv()).await,
            Ok(Some(device::Value::Int(1)))
        ));

        // A client re-sends the value we're already applying.
        let (os_tx, mut os_rx) = oneshot::channel();
        assert!(matches!(tx_set.send((1.into(), os_tx)).await, Ok(())));

        // It's ack'd immediately without bothering the driver again...
        assert!(timeout(Duration::from_secs(0), sh_dev.next_setting())
            .await
            .is_err());
        assert_eq!(os_rx.try_recv(), Ok(Ok(device::Value::Int(1))));

        // ...but it does get re-reported to the backend.
        assert!(matches!(
            timeout(Duration::from_secs(0), rx_rdg.recv()).await,
            Ok(Some(device::Value::Int(1)))
        ));

        std::mem::drop(tx_set);
    }

    #[tokio::test]
    async fn test_next_setting_cancel_safe_during_unknown_trans_report() {
        let (rrtx, mut rrrx) = mpsc::channel::<device::Value>(1);
        let (srtx, srrx) = mpsc::channel(20);

        // Pre-fill the backend channel so the pending report blocks.
        assert!(matches!(rrtx.send(device::Value::Int(0)).await, Ok(())));

        let mut sh_dev: OverridableDevice<i32, MockReporter> =
            OverridableDevice::new(MockReporter(rrtx), srrx, None, None, None);

        let (os_tx, mut os_rx) = oneshot::channel();
        assert!(matches!(srtx.send((1.into(), os_tx)).await, Ok(())));

        let waker = noop_waker();
        let mut cx = Context::from_waker(&waker);

        // First poll: Unknown -> UnknownTrans. The setting is consumed
        // from the stream, but no report has been attempted yet.
        {
            let fut = sh_dev.next_setting();
            tokio::pin!(fut);
            assert!(fut.poll(&mut cx).is_pending());
        }
        assert!(matches!(&sh_dev.state, State::UnknownTrans { .. }));

        // Second poll: tries to report, but the backend channel is
        // full, so this must return `Pending` without mutating state
        // any further. Dropping this future (end of the block)
        // simulates a cancellation.
        {
            let fut = sh_dev.next_setting();
            tokio::pin!(fut);
            assert!(fut.poll(&mut cx).is_pending());
        }
        assert!(matches!(&sh_dev.state, State::UnknownTrans { .. }));
        assert!(os_rx.try_recv().is_err());

        // Drain the dummy value so the retried report can proceed.
        assert_eq!(rrrx.try_recv(), Ok(device::Value::Int(0)));

        // Retry: the setting is handed to the driver and exactly one
        // (correct) value was reported -- nothing lost, nothing
        // duplicated.
        assert!(matches!(
            timeout(Duration::from_secs(0), sh_dev.next_setting()).await,
            Ok(Some((1, Some(_))))
        ));
        assert_eq!(rrrx.try_recv(), Ok(device::Value::Int(1)));
        assert_eq!(rrrx.try_recv(), Err(mpsc::error::TryRecvError::Empty));

        std::mem::drop(srtx);
    }

    #[tokio::test]
    async fn test_next_setting_cancel_safe_during_applying_trans_report() {
        let (rrtx, mut rrrx) = mpsc::channel::<device::Value>(1);
        let (srtx, srrx) = mpsc::channel(20);

        assert!(matches!(rrtx.send(device::Value::Int(0)).await, Ok(())));

        let mut sh_dev: OverridableDevice<i32, MockReporter> =
            OverridableDevice::new(MockReporter(rrtx), srrx, None, None, None);

        sh_dev.state = State::Synced { value: 1 };

        let (os_tx, mut os_rx) = oneshot::channel();
        assert!(matches!(srtx.send((2.into(), os_tx)).await, Ok(())));

        let waker = noop_waker();
        let mut cx = Context::from_waker(&waker);

        // First poll: Synced -> ApplyingTrans. The report is attempted
        // inside `ApplyingTrans`, so it hasn't happened yet.
        {
            let fut = sh_dev.next_setting();
            tokio::pin!(fut);
            assert!(fut.poll(&mut cx).is_pending());
        }
        assert!(matches!(&sh_dev.state, State::ApplyingTrans { .. }));

        // Second poll: the report is attempted, but the channel is
        // full. Dropping this future simulates a cancellation; state
        // must be untouched.
        {
            let fut = sh_dev.next_setting();
            tokio::pin!(fut);
            assert!(fut.poll(&mut cx).is_pending());
        }
        assert!(matches!(&sh_dev.state, State::ApplyingTrans { .. }));
        assert!(os_rx.try_recv().is_err());

        assert_eq!(rrrx.try_recv(), Ok(device::Value::Int(0)));

        assert!(matches!(
            timeout(Duration::from_secs(0), sh_dev.next_setting()).await,
            Ok(Some((2, Some(_))))
        ));
        assert_eq!(rrrx.try_recv(), Ok(device::Value::Int(2)));
        assert_eq!(rrrx.try_recv(), Err(mpsc::error::TryRecvError::Empty));

        std::mem::drop(srtx);
    }

    #[tokio::test]
    async fn test_next_setting_cancel_safe_during_envelope_select() {
        let (rrtx, _rrrx) = mpsc::channel::<device::Value>(20);
        let (srtx, srrx) = mpsc::channel(20);
        let mut sh_dev: OverridableDevice<bool, MockReporter> =
            OverridableDevice::new(
                MockReporter(rrtx),
                srrx,
                None,
                None,
                Some(Duration::from_millis(100)),
            );

        let deadline = tokio::time::Instant::now() + Duration::from_millis(100);

        sh_dev.state = State::Applying {
            setting: true,
            last_seen: true,
            deadline,
            needs_reaffirm: false,
        };

        // Nothing is ready yet (no incoming setting, envelope not
        // elapsed). Poll once and drop the future mid-`select!` to
        // simulate a cancellation.
        let waker = noop_waker();
        let mut cx = Context::from_waker(&waker);
        {
            let fut = sh_dev.next_setting();
            tokio::pin!(fut);
            assert!(fut.poll(&mut cx).is_pending());
        }

        // The deadline (and the rest of the state) must be untouched.
        assert!(matches!(
            &sh_dev.state,
            State::Applying { deadline: d, .. } if *d == deadline
        ));

        // Re-poll: it must still resolve against the *original*
        // deadline (proving the cancellation didn't reset the timer),
        // eventually committing to `Synced`.
        assert!(timeout(Duration::from_millis(300), sh_dev.next_setting())
            .await
            .is_err());
        assert!(matches!(&sh_dev.state, State::Synced { value: true }));

        std::mem::drop(srtx);
    }

    #[tokio::test]
    async fn test_report_update_cancel_safe() {
        let (rrtx, mut rrrx) = mpsc::channel::<device::Value>(1);
        let (_srtx, srrx) = mpsc::channel(20);

        assert!(matches!(rrtx.send(device::Value::Int(0)).await, Ok(())));

        let mut sh_dev: OverridableDevice<i32, MockReporter> =
            OverridableDevice::new(MockReporter(rrtx), srrx, None, None, None);

        sh_dev.state = State::Synced { value: 1 };

        let waker = noop_waker();
        let mut cx = Context::from_waker(&waker);

        // The channel is full, so the report inside `report_update()`
        // can't complete. Dropping this future simulates a
        // cancellation; state must stay untouched.
        {
            let fut = sh_dev.report_update(2);
            tokio::pin!(fut);
            assert!(fut.poll(&mut cx).is_pending());
        }
        assert!(matches!(&sh_dev.state, State::Synced { value: 1 }));

        // Drain the dummy and retry -- the end state must match an
        // uninterrupted call, with exactly one reported value.
        assert_eq!(rrrx.try_recv(), Ok(device::Value::Int(0)));
        sh_dev.report_update(2).await;

        assert!(matches!(
            &sh_dev.state,
            State::Overridden {
                setting: 1,
                r#override: 2,
                ..
            }
        ));
        assert_eq!(rrrx.try_recv(), Ok(device::Value::Int(2)));
        assert_eq!(rrrx.try_recv(), Err(mpsc::error::TryRecvError::Empty));
    }
}

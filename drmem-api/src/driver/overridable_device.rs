/// Defines a settable device that is "shared" with the outside world.
///
/// Some devices are purely controlled by DrMem. However, there are
/// many commercial devices that are intended to be controlled by
/// users outside of DrMem. LED WiFi light bulbs are one, obvious
/// example. For devices that can be controlled outside of DrMem, we
/// need a way to cooperatively control them. That's what
/// `OverridableDevice`s do.
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
/// `OverridableDevice`s implement a simple state machine to know how
/// to handle incoming incoming settings.
use crate::{
    device,
    driver::{rw_device, ReportReading, RxDeviceSetting, SettingResponder},
};
use tokio_stream::StreamExt;
use tracing::info;

pub type SettingTransaction<T> = (T, Option<SettingResponder<T>>);

// Describes the states that the device goes through as it receives
// settings and polled readings.

//#[derive(PartialEq)]
enum State<T: device::ReadWriteCompat> {
    Unknown,
    UnknownTrans {
        value: T,
        report: SettingResponder<T>,
    },
    Synced {
        value: T,
    },
    SyncedTrans {
        value: T,
    },
    UnreportedSetting {
        value: T,
    },
    Setting {
        value: T,
    },
    SettingTrans {
        value: (T, Option<SettingResponder<T>>),
    },
    ReassertSetting {
        value: T,
    },
    Overridden {
        setting: T,
        r#override: T,
        tmo: tokio::time::Instant,
    },
}

pub struct OverridableDevice<T: device::ReadWriteCompat> {
    state: State<T>,
    override_duration: Option<tokio::time::Duration>,
    report_chan: ReportReading,
    set_stream: rw_device::SettingStream<T>,
}

impl<T> OverridableDevice<T>
where
    T: device::ReadWriteCompat,
{
    pub fn new(
        report_chan: ReportReading,
        setting_chan: RxDeviceSetting,
        desired_value: Option<T>,
        override_duration: Option<tokio::time::Duration>,
    ) -> Self {
        OverridableDevice {
            state: desired_value
                .map(|value| State::SettingTrans {
                    value: (value, None),
                })
                .unwrap_or(State::Unknown),
            report_chan,
            set_stream: rw_device::create_setting_stream(setting_chan),
            override_duration,
        }
    }

    /// Saves a new value, returned by the device, to the backend
    /// storage. This only writes values that have changed.
    ///
    /// This method is not cancel-safe.
    pub async fn report_update(&mut self, new_value: T) {
        match &mut self.state {
            State::Unknown => {
                (self.report_chan)(new_value.clone().into()).await;

                // If we are in the unknown state and we get a polled
                // reading, we switch to the synced state. If a
                // setting comes it, it can further modify the state.

                self.state = State::Synced { value: new_value }
            }

            // If we're in this state, then we are in the process of
            // applying a setting from the Unknown state. We ignore
            // this spurious polled value so we can complete the
            // setting.
            //
            // This situation will probably never happen.
            State::UnknownTrans { .. } => {}

            State::Overridden {
                r#override: value,
                setting,
                ..
            } => {
                // The settings are currently overridden. If the value
                // is the same as the saved setting, we're back in
                // sync. If it's different than the last overridden
                // value, report it, reset the timer, and save the new
                // reading.

                if setting == &new_value {
                    (self.report_chan)(new_value.clone().into()).await;
                    self.state = State::Synced {
                        value: setting.clone(),
                    };
                    info!("value matches setting ... exiting override mode")
                } else if value != &new_value {
                    (self.report_chan)(new_value.clone().into()).await;
                    self.state = State::Overridden {
                        tmo: tokio::time::Instant::now(),
                        setting: value.clone(),
                        r#override: new_value,
                    };
                    info!("override timer reset")
                }
            }

            State::Synced { value } => {
                // If the value is different from the previously
                // polled value, then we go into the overridden state.

                if value != &new_value {
                    (self.report_chan)(new_value.clone().into()).await;
                    self.state = State::Overridden {
                        tmo: tokio::time::Instant::now(),
                        setting: value.clone(),
                        r#override: new_value,
                    };
                    info!("device in override mode")
                }
            }

            State::Setting { value } => {
                // When we're handling a setting, we enter the
                // `Synced` state if the polled value matches. If not,
                // then we have to reassert the setting.

                self.state = if value == &new_value {
                    State::Synced { value: new_value }
                } else {
                    State::ReassertSetting {
                        value: value.clone(),
                    }
                }
            }

            State::ReassertSetting { value }
            | State::UnreportedSetting { value } => {
                // When we're handling a setting, we enter the
                // `Synced` state if the polled value matches.

                if value == &new_value {
                    self.state = State::Synced { value: new_value }
                }
            }

            State::SettingTrans {
                value: (value, resp_ref),
            } => {
                // If the polled value happens to equal the incoming
                // setting that's still being processed, we should go
                // into the `Synced` state.

                if value == &new_value {
                    let value = value.clone();

                    // If there was a function to reply to the client,
                    // we need to perform the reply.

                    if let Some(resp) = resp_ref.take() {
                        resp.ok(value.clone());
                    }

                    // Go to the `SyncedTrans` state, which will
                    // report the new value to the backend.

                    self.state = State::SyncedTrans { value };
                }
            }

            State::SyncedTrans { value } => {
                if value != &new_value {
                    // These two statements are the reason this method
                    // isn't cancel-safe. We could try to add an
                    // `OverriddenTrans` state, but then we have to
                    // figure out what to do when settings or new
                    // polled values arrive.
                    //
                    // The sad part is that this state and the
                    // previous `SettingTrans` state are probably
                    // never going to be active when a new value is
                    // reported with this method.

                    (self.report_chan)(value.clone().into()).await;
                    (self.report_chan)(new_value.clone().into()).await;
                    self.state = State::Overridden {
                        tmo: tokio::time::Instant::now(),
                        setting: value.clone(),
                        r#override: new_value,
                    }
                }
            }
        }
    }

    /// Gets the last value of the device. If DrMem is built with
    /// persistent storage, this value will be initialized with the
    /// last value saved to storage.
    pub fn get_last(&self) -> Option<&T> {
        match &self.state {
            State::Unknown => None,
            State::UnknownTrans { value, .. }
            | State::Synced { value }
            | State::SyncedTrans { value }
            | State::Setting { value }
            | State::SettingTrans { value: (value, _) }
            | State::ReassertSetting { value }
            | State::UnreportedSetting { value } => Some(value),
            State::Overridden { r#override, .. } => Some(r#override),
        }
    }

    /// Waits for the next setting to arrive.
    ///
    /// This method is cancel-safe.
    pub async fn next_setting(&mut self) -> Option<SettingTransaction<T>> {
        loop {
            match &mut self.state {
                State::Unknown =>
                // At this point, we have no known state. If a
                // setting comes in, we're going to assume it's
                // different from the hardware's state so we
                // switch to Setting.
                {
                    match self.set_stream.next().await {
                        Some(reply) => {
                            self.state = State::UnknownTrans {
                                value: reply.0.clone(),
                                report: reply.1,
                            };
                        }
                        None => return None,
                    }
                }

                // This is a transition state between Unknown and
                // Setting. This was needed to break up the Unknown
                // state so that each state has one future to
                // await. This makes the function "cancel safe".
                State::UnknownTrans { value, .. } => {
                    (self.report_chan)(value.clone().into()).await;

                    let value = value.clone();

                    if let State::UnknownTrans { value, report } =
                        std::mem::replace(
                            &mut self.state,
                            State::Setting {
                                value: value.clone(),
                            },
                        )
                    {
                        return Some((value, Some(report)));
                    } else {
                        unreachable!()
                    }
                }

                State::UnreportedSetting { value } =>
                // If we have an unreported setting, re-report it and
                // switch to the "reported" setting state.
                {
                    (self.report_chan)(value.clone().into()).await;
                    self.state = State::Setting {
                        value: value.clone(),
                    };
                }

                State::ReassertSetting { value } =>
                // We need to reassert the setting. Immediately return
                // it and switch to the "reported" setting state.
                {
                    let result = (value.clone(), None);

                    self.state = State::Setting {
                        value: value.clone(),
                    };
                    return Some(result);
                }

                State::Setting { value } => {
                    match self.set_stream.next().await {
                        Some(reply) => {
                            if reply.0 != *value {
                                self.state = State::SettingTrans {
                                    value: (reply.0, Some(reply.1)),
                                };
                            } else {
                                reply.1.ok(reply.0.clone());
                                self.state =
                                    State::UnreportedSetting { value: reply.0 };
                            }
                        }
                        None => return None,
                    }
                }

                State::Synced { value } => match self.set_stream.next().await {
                    Some(reply) => {
                        self.state = if reply.0 != *value {
                            State::SettingTrans {
                                value: (reply.0, Some(reply.1)),
                            }
                        } else {
                            reply.1.ok(reply.0.clone());
                            State::SyncedTrans { value: reply.0 }
                        };
                    }
                    None => return None,
                },

                State::SettingTrans { value: (val, _) } => {
                    (self.report_chan)(val.clone().into()).await;

                    let val = val.clone();

                    if let State::SettingTrans { value: reply } =
                        std::mem::replace(
                            &mut self.state,
                            State::Setting { value: val.clone() },
                        )
                    {
                        return Some(reply);
                    } else {
                        unreachable!()
                    }
                }

                State::SyncedTrans { value } => {
                    (self.report_chan)(value.clone().into()).await;
                    self.state = State::Synced {
                        value: value.clone(),
                    };
                }

                State::Overridden {
                    tmo,
                    setting,
                    r#override,
                } =>
                // Being in the overridden state is a little more
                // complicated. It has an optional timeout for when
                // the override should switch back to the last
                // setting.
                {
                    if let Some(duration) = self.override_duration {
                        let delay = duration
                            .checked_sub(tmo.elapsed())
                            .unwrap_or(tokio::time::Duration::new(0, 0));

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
                                    None => return None
                                }
                            }
                            _ = tokio::time::sleep(delay) => {
                                // The timeout has occurred so we have
                                // to cancel the override. If the
                                // setting is the same as the
                                // override, we go into the `Synced`
                                // state. If they're different, then
                                // treat it as a new setting and
                                // return it to the driver so the
                                // hardware can be adjusted.

                                self.state = if setting != r#override {
                                    info!("timer expired ... restoring setting");
                                    State::SettingTrans {
                                        value: (setting.clone(), None)
                                    }
                                } else {
                                    State::Synced {
                                        value: r#override.clone()
                                    }
                                }
                            }
                        }
                    } else {
                        match self.set_stream.next().await {
                            Some(reply) => {
                                *setting = reply.0.clone();
                                reply.1.ok(reply.0);
                            }
                            None => return None,
                        }
                    }
                }
            }
        }
    }
}

impl<T> super::ResettableState for OverridableDevice<T>
where
    T: device::ReadWriteCompat,
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

    // Helper function that creates a `OverridableDevice`.

    fn mk_device<T: device::ReadWriteCompat>(
        init: Option<T>,
        tmo: Option<Duration>,
    ) -> (
        TxDeviceSetting,
        mpsc::Receiver<device::Value>,
        OverridableDevice<T>,
    ) {
        let (rrtx, rrrx) = mpsc::channel(20);
        let (srtx, srrx) = mpsc::channel(20);

        (
            srtx,
            rrrx,
            OverridableDevice::new(
                Box::new(move |v| {
                    let rrtx = rrtx.clone();

                    Box::pin(async move { rrtx.send(v).await.unwrap() })
                }),
                srrx,
                init,
                tmo,
            ),
        )
    }

    #[tokio::test]
    async fn test_initialized_shared_device() {
        let (tx_set, mut rx_rdg, mut sh_dev) = mk_device::<i32>(Some(1), None);

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
            // Adjust the timeout so that we guarantee it times out
            // right away.

            if let State::Overridden { tmo, .. } = &sh_dev.state {
                sh_dev.override_duration = Some(tmo.elapsed())
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
        let (tx_set, mut rx_rdg, mut sh_dev) = mk_device::<i32>(None, None);
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
            mk_device::<bool>(Some(true), Some(Duration::from_secs(60)));

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
            // Adjust the timeout so that we guarantee it times out
            // right away.

            if let State::Overridden { tmo, .. } = &sh_dev.state {
                sh_dev.override_duration = Some(tmo.elapsed())
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
}

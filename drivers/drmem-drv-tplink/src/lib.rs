// A driver to manage devices that use the TP-Link protocol. This
// protocol sends and receives JSON data over a TCP connection. Some
// sample exchanges for the HS220 dimmer:
//
//  Get status:
//
//   Sent:      {"system":{"get_sysinfo":{}}}
//   Received:  ???
//
//  Turning it on/off:
//
//   Turn on:   {"system":{"set_relay_state":{"state":1}}}
//   Turn off:  {"system":{"set_relay_state":{"state":0}}}
//   Received:  {"system":{"set_relay_state":{"err_code":0}}}
//
//  Setting the brightness to 75%:
//
//   Sent:      {"smartlife.iot.dimmer":{"set_brightness":{"brightness":75}}}
//   Received:  {"smartlife.iot.dimmer":{"set_brightness":{"err_code":0}}}
//
//  Controlling LED indicator:
//
//   Turn on:   {"system":{"set_led_off":{"off":0}}}
//   Turn off:  {"system":{"set_led_off":{"off":1}}}
//   Received:  {"system":{"set_led_off":{"err_code":0}}}
//
//  Error reply (example):
//
//   Sent:      {"system":{"set_bright":{"bright":75}}}
//   Received:  {"system":{"set_bright":{"err_code":-2,"err_msg":"member not support"}}}

use drmem_api::{
    device,
    driver::{self, DriverConfig},
    Error, Result,
};
use futures::{Future, StreamExt};
use std::net::SocketAddrV4;
use std::sync::Arc;
use std::{convert::Infallible, pin::Pin};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    sync::{Mutex, MutexGuard},
    time,
};
use tracing::{debug, error, info, Span};

mod tplink_api;

pub struct Instance {
    reported_error: Option<bool>,
}

pub struct Devices {
    addr: SocketAddrV4,

    d_error: driver::ReportReading<bool>,
    d_brightness: driver::ReportReading<f64>,
    s_brightness: driver::SettingStream<f64>,
    d_led: driver::ReportReading<bool>,
    s_led: driver::SettingStream<bool>,
}

impl Instance {
    pub const NAME: &'static str = "tplink";

    pub const SUMMARY: &'static str = "monitors and controls TP-Link devices";

    pub const DESCRIPTION: &'static str = include_str!("../README.md");

    // Pull the hostname/port for the remote process from the
    // configuration.

    fn get_cfg_address(cfg: &DriverConfig) -> Result<SocketAddrV4> {
        match cfg.get("addr") {
            Some(toml::value::Value::String(addr)) => {
                if let Ok(addr) = addr.parse::<SocketAddrV4>() {
                    Ok(addr)
                } else {
                    Err(Error::BadConfig(String::from(
                        "'addr' not in hostname:port format",
                    )))
                }
            }
            Some(_) => Err(Error::BadConfig(String::from(
                "'addr' config parameter should be a string",
            ))),
            None => Err(Error::BadConfig(String::from(
                "missing 'addr' parameter in config",
            ))),
        }
    }

    // Performs an "RPC" call to the device; it sends the command and
    // returns the reply.

    async fn rpc(
        s: &mut TcpStream,
        cmd: tplink_api::Cmd,
    ) -> Result<tplink_api::Reply> {
        Instance::send_cmd(s, cmd).await?;
        Instance::read_reply(s).await
    }

    // Sets the relay state on or off, depending on the argument.

    async fn relay_state_rpc(s: &mut TcpStream, v: bool) -> Result<()> {
        use tplink_api::{active_cmd, ErrorStatus, Reply};

        match Instance::rpc(s, active_cmd(v as u8)).await? {
            Reply::System {
                set_relay_state: Some(ErrorStatus { err_code: 0, .. }),
                ..
            } => Ok(()),

            Reply::System {
                set_relay_state:
                    Some(ErrorStatus {
                        err_msg: Some(em), ..
                    }),
                ..
            } => Err(Error::ProtocolError(format!("{}", &em))),

            reply => Err(Error::ProtocolError(format!(
                "unexpected reply : {:?}",
                &reply
            ))),
        }
    }

    // Sets the LED state on or off, depending on the argument.

    async fn led_state_rpc(s: &mut TcpStream, v: bool) -> Result<()> {
        use tplink_api::{led_cmd, ErrorStatus, Reply};

        // Send the request and receive the reply. Use pattern
        // matching to determine the return value of the function.

        match Instance::rpc(s, led_cmd(v)).await? {
            Reply::System {
                set_led_off: Some(ErrorStatus { err_code: 0, .. }),
                ..
            } => Ok(()),

            Reply::System {
                set_led_off:
                    Some(ErrorStatus {
                        err_msg: Some(em), ..
                    }),
                ..
            } => Err(Error::ProtocolError(format!("{}", &em))),

            reply => Err(Error::ProtocolError(format!(
                "unexpected reply : {:?}",
                &reply
            ))),
        }
    }

    // Retrieves info.

    async fn info_rpc(s: &mut TcpStream) -> Result<(bool, u8)> {
        use tplink_api::{info_cmd, Reply};

        // Send the request and receive the reply. Use pattern
        // matching to determine the return value of the function.

        match Instance::rpc(s, info_cmd()).await? {
            Reply::System {
                get_sysinfo: Some(info),
                ..
            } => {
                let led = info.led_off.unwrap_or(1) == 0;

                match (info.relay_state.map(|v| v != 0), info.brightness) {
                    (None, None) => Ok((led, 0)),
                    (None, Some(br)) => Ok((led, br)),
                    (Some(false), _) => Ok((led, 0)),
                    (Some(true), br) => Ok((led, br.unwrap_or(100))),
                }
            }

            reply => Err(Error::ProtocolError(format!(
                "unexpected reply : {:?}",
                &reply
            ))),
        }
    }

    // Sets the brightness between 0 and 100, depending on the
    // argument.

    async fn brightness_rpc(s: &mut TcpStream, v: u8) -> Result<()> {
        use tplink_api::{brightness_cmd, ErrorStatus, Reply};

        match Instance::rpc(s, brightness_cmd(v)).await? {
            Reply::Dimmer {
                set_brightness: Some(ErrorStatus { err_code: 0, .. }),
                ..
            } => Ok(()),

            Reply::Dimmer {
                set_brightness:
                    Some(ErrorStatus {
                        err_msg: Some(em), ..
                    }),
                ..
            } => Err(Error::ProtocolError(format!("{}", &em))),

            reply => Err(Error::ProtocolError(format!(
                "unexpected reply : {:?}",
                &reply
            ))),
        }
    }

    // Sends commands to change the brightness. NOTE: This function
    // assumes `v` is in the range 0.0..=100.0.

    async fn set_brightness(s: &mut TcpStream, v: f64) -> Result<()> {
        // If the brightness is zero, we trun off the dimmer instead
        // of setting the brightness to 0.0. If it's greater than 0.0,
        // set the brightness and then turn on the dimmer.

        if v > 0.0 {
            Instance::brightness_rpc(s, v as u8).await?;
            Instance::relay_state_rpc(s, true).await
        } else {
            Instance::relay_state_rpc(s, false).await
        }
    }

    // Connects to the address. Sets a timeout of 1 second for the
    // connection.

    async fn connect(addr: &SocketAddrV4) -> Result<TcpStream> {
        let fut = time::timeout(
            time::Duration::from_secs(1),
            TcpStream::connect(addr),
        );

        if let Ok(Ok(s)) = fut.await {
            Ok(s)
        } else {
            Err(Error::MissingPeer("tplink device".into()))
        }
    }

    // Handles incoming settings for brightness.

    async fn handle_brightness_setting<'a>(
        s: &'a mut TcpStream,
        v: f64,
        reply: driver::SettingReply<f64>,
        report: &'a driver::ReportReading<f64>,
    ) -> bool {
        if !v.is_nan() {
            // Clip incoming settings to the range 0.0..=100.0. Handle
            // infinities, too.

            let v = match v {
                v if v == f64::INFINITY => 100.0,
                v if v == f64::NEG_INFINITY => 0.0,
                v if v < 0.0 => 0.0,
                v if v > 100.0 => 100.0,
                v => v,
            };

            // Always log incoming settings. Let the client know there
            // was a successful setting, and include the value that
            // was used.

            match Instance::set_brightness(s, v).await {
                Ok(()) => {
                    report(v).await;
                    reply(Ok(v));
                    true
                }
                Err(e) => {
                    error!("brightness setting failed : {}", &e);
                    reply(Err(e));
                    false
                }
            }
        } else {
            reply(Err(Error::InvArgument("device doesn't accept NaN".into())));
            false
        }
    }

    // Handles incoming settings for controlling the LED indicator.

    async fn handle_led_setting<'a>(
        s: &'a mut TcpStream,
        v: bool,
        reply: driver::SettingReply<bool>,
        report: &'a driver::ReportReading<bool>,
    ) -> bool {
        debug!("setting LED");
        match Instance::led_state_rpc(s, v).await {
            Ok(()) => {
                debug!("success!");
                report(v).await;
                reply(Ok(v));
                true
            }
            Err(e) => {
                error!("LED setting failed : {}", &e);
                reply(Err(e));
                false
            }
        }
    }

    // Checks to see if the current error state ('value') matches the
    // previosuly reported error state. If not, it saves the current
    // state and sends the updated value to the backend.

    async fn sync_error_state(
        &mut self,
        report: &driver::ReportReading<bool>,
        value: bool,
    ) {
        if self.reported_error != Some(value) {
            self.reported_error = Some(value);
            report(value).await;
        }
    }

    // Attempts to read a `tplink_api::Reply` type from the socket.
    //
    // NOTE: All replies are shorter than 1,000 bytes (on the wire.)
    // Since the device has to encrypt the packet, it'll all go out in
    // one 1500 byte fragment so we don't have to try to read until a
    // complete message is received; it SHOULD all get included in one
    // read.

    async fn read_reply(s: &mut TcpStream) -> Result<tplink_api::Reply> {
        if let Ok(sz) = s.read_u32().await {
            let mut buf = [0; 1000];
            let filled = &mut buf[0..sz as usize];

            if let Err(e) = s.read_exact(filled).await {
                error!("when reading reply : {}", &e);
                Err(Error::MissingPeer("tplink device".into()))
            } else {
                tplink_api::Reply::decode(filled).ok_or_else(|| {
                    error!("bad reply : {}", String::from_utf8_lossy(filled));
                    Error::ParseError("tplink device".into())
                })
            }
        } else {
            Err(Error::MissingPeer("tplink device".into()))
        }
    }

    // Attempts to send a command to the socket.

    async fn send_cmd(s: &mut TcpStream, cmd: tplink_api::Cmd) -> Result<()> {
        let buf = cmd.encode();

        s.write_u32(buf.len() as u32)
            .await
            .map_err(|_| Error::MissingPeer("tplink device".into()))?;
        s.write_all(&buf)
            .await
            .map(|_| ())
            .map_err(|_| Error::MissingPeer("tplink device".into()))
    }

    async fn main_loop<'a>(&mut self, devices: &mut MutexGuard<'_, Devices>) {
        // Create a 5-second interval timer which will be used to poll
        // the device to see if its state was changed by some outside
        // mechanism.

        let mut timer =
            tokio::time::interval(tokio::time::Duration::from_secs(5));
        let mut current_led = false;
        let mut current_brightness = -1.0f64;

        // First, connect to the device. We'll leave the TCP
        // connection open so we're ready for the next transaction.
        //
        // XXX: Will keeping the socket open prevent the phone app
        // from controlling the device?

        if let Ok(mut s) = Instance::connect(&devices.addr).await {
            // Main loop of the driver. This loop never ends.

            loop {
                self.sync_error_state(&devices.d_error, false).await;

                // Get mutable references to the setting channels.

                let Devices {
                    s_brightness: ref mut s_b,
                    s_led: ref mut s_l,
                    ..
                } = **devices;

                // Now wait for one of two events to occur.

                #[rustfmt::skip]
                tokio::select! {
                    // If the timer tick expires, it's time to get the
                    // latest state of the device. Since external apps
                    // can modify the device outside of DrMem's
                    // control, we have to periodically poll it to
                    // stay in sync.

                    _ = timer.tick() => {
			if let Ok((led, br)) = Instance::info_rpc(&mut s).await {
			    let br = br as f64;

			    // If the LED state has changed outside of
			    // the driver, update the local state.

			    if current_led != led {
				info!("updating LED state: {}", led);
				current_led = led;
				(devices.d_led)(led).await;
			    }

			    // If the brightness state has changed
			    // outside of the driver, update the local
			    // state.

			    if current_brightness != br {
				info!("updating brightness state: {}", br);
				current_brightness = br;
				(devices.d_brightness)(br).await;
			    }
			}
                    }

		    // Handle settings to the brightness device.

                    Some((v, reply)) = s_b.next() => {

			// If the settings matches the current state,
			// then don't actually control the hardware.

			if current_brightness != v {
			    if Instance::handle_brightness_setting(
				&mut s, v, reply, &devices.d_brightness
			    ).await {
				current_brightness = v;
			    }
			} else {

			    // Hardware wasn't updated, but we still
			    // need to log the setting and return a
			    // reply to the client.

			    (devices.d_brightness)(v).await;
			    reply(Ok(v))
			}
                    }

		    // Handle settings to the LED indicator device.

                    Some((v, reply)) = s_l.next() => {
			debug!("led setting -> {}", &v);

			// If the settings matches the current state,
			// then don't actually control the hardware.

			if current_led != v {
			    if Instance::handle_led_setting(
				&mut s, v, reply, &devices.d_led
			    ).await {
				current_led = v;
			    }
			} else {
			    debug!("led won't change");

			    // Hardware wasn't updated, but we still
			    // need to log the setting and return a
			    // reply to the client.

			    (devices.d_led)(v).await;
			    reply(Ok(v))
			}
                    }
                }
            }
        }
    }
}

impl driver::API for Instance {
    type DeviceSet = Devices;

    // Registers two devices, `error` and `brightness`.

    fn register_devices(
        core: driver::RequestChan,
        cfg: &DriverConfig,
        max_history: Option<usize>,
    ) -> Pin<Box<dyn Future<Output = Result<Self::DeviceSet>> + Send>> {
        let error_name = "error"
            .parse::<device::Base>()
            .expect("parsing 'error' should never fail");
        let brightness_name = "brightness"
            .parse::<device::Base>()
            .expect("parsing 'brightness' should never fail");
        let led_name = "led"
            .parse::<device::Base>()
            .expect("parsing 'led' should never fail");
        let addr = Instance::get_cfg_address(cfg);

        Box::pin(async move {
            // Validate the configuration.

            let addr = addr?;

            // Define the devices managed by this driver.

            let (d_error, _) =
                core.add_ro_device(error_name, None, max_history).await?;
            let (d_brightness, s_brightness, _) = core
                .add_rw_device(brightness_name, None, max_history)
                .await?;
            let (d_led, s_led, _) =
                core.add_rw_device(led_name, None, max_history).await?;

            Ok(Devices {
                addr,
                d_error,
                d_brightness,
                s_brightness,
                d_led,
                s_led,
            })
        })
    }

    // This driver doesn't store any data in its instance; it's all
    // stored in local variables in the `.run()` method.

    fn create_instance(
        _cfg: &DriverConfig,
    ) -> Pin<Box<dyn Future<Output = Result<Box<Self>>> + Send>> {
        Box::pin(async {
            Ok(Box::new(Instance {
                reported_error: None,
            }))
        })
    }

    // Main run loop for the driver.

    fn run<'a>(
        &'a mut self,
        devices: Arc<Mutex<Devices>>,
    ) -> Pin<Box<dyn Future<Output = Infallible> + Send + 'a>> {
        let fut = async move {
            // Lock the mutex for the life of the driver. There is no
            // other task that wants access to these device handles.
            // An Arc<Mutex<>> is the other way I know of passing a
            // mutable value to async tasks.

            let mut devices = devices.lock().await;

            // Record the devices's address in the "cfg" field of the
            // span.

            Span::current().record("cfg", devices.addr.to_string());

            loop {
                self.main_loop(&mut devices).await;
                self.sync_error_state(&devices.d_error, true).await;

                // Log the error and then sleep for 10 seconds.
                // Hopefully the device will be available then.

                tokio::time::sleep(tokio::time::Duration::from_secs(10)).await
            }
        };

        Box::pin(fut)
    }
}

#[cfg(test)]
mod test {}

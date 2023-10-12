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
use futures::{Future, FutureExt, StreamExt};
use std::net::SocketAddrV4;
use std::sync::Arc;
use std::{convert::Infallible, pin::Pin};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    sync::{Mutex, MutexGuard},
    time,
};
use tracing::{debug, error, warn, Span};

mod tplink_api;

const BUF_TOTAL: usize = 4_096;

pub struct Instance {
    addr: SocketAddrV4,
    reported_error: Option<bool>,
    buf: [u8; BUF_TOTAL],
}

pub struct Devices {
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
                    Err(Error::ConfigError(String::from(
                        "'addr' not in hostname:port format",
                    )))
                }
            }
            Some(_) => Err(Error::ConfigError(String::from(
                "'addr' config parameter should be a string",
            ))),
            None => Err(Error::ConfigError(String::from(
                "missing 'addr' parameter in config",
            ))),
        }
    }

    // Attempts to read a `tplink_api::Reply` type from the socket.
    // All replies have a 4-byte length header so we know how much
    // data to read.

    async fn read_reply<R>(&mut self, s: &mut R) -> Result<tplink_api::Reply>
    where
        R: AsyncReadExt + std::marker::Unpin,
    {
        if let Ok(sz) = s.read_u32().await {
            let sz = sz as usize;

            if sz <= BUF_TOTAL {
                let filled = &mut self.buf[0..sz];

                if let Err(e) = s.read_exact(filled).await {
                    Err(Error::MissingPeer(e.to_string()))
                } else {
                    tplink_api::Reply::decode(filled).ok_or_else(|| {
                        Error::ParseError(format!(
                            "bad reply : {}",
                            String::from_utf8_lossy(filled)
                        ))
                    })
                }
            } else {
                Err(Error::ParseError(format!(
                    "reply size ({sz}) is greater than {BUF_TOTAL}"
                )))
            }
        } else {
            Err(Error::MissingPeer("error reading header".into()))
        }
    }

    // Attempts to send a command to the socket.

    async fn send_cmd<S>(s: &mut S, cmd: tplink_api::Cmd) -> Result<()>
    where
        S: AsyncWriteExt + std::marker::Unpin,
    {
        const ERR_F: fn(std::io::Error) -> Error =
            |e| Error::MissingPeer(e.to_string());
        let out_buf = cmd.encode();

        #[rustfmt::skip]
	tokio::select! {
	    result = s.write_all(&out_buf[..]) => {
		match result {
		    Ok(_) => s.flush().await.map_err(ERR_F),
		    Err(e) => Err(ERR_F(e))
		}
	    }
	    _ = time::sleep(time::Duration::from_millis(500)) =>
		Err(Error::TimeoutError)
	}
    }

    // Performs an "RPC" call to the device; it sends the command and
    // returns the reply.

    async fn rpc<R, S>(
        &mut self,
        rx: &mut R,
        tx: &mut S,
        cmd: tplink_api::Cmd,
    ) -> Result<tplink_api::Reply>
    where
        R: AsyncReadExt + std::marker::Unpin,
        S: AsyncWriteExt + std::marker::Unpin,
    {
        Instance::send_cmd(tx, cmd)
            .then(|res| async {
                match res {
                    Ok(()) => {
                        #[rustfmt::skip]
			tokio::select! {
			    result = self.read_reply(rx) => result,
			    _ = time::sleep(time::Duration::from_millis(500)) =>
				Err(Error::TimeoutError)
			}
                    }
                    Err(e) => Err(e),
                }
            })
            .await
    }

    // Sets the relay state on or off, depending on the argument.

    async fn relay_state_rpc(
        &mut self,
        s: &mut TcpStream,
        v: bool,
    ) -> Result<()> {
        use tplink_api::{active_cmd, ErrorStatus, Reply};

        let (mut rx, mut tx) = s.split();

        match self.rpc(&mut rx, &mut tx, active_cmd(v as u8)).await? {
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
            } => Err(Error::ProtocolError(em)),

            reply => Err(Error::ProtocolError(format!(
                "unexpected reply : {:?}",
                &reply
            ))),
        }
    }

    // Sets the LED state on or off, depending on the argument.

    async fn led_state_rpc(
        &mut self,
        s: &mut TcpStream,
        v: bool,
    ) -> Result<()> {
        use tplink_api::{led_cmd, ErrorStatus, Reply};

        let (mut rx, mut tx) = s.split();

        // Send the request and receive the reply. Use pattern
        // matching to determine the return value of the function.

        match self.rpc(&mut rx, &mut tx, led_cmd(v)).await? {
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
            } => Err(Error::ProtocolError(em)),

            reply => Err(Error::ProtocolError(format!(
                "unexpected reply : {:?}",
                &reply
            ))),
        }
    }

    // Retrieves info.

    async fn info_rpc(&mut self, s: &mut TcpStream) -> Result<(bool, u8)> {
        use tplink_api::{info_cmd, Reply};

        let (mut rx, mut tx) = s.split();

        // Send the request and receive the reply. Use pattern
        // matching to determine the return value of the function.

        match self.rpc(&mut rx, &mut tx, info_cmd()).await? {
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

    async fn brightness_rpc(&mut self, s: &mut TcpStream, v: u8) -> Result<()> {
        use tplink_api::{brightness_cmd, ErrorStatus, Reply};

        let (mut rx, mut tx) = s.split();

        match self.rpc(&mut rx, &mut tx, brightness_cmd(v)).await? {
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
            } => Err(Error::ProtocolError(em)),

            reply => Err(Error::ProtocolError(format!(
                "unexpected reply : {:?}",
                &reply
            ))),
        }
    }

    // Sends commands to change the brightness. NOTE: This function
    // assumes `v` is in the range 0.0..=100.0.

    async fn set_brightness(
        &mut self,
        s: &mut TcpStream,
        v: f64,
    ) -> Result<()> {
        // If the brightness is zero, we trun off the dimmer instead
        // of setting the brightness to 0.0. If it's greater than 0.0,
        // set the brightness and then turn on the dimmer.

        if v > 0.0 {
            self.brightness_rpc(s, v as u8).await?;
            self.relay_state_rpc(s, true).await
        } else {
            self.relay_state_rpc(s, false).await
        }
    }

    // Connects to the address. Sets a timeout of 1 second for the
    // connection.

    async fn connect(addr: &SocketAddrV4) -> Result<TcpStream> {
        use tokio::net::TcpSocket;

        let fut = time::timeout(time::Duration::from_secs(1), async {
            match TcpSocket::new_v4() {
                Ok(s) => {
                    s.set_recv_buffer_size((BUF_TOTAL * 2) as u32)?;

                    let s = s.connect((*addr).into()).await?;

                    s.set_nodelay(false)?;
                    Ok(s)
                }
                Err(e) => Err(e),
            }
        });

        match fut.await {
            Ok(Ok(s)) => Ok(s),
            Ok(Err(e)) => Err(Error::MissingPeer(e.to_string())),
            Err(_) => Err(Error::MissingPeer("timeout".into())),
        }
    }

    // Handles incoming settings for brightness.

    async fn handle_brightness_setting<'a>(
        &mut self,
        s: &'a mut TcpStream,
        v: f64,
        reply: driver::SettingReply<f64>,
        report: &'a driver::ReportReading<f64>,
    ) -> Result<Option<f64>> {
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

            // Send an OK reply to the client with the updated value.

            reply(Ok(v));

            // Always log incoming settings. Let the client know there
            // was a successful setting, and include the value that
            // was used.

            match self.set_brightness(s, v).await {
                Ok(()) => {
                    report(v).await;
                    Ok(Some(v))
                }
                Err(e) => {
                    error!("setting brightness : {}", &e);
                    Err(e)
                }
            }
        } else {
            reply(Err(Error::InvArgument("device doesn't accept NaN".into())));
            Ok(None)
        }
    }

    // Handles incoming settings for controlling the LED indicator.

    async fn handle_led_setting<'a>(
        &mut self,
        s: &'a mut TcpStream,
        v: bool,
        reply: driver::SettingReply<bool>,
        report: &'a driver::ReportReading<bool>,
    ) -> Result<()> {
        reply(Ok(v));
        match self.led_state_rpc(s, v).await {
            Ok(()) => {
                report(v).await;
                Ok(())
            }
            Err(e) => {
                error!("setting LED : {}", &e);
                Err(e)
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

    async fn main_loop<'a>(
        &mut self,
        s: &mut TcpStream,
        devices: &mut MutexGuard<'_, Devices>,
    ) {
        // Create a 5-second interval timer which will be used to poll
        // the device to see if its state was changed by some outside
        // mechanism.

        let mut timer =
            tokio::time::interval(tokio::time::Duration::from_secs(5));
        let mut current_led = false;
        let mut current_brightness = -1.0f64;

        // Main loop of the driver. This loop never ends.

        'main: loop {
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
                // latest state of the device. Since external apps can
                // modify the device outside of DrMem's control, we
                // have to periodically poll it to stay in sync.

                _ = timer.tick() => {
		    if let Ok((led, br)) = self.info_rpc(s).await {
			let br = br as f64;

			// If the LED state has changed outside of the
			// driver, update the local state.

			if current_led != led {
			    debug!("external LED update: {}", led);
			    current_led = led;
			    (devices.d_led)(led).await;
			}

			// If the brightness state has changed outside
			// of the driver, update the local state.

			if current_brightness != br {
			    debug!("external brightness update: {}", br);
			    current_brightness = br;
			    (devices.d_brightness)(br).await;
			}
		    } else {
			break 'main
		    }
                }

		// Handle settings to the brightness device.

                Some((v, reply)) = s_b.next() => {

		    // If the settings matches the current state, then
		    // don't actually control the hardware.

		    if current_brightness != v {
			match self.handle_brightness_setting(
			    s, v, reply, &devices.d_brightness
			).await {
			    Ok(Some(v)) => current_brightness = v,
			    Ok(None) => (),
			    Err(_) => break 'main
			}
		    } else {
			debug!("don't need to apply brightness setting");

			// Hardware wasn't updated, but we still need
			// to log the setting and return a reply to
			// the client.

			(devices.d_brightness)(v).await;
			reply(Ok(v))
		    }
                }

		// Handle settings to the LED indicator device.

                Some((v, reply)) = s_l.next() => {
		    debug!("led setting -> {}", &v);

		    // If the settings matches the current state, then
		    // don't actually control the hardware.

		    if current_led != v {
			if self.handle_led_setting(
			    s, v, reply, &devices.d_led
			).await == Ok(()) {
			    current_led = v;
			} else {
			    break 'main
			}
		    } else {
			debug!("don't need to apply led setting");

			// Hardware wasn't updated, but we still need
			// to log the setting and return a reply to
			// the client.

			(devices.d_led)(v).await;
			reply(Ok(v))
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
        _cfg: &DriverConfig,
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

        Box::pin(async move {
            // Define the devices managed by this driver.

            let (d_error, _) =
                core.add_ro_device(error_name, None, max_history).await?;
            let (d_brightness, s_brightness, _) = core
                .add_rw_device(brightness_name, None, max_history)
                .await?;
            let (d_led, s_led, _) =
                core.add_rw_device(led_name, None, max_history).await?;

            Ok(Devices {
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
        cfg: &DriverConfig,
    ) -> Pin<Box<dyn Future<Output = Result<Box<Self>>> + Send>> {
        let cfg_addr = Instance::get_cfg_address(cfg);

        Box::pin(async {
            Ok(Box::new(Instance {
                addr: cfg_addr?,
                reported_error: None,
                buf: [0; BUF_TOTAL],
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
            // An Arc<Mutex<>> is the only way I know of sharing a
            // mutable value with async tasks.

            let mut devices = devices.lock().await;

            // Record the devices's address in the "cfg" field of the
            // span.

            Span::current().record("cfg", self.addr.to_string());

            loop {
                // First, connect to the device. We'll leave the TCP
                // connection open so we're ready for the next
                // transaction. Tests have shown that the HS220
                // handles multiple client connections.

                match Instance::connect(&self.addr).await {
                    Ok(mut s) => {
                        self.main_loop(&mut s, &mut devices).await;
                    }
                    Err(e) => {
                        warn!("couldn't connect : '{}'", e);
                    }
                }

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
mod test {
    use super::{tplink_api, Instance};
    use crate::BUF_TOTAL;
    use std::{
        io::Write,
        net::{Ipv4Addr, SocketAddrV4},
    };

    #[tokio::test]
    async fn test_read_reply() {
        // Make sure packets with less than 4 bytes causes an error.

        {
            let buf: &[u8] = &[0, 0, 0];
            let mut inst = Instance {
                addr: SocketAddrV4::new(Ipv4Addr::new(0, 0, 0, 0), 0),
                reported_error: None,
                buf: [0u8; BUF_TOTAL],
            };

            assert!(inst.read_reply(&mut &buf[0..=0]).await.is_err());
            assert!(inst.read_reply(&mut &buf[0..1]).await.is_err());
            assert!(inst.read_reply(&mut &buf[0..2]).await.is_err());
            assert!(inst.read_reply(&mut &buf[0..3]).await.is_err());
        }

        {
            const REPLY: &[u8] =
                b"{\"system\":{\"set_led_off\":{\"err_code\":0}}}";

            let mut buf = vec![0, 0, 0, REPLY.len() as u8];

            {
                let mut wr = tplink_api::CmdWriter::create(&mut buf);

                assert_eq!(wr.write(REPLY).unwrap(), REPLY.len());
            }

            assert!(buf.len() == 45);
            assert!(buf.as_slice().len() == 45);

            let mut inst = Instance {
                addr: SocketAddrV4::new(Ipv4Addr::new(0, 0, 0, 0), 0),
                reported_error: None,
                buf: [0u8; BUF_TOTAL],
            };

            assert!(inst.read_reply(&mut &buf[0..4]).await.is_err());
            assert!(inst.read_reply(&mut &buf[0..5]).await.is_err());
            assert!(inst.read_reply(&mut buf.as_slice()).await.is_ok());
        }
    }
}

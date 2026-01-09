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
    driver::{
        self, classes, DriverConfig, Registrator, RequestChan, ResettableState,
    },
    Error, Result,
};
use futures::{Future, FutureExt};
use std::convert::Infallible;
use std::net::SocketAddrV4;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    time::{self, Duration},
};
use tracing::{debug, error, warn, Span};

mod tplink;

const BUF_TOTAL: usize = 4_096;

struct DeviceState {
    indicator: Option<bool>,
    brightness: Option<f64>,
    relay: Option<bool>,
}

pub enum DevType {
    Switch(classes::Switch),
    Dimmer(classes::Dimmer),
}

impl ResettableState for DevType {
    fn reset_state(&mut self) {
        match self {
            DevType::Switch(dev) => {
                dev.state.reset_state();
                dev.indicator.reset_state();
            }
            DevType::Dimmer(dev) => {
                dev.brightness.reset_state();
                dev.indicator.reset_state();
            }
        }
    }
}

impl Registrator for DevType {
    fn register_devices<'a>(
        drc: &'a mut RequestChan,
        cfg: &'a DriverConfig,
        override_timeout: Option<Duration>,
        max_history: Option<usize>,
    ) -> impl Future<Output = Result<Self>> + Send + 'a {
        async move {
            match cfg.get("type") {
                Some(toml::value::Value::String(dtype)) => match dtype.as_str()
                {
                    "outlet" | "switch" => Ok(DevType::Switch(
                        classes::Switch::register_devices(
                            drc,
                            cfg,
                            override_timeout,
                            max_history,
                        )
                        .await?,
                    )),
                    "dimmer" => Ok(DevType::Dimmer(
                        classes::Dimmer::register_devices(
                            drc,
                            cfg,
                            override_timeout,
                            max_history,
                        )
                        .await?,
                    )),
                    _ => Err(Error::ConfigError(String::from(
                        "'type' must be \"dimmer\", \"outlet\", or \"switch\"",
                    ))),
                },
                Some(_) => Err(Error::ConfigError(String::from(
                    "'type' config parameter should be a string",
                ))),
                None => Err(Error::ConfigError(String::from(
                    "missing 'type' parameter in config",
                ))),
            }
        }
    }
}

pub struct Instance {
    addr: SocketAddrV4,
    reported_error: Option<bool>,
    buf: [u8; BUF_TOTAL],
    poll_timeout: Duration,
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

    // Attempts to read a `tplink::Reply` type from the socket.
    // All replies have a 4-byte length header so we know how much
    // data to read.

    async fn read_reply<R>(&mut self, s: &mut R) -> Result<tplink::Reply>
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
                    tplink::Reply::decode(filled).ok_or_else(|| {
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

    async fn send_cmd<S>(s: &mut S, cmd: tplink::Cmd) -> Result<()>
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
	    _ = time::sleep(Duration::from_millis(500)) =>
		Err(Error::TimeoutError)
	}
    }

    // Performs an "RPC" call to the device; it sends the command and
    // returns the reply.

    async fn rpc<R, S>(
        &mut self,
        rx: &mut R,
        tx: &mut S,
        cmd: tplink::Cmd,
    ) -> Result<tplink::Reply>
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
			    _ = time::sleep(Duration::from_millis(500)) =>
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
        use tplink::{Cmd, ErrorStatus, Reply};

        let (mut rx, mut tx) = s.split();

        match self
            .rpc(&mut rx, &mut tx, Cmd::mk_active_cmd(v as u8))
            .await?
        {
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
        use tplink::{Cmd, ErrorStatus, Reply};

        let (mut rx, mut tx) = s.split();

        // Send the request and receive the reply. Use pattern
        // matching to determine the return value of the function.

        match self.rpc(&mut rx, &mut tx, Cmd::mk_led_cmd(v)).await? {
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

    async fn info_rpc(&mut self, s: &mut TcpStream) -> Result<DeviceState> {
        use tplink::{Cmd, Reply};

        let (mut rx, mut tx) = s.split();

        // Send the request and receive the reply. Use pattern
        // matching to determine the return value of the function.

        match self.rpc(&mut rx, &mut tx, Cmd::mk_info_cmd()).await? {
            Reply::System {
                get_sysinfo: Some(info),
                ..
            } => {
                let indicator = info.led_off.unwrap_or(1) == 0;

                match (info.relay_state.map(|v| v != 0), info.brightness) {
                    (None, None) => Ok(DeviceState {
                        indicator: Some(indicator),
                        relay: None,
                        brightness: Some(0.0),
                    }),
                    (None, Some(_)) => Ok(DeviceState {
                        indicator: Some(indicator),
                        relay: None,
                        brightness: Some(0.0),
                    }),
                    (rs @ Some(false), _) => Ok(DeviceState {
                        indicator: Some(indicator),
                        relay: rs,
                        brightness: Some(0.0),
                    }),
                    (rs @ Some(true), br) => Ok(DeviceState {
                        indicator: Some(indicator),
                        relay: rs,
                        brightness: Some(br.unwrap_or(100).into()),
                    }),
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
        use tplink::{Cmd, ErrorStatus, Reply};

        let (mut rx, mut tx) = s.split();

        match self
            .rpc(&mut rx, &mut tx, Cmd::mk_brightness_cmd(v))
            .await?
        {
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
        // If the brightness is zero, we turn off the dimmer instead
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

        let fut = time::timeout(Duration::from_secs(1), async {
            match TcpSocket::new_v4() {
                Ok(s) => {
                    s.set_recv_buffer_size((BUF_TOTAL * 2) as u32)?;
                    s.set_nodelay(true)?;

                    Ok(s.connect((*addr).into()).await?)
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
        reply: Option<driver::SettingReply<f64>>,
    ) -> Result<()> {
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

            if let Some(reply) = reply {
                reply(Ok(v));
            }

            // Always log incoming settings. Let the client know there
            // was a successful setting, and include the value that
            // was used.

            self.set_brightness(s, v).await
        } else {
            if let Some(reply) = reply {
                reply(Err(Error::InvArgument(
                    "device doesn't accept NaN".into(),
                )));
            }
            Ok(())
        }
    }

    // Handles incoming settings for controlling the LED indicator.

    async fn handle_led_setting<'a>(
        &mut self,
        s: &'a mut TcpStream,
        v: bool,
        reply: Option<driver::SettingReply<bool>>,
    ) -> Result<()> {
        if let Some(reply) = reply {
            reply(Ok(v));
        }
        self.led_state_rpc(s, v).await
    }

    // Checks to see if the current error state ('value') matches the
    // previosuly reported error state. If not, it saves the current
    // state and sends the updated value to the backend.

    async fn sync_error_state(&mut self, dev: &mut DevType, value: bool) {
        if self.reported_error != Some(value) {
            self.reported_error = Some(value);
            match dev {
                DevType::Switch(classes::Switch { error, .. }) => {
                    error.report_update(value).await
                }
                DevType::Dimmer(classes::Dimmer { error, .. }) => {
                    error.report_update(value).await
                }
            }
        }
    }

    async fn manage_switch<'a>(
        &mut self,
        s: &'a mut TcpStream,
        dev: &mut classes::Switch,
    ) -> bool {
        // Get mutable references to the setting channels.

        let classes::Switch {
            state: ref mut d_r,
            indicator: ref mut d_i,
            ..
        } = dev;

        // Now wait for one of three events to occur.

        #[rustfmt::skip]
        tokio::select! {
            // If our poll timeout expires, we need to request the
            // current state of the hardware (because it could be
            // changed by external apps or a person using the manual
            // switch.)

            _ = tokio::time::sleep(self.poll_timeout) => {

                // Reset the next timeout to be 5 seconds.

                self.poll_timeout = Duration::from_secs(5);

                // Request the hardware state.

                match self.info_rpc(s).await {
                    Ok(info) => {
                        if let Some(indicator) = info.indicator {
                            d_i.report_update(indicator).await
                        }

                        if let Some(relay) = info.relay {
                            d_r.report_update(relay).await
                        }
                    }
                    Err(e) => {
                        error!("error getting state -- {e}");
                        return false
                    }
                }
            }

	    // Handle settings to the brightness device.

            Some((v, reply)) = d_r.next_setting() => {
                if let Some(reply) = reply {
                    reply(Ok(v));
                }
                if let Err(e) = self.relay_state_rpc(s, v).await {
                    error!("couldn't set relay state -- {e}");
		    return false
		};
                self.poll_timeout = Duration::from_secs(0);
            }

	    // Handle settings to the LED indicator device.

            Some((v, reply)) = d_i.next_setting() => {
		debug!("led setting -> {}", &v);
                if let Err(e) = self.handle_led_setting(s, v, reply).await {
                    error!("couldn't set indicator -- {e}");
		    return false
		}
                self.poll_timeout = Duration::from_secs(0);
            }
        }
        return true;
    }

    async fn manage_dimmer<'a>(
        &mut self,
        s: &'a mut TcpStream,
        dev: &mut classes::Dimmer,
    ) -> bool {
        // Get mutable references to the setting channels.

        let classes::Dimmer {
            brightness: ref mut d_b,
            indicator: ref mut d_i,
            ..
        } = dev;

        // Now wait for one of three events to occur.

        #[rustfmt::skip]
        tokio::select! {
            // If our poll timeout expires, we need to request the
            // current state of the hardware (because it could be
            // changed by external apps or a person using the manual
            // switch.)

            _ = tokio::time::sleep(self.poll_timeout) => {

                // Reset the next timeout to be 5 seconds.

                self.poll_timeout = Duration::from_secs(5);

                // Request the hardware state.

                match self.info_rpc(s).await {
                    Ok(info) => {
                        if let Some(indicator) = info.indicator {
                            d_i.report_update(indicator).await
                        }

                        if let Some(brightness) = info.brightness {
                            d_b.report_update(brightness).await
                        }
                    }
                    Err(e) => {
                        error!("error getting state -- {e}");
                        return false
                    }
                }
            }

	    // Handle settings to the brightness device.

            Some((v, reply)) = d_b.next_setting() => {
                if let Err(e) = self.handle_brightness_setting(s, v, reply).await {
                    error!("couldn't set brightness -- {e}");
		    return false
		};
                self.poll_timeout = Duration::from_secs(0);
            }

	    // Handle settings to the LED indicator device.

            Some((v, reply)) = d_i.next_setting() => {
		debug!("led setting -> {}", &v);
                if let Err(e) = self.handle_led_setting(s, v, reply).await {
                    error!("couldn't set indicator -- {e}");
		    return false
		}
                self.poll_timeout = Duration::from_secs(0);
            }
        }
        return true;
    }

    async fn main_loop(
        &mut self,
        s: &mut TcpStream,
        devices: &mut <Instance as driver::API>::HardwareType,
    ) {
        self.sync_error_state(&mut *devices, false).await;
        loop {
            if !match &mut *devices {
                DevType::Switch(dev) => self.manage_switch(s, dev).await,
                DevType::Dimmer(dev) => self.manage_dimmer(s, dev).await,
            } {
                break;
            }
        }
        self.sync_error_state(&mut *devices, true).await;
    }
}

impl driver::API for Instance {
    type HardwareType = DevType;

    // This driver doesn't store any data in its instance; it's all
    // stored in local variables in the `.run()` method.

    fn create_instance(
        cfg: &DriverConfig,
    ) -> impl Future<Output = Result<Box<Self>>> + Send {
        let cfg_addr = Instance::get_cfg_address(cfg);

        async {
            Ok(Box::new(Instance {
                addr: cfg_addr?,
                reported_error: None,
                buf: [0; BUF_TOTAL],
                poll_timeout: Duration::from_secs(0),
            }))
        }
    }

    // Main run loop for the driver.

    async fn run(&mut self, devices: &mut Self::HardwareType) -> Infallible {
        // Record the devices's address in the "cfg" field of the
        // span.

        Span::current().record("cfg", self.addr.to_string());

        loop {
            // First, connect to the device. We'll leave the TCP
            // connection open so we're ready for the next
            // transaction. Tests have shown that the HS220 handles
            // multiple client connections.

            match Instance::connect(&self.addr).await {
                Ok(mut s) => {
                    self.main_loop(&mut s, devices).await;
                }
                Err(e) => {
                    warn!("couldn't connect : '{}'", e);
                }
            }

            self.sync_error_state(devices, true).await;

            // Log the error and then sleep for 10 seconds. Hopefully
            // the device will be available then.

            tokio::time::sleep(Duration::from_secs(10)).await
        }
    }
}

#[cfg(test)]
mod test {
    use super::{tplink, Instance};
    use crate::BUF_TOTAL;
    use std::net::{Ipv4Addr, SocketAddrV4};
    use tokio::time::Duration;

    #[tokio::test]
    async fn test_read_reply() {
        // Make sure packets with less than 4 bytes causes an error.

        {
            let buf: &[u8] = &[0, 0, 0];
            let mut inst = Instance {
                addr: SocketAddrV4::new(Ipv4Addr::new(0, 0, 0, 0), 0),
                reported_error: None,
                buf: [0u8; BUF_TOTAL],
                poll_timeout: Duration::from_secs(5),
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

            buf.extend_from_slice(&REPLY[..]);
            tplink::crypt::encode(&mut buf[4..]);

            assert!(buf.len() == 45);
            assert!(buf.as_slice().len() == 45);

            let mut inst = Instance {
                addr: SocketAddrV4::new(Ipv4Addr::new(0, 0, 0, 0), 0),
                reported_error: None,
                buf: [0u8; BUF_TOTAL],
                poll_timeout: Duration::from_secs(5),
            };

            assert!(inst.read_reply(&mut &buf[0..4]).await.is_err());
            assert!(inst.read_reply(&mut &buf[0..5]).await.is_err());
            assert!(inst.read_reply(&mut buf.as_slice()).await.is_ok());
        }
    }
}

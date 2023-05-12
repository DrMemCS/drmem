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
    io::{self, AsyncReadExt},
    net::{
        tcp::{OwnedReadHalf, OwnedWriteHalf},
        TcpStream,
    },
    sync::Mutex,
    time,
};
use tracing::{debug, error, info, warn, Span};

pub struct Instance;

pub struct Devices {
    addr: SocketAddrV4,

    d_error: driver::ReportReading<bool>,
    d_brightness: driver::ReportReading<f64>,
    s_brightness: driver::SettingStream<f64>,
}

impl Instance {
    pub const NAME: &'static str = "tplink";

    pub const SUMMARY: &'static str = "monitors and controls TP-Link devices";

    pub const DESCRIPTION: &'static str = include_str!("../README.md");

    // Attempts to pull the hostname/port for the remote process.

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

    // This is the encryption/decryption algorithm. It's a simple, XOR
    // algorithm so running this function on the same buffer, over and
    // over, encrypts it, then decrypts it, then encrypts it, etc.

    fn crypt(buf: &mut [u8]) {
        let mut key = 171u8;

        for b in buf.iter_mut() {
            key = *b ^ key;
            *b = key;
        }
    }

    // Returns a buffer containing the encoded command to set the
    // brightness. NOTE: This function assumes `v` is in the range
    // 0.0 ..= 100.0.

    fn set_brightness_cmd(v: f64) -> Vec<Vec<u8>> {
        let mut cmds = if v > 0.0 {
            vec! [
		format!("{{\"smartlife.iot.dimmer\":{{\"set_brightness\":{{\"brightness\":{}}}}}}}", v as u8),
		format!("{{\"system\":{{\"set_relay_state\":{{\"state\":1}}}}}}")
	    ]
        } else {
            vec![format!(
                "{{\"system\":{{\"set_relay_state\":{{\"state\":0}}}}}}"
            )]
        };

        cmds.drain(..)
            .map(|s| {
                let mut tmp = s.into_bytes();

                Instance::crypt(&mut tmp[..]);
                tmp
            })
            .collect()
    }

    // Connects to the address. Sets a timeout of 1 second.

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
}

impl driver::API for Instance {
    type DeviceSet = Devices;

    // Registers two devices, `error` and `brightness`.

    fn register_devices(
        core: driver::RequestChan, cfg: &DriverConfig,
        max_history: Option<usize>,
    ) -> Pin<Box<dyn Future<Output = Result<Self::DeviceSet>> + Send>> {
        let error_name = "error"
            .parse::<device::Base>()
            .expect("parsing 'error' should never fail");
        let brightness_name = "brightness"
            .parse::<device::Base>()
            .expect("parsing 'brightness' should never fail");
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

            Ok(Devices {
                addr,
                d_error,
                d_brightness,
                s_brightness,
            })
        })
    }

    // This driver doesn't store any data in its instance; it's all
    // stored in local variables in the `.run()` method.

    fn create_instance(
        _cfg: &DriverConfig,
    ) -> Pin<Box<dyn Future<Output = Result<Box<Self>>> + Send>> {
        Box::pin(async { Ok(Box::new(Instance)) })
    }

    // Main run loop for the driver.

    fn run<'a>(
        &'a mut self, devices: Arc<Mutex<Devices>>,
    ) -> Pin<Box<dyn Future<Output = Infallible> + Send + 'a>> {
        let fut = async move {
            let mut err_state = None;
            let mut devices = devices.lock().await;
            let mut timer =
                tokio::time::interval(tokio::time::Duration::from_secs(5));

            // Record the devices's address in the "cfg" field of the
            // span.

            Span::current().record("cfg", devices.addr.to_string());

            // Main loop of the driver. This loop can only ever exit
            // via an Err() type.

            loop {
                // First, connect to the device. We'll leave the TCP
                // connection open so we're ready for the next
                // transaction.
                //
                // XXX: Will keeping the socket open prevent the phone
                // app from controlling the device?

                match Instance::connect(&devices.addr).await {
                    Ok(s) => {
                        // Update the state of the error device. Make
                        // sure we only report state changes.

                        if err_state != Some(false) {
                            err_state = Some(false);
                            (devices.d_error)(false).await;
                        }

                        // Now wait for one of two events to occur.

                        #[rustfmt::skip]
                        tokio::select! {
                            // If the timer tick expires, it's time to
                            // get the latest state of the device.
                            // Since external apps can modify the
                            // device outside of DrMem's control, we
                            // have to periodically poll it to stay in
                            // sync.

                            _ = timer.tick() => {
                            }

                            // Look for incoming settings. We don't
                            // accept NaN and we clip other values
                            // into 0.0 ..= 100.0 range.

                            Some((v, reply)) = devices.s_brightness.next() => {
				if !v.is_nan() {
                                    let v = match v {
					v if v == f64::INFINITY => 100.0,
					v if v == f64::NEG_INFINITY => 0.0,
					v if v < 0.0 => 0.0,
					v if v > 100.0 => 100.0,
					v => v
                                    };

				    // Always log incoming settings.
				    // Let the client know there was a
				    // successful setting, and include
				    // the value that was used.

                                    (devices.d_brightness)(v).await;
                                    reply(Ok(v))
				} else {
                                    reply(Err(Error::InvArgument(
					"device doesn't accept NaN".into()
				    )))
				}
                            }
                        }
                    }
                    Err(e) => {
                        // Update the state of the error device. Make
                        // sure we only report state changes.

                        if err_state != Some(true) {
                            err_state = Some(true);
                            (devices.d_error)(true).await;
                        }

                        // Log the error and then sleep for 10
                        // seconds. Hopefully the device will be
                        // available then.

                        error!("{}", e);
                        tokio::time::sleep(tokio::time::Duration::from_secs(10))
                            .await
                    }
                }
            }
        };

        Box::pin(fut)
    }
}

#[cfg(test)]
mod tests {}

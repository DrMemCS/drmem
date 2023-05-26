// A driver to manage devices that use the TP-Link protocol. This
// protocol sends and receives JSON data over a TCP connection. Some
// sample exchanges for the HS220 dimmer:
//
//  Turning it on:
//
//   Sent:      {"system":{"set_relay_state":{"state":1}}}
//   Received:  {"system":{"set_relay_state":{"err_code":0}}}
//
//  Setting the brightness to 75%:
//
//   Sent:      {"smartlife.iot.dimmer":{"set_brightness":{"brightness":75}}}
//   Received:  {"smartlife.iot.dimmer":{"set_brightness":{"err_code":0}}}
//
//  Turning it off:
//
//   Sent:      {"system":{"set_relay_state":{"state":0}}}
//   Received:  {"system":{"set_relay_state":{"err_code":0}}}
//
//  Error reply:
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
    io::{self, AsyncReadExt},
    net::{
        tcp::{OwnedReadHalf, OwnedWriteHalf},
        TcpStream,
    },
    sync::Mutex,
    time,
};
use tracing::{debug, error, info, warn, Span};

mod tplink_api {
    use serde::{Deserialize, Serialize, Serializer};

    #[derive(Serialize)]
    pub struct ActiveValue {
        #[serde(rename = "state")]
        pub value: u8,
    }

    #[derive(Serialize)]
    pub struct BrightnessValue {
        #[serde(rename = "brightness")]
        pub value: u8,
    }

    #[derive(Serialize)]
    pub enum Cmd {
        #[serde(rename = "system")]
        Active {
            #[serde(rename = "set_relay_state")]
            value: ActiveValue,
        },

        #[serde(rename = "smartlife.iot.dimmer")]
        Brightness {
            #[serde(rename = "set_brightness")]
            value: BrightnessValue,
        },
    }
}

type Cmds = Vec<tplink_api::Cmd>;

pub struct Instance {
    reported_error: Option<bool>,
}

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

    fn set_brightness_cmd(v: f64) -> Cmds {
        use tplink_api::{ActiveValue, BrightnessValue, Cmd};

        // If the brightness is zero, we trun off the dimmer instead
        // of setting the brightness to 0.0. If it's greater than 0.0,
        // set the brightness and then turn on the dimmer.

        if v > 0.0 {
            vec![
                Cmd::Brightness {
                    value: BrightnessValue { value: v as u8 },
                },
                Cmd::Active {
                    value: ActiveValue { value: 1 },
                },
            ]
        } else {
            vec![Cmd::Active {
                value: ActiveValue { value: 0 },
            }]
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

    async fn handle_brightness_setting(
        v: f64, reply: driver::SettingReply<f64>,
        report: &driver::ReportReading<f64>,
    ) {
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

            report(v).await;
            reply(Ok(v))
        } else {
            reply(Err(Error::InvArgument("device doesn't accept NaN".into())))
        }
    }

    // Checks to see if the current error state ('value') matches the
    // previosuly reported error state. If not, it saves the current
    // state and sends the updated value to the backend.

    async fn sync_error_state(
        &mut self, report: &driver::ReportReading<bool>, value: bool,
    ) {
        if self.reported_error != Some(value) {
            self.reported_error = Some(value);
            report(value).await;
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
        Box::pin(async {
            Ok(Box::new(Instance {
                reported_error: None,
            }))
        })
    }

    // Main run loop for the driver.

    fn run<'a>(
        &'a mut self, devices: Arc<Mutex<Devices>>,
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

            // Create a 5-second interval timer which will be used to
            // poll the device to see if its state was changed by some
            // outside mechanism.

            let mut timer =
                tokio::time::interval(tokio::time::Duration::from_secs(5));

            // Main loop of the driver. This loop never ends.

            loop {
                // First, connect to the device. We'll leave the TCP
                // connection open so we're ready for the next
                // transaction.
                //
                // XXX: Will keeping the socket open prevent the phone
                // app from controlling the device?

                match Instance::connect(&devices.addr).await {
                    Ok(s) => {
                        self.sync_error_state(&devices.d_error, false).await;

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

                            Some((v, reply)) = devices.s_brightness.next() => {
				Instance::handle_brightness_setting(
				    v, reply, &devices.d_brightness
				).await
                            }
                        }
                    }
                    Err(e) => {
                        self.sync_error_state(&devices.d_error, true).await;

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
mod tests {
    use super::tplink_api::{ActiveValue, BrightnessValue, Cmd};
    use serde_json;

    #[test]
    fn test_cmds() {
        assert_eq!(
            serde_json::to_string(&Cmd::Active {
                value: ActiveValue { value: 1 }
            })
            .unwrap(),
            "{\"system\":{\"set_relay_state\":{\"state\":1}}}"
        );
        assert_eq!(
            serde_json::to_string(&Cmd::Brightness {
                value: BrightnessValue { value: 0 }
            })
            .unwrap(),
            "{\"smartlife.iot.dimmer\":{\"set_brightness\":{\"brightness\":0}}}"
        );
        assert_eq!(
            serde_json::to_string(&Cmd::Brightness {
                value: BrightnessValue { value: 50 }
            })
            .unwrap(),
            "{\"smartlife.iot.dimmer\":{\"set_brightness\":{\"brightness\":50}}}"
        );
        assert_eq!(
            serde_json::to_string(&Cmd::Brightness {
                value: BrightnessValue { value: 100 }
            })
            .unwrap(),
            "{\"smartlife.iot.dimmer\":{\"set_brightness\":{\"brightness\":100}}}"
        );
    }
}

// This module defines the commands that can be sent to the TP-Link
// device. It also configures the `serde` crate so these commands are
// converted to the expected JSON layout.

use serde::{Deserialize, Serialize};
use std::marker::PhantomData;

// This is the encryption algorithm.

pub fn encrypt(buf: &mut [u8]) {
    let mut key = 171u8;

    for b in buf.iter_mut() {
        key ^= *b;
        *b = key;
    }
}

// This is the decryption algorithm.

fn decrypt(buf: &mut [u8]) {
    let mut key = 171u8;

    for b in buf.iter_mut() {
        let tmp = *b;

        *b ^= key;
        key = tmp;
    }
}

// Defines the internal value used by the `set_relay_state` command.
// Needs to convert to `{"state":value}`.

#[derive(Serialize, PartialEq, Debug)]
pub struct ActiveValue {
    pub state: u8,
}

// Defines the internal value used by the `set_led_off` command. Needs
// to convert to `{"off":value}`.

#[derive(Serialize, PartialEq, Debug)]
pub struct LedValue {
    pub off: u8,
}

// Defines the internal value used by the `get_sysinfo` command. Needs
// to convert to `{}`.

#[derive(Serialize, PartialEq, Debug)]
pub struct InfoValue {
    #[serde(skip)]
    pub nothing: PhantomData<()>,
}

// Defines the internal value used by the `Brightness` command. Needs
// to convert to `{"brightness":value}`.

#[derive(Serialize, PartialEq, Debug)]
pub struct BrightnessValue {
    pub brightness: u8,
}

#[derive(Serialize, PartialEq, Debug)]
pub enum Cmd {
    #[serde(rename = "system")]
    System {
        #[serde(skip_serializing_if = "Option::is_none")]
        set_relay_state: Option<ActiveValue>,
        #[serde(skip_serializing_if = "Option::is_none")]
        get_sysinfo: Option<InfoValue>,
        #[serde(skip_serializing_if = "Option::is_none")]
        set_led_off: Option<LedValue>,
    },

    #[serde(rename = "smartlife.iot.dimmer")]
    Dimmer { set_brightness: BrightnessValue },
}

impl Cmd {
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = serde_json::to_vec(&self).unwrap();

        encrypt(&mut buf);
        buf
    }
}

#[derive(Deserialize, PartialEq, Debug)]
pub struct ErrorStatus {
    pub err_code: i32,
    pub err_msg: Option<String>,
}

#[derive(Deserialize, PartialEq, Debug)]
pub struct InfoReply {
    pub sw_ver: String,
    pub hw_ver: String,
    pub model: String,
    #[serde(rename = "deviceId")]
    pub device_id: String,
    #[serde(rename = "oemId")]
    pub oem_id: String,
    #[serde(rename = "hwId")]
    pub hw_id: String,
    pub updating: Option<u8>,
    pub led_off: Option<u8>,
    pub relay_state: Option<u8>,
    pub brightness: Option<u8>,
    pub err_code: i32,
}

// This type models a subset of the replies that are returned by the
// device (only define the replies that come from commands we send.)

#[derive(Deserialize, PartialEq, Debug)]
pub enum Reply {
    #[serde(rename = "system")]
    System {
        set_relay_state: Option<ErrorStatus>,
        set_led_off: Option<ErrorStatus>,
        get_sysinfo: Option<InfoReply>,
    },

    #[serde(rename = "smartlife.iot.dimmer")]
    Dimmer { set_brightness: Option<ErrorStatus> },
}

impl Reply {
    pub fn decode(buf: &mut [u8]) -> Option<Reply> {
        decrypt(buf);
        serde_json::from_slice(buf).ok()
    }
}

pub fn active_cmd(v: u8) -> Cmd {
    Cmd::System {
        set_relay_state: Some(ActiveValue { state: v }),
        get_sysinfo: None,
        set_led_off: None,
    }
}

pub fn brightness_cmd(v: u8) -> Cmd {
    Cmd::Dimmer {
        set_brightness: BrightnessValue { brightness: v },
    }
}

pub fn info_cmd() -> Cmd {
    Cmd::System {
        set_relay_state: None,
        get_sysinfo: Some(InfoValue {
            nothing: PhantomData,
        }),
        set_led_off: None,
    }
}

pub fn led_cmd(v: bool) -> Cmd {
    Cmd::System {
        set_relay_state: None,
        get_sysinfo: None,
        set_led_off: Some(LedValue { off: (!v) as u8 }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json;

    #[test]
    fn test_crypt() {
        let buf = [1u8, 2u8, 3u8, 4u8, 5u8];
        let mut buf2 = buf.clone();

        encrypt(&mut buf2);
        decrypt(&mut buf2);
        assert_eq!(&buf, &buf2);
    }

    #[test]
    fn test_cmds() {
        assert_eq!(
            serde_json::to_string(&active_cmd(1)).unwrap(),
            "{\"system\":{\"set_relay_state\":{\"state\":1}}}"
        );
        assert_eq!(
            serde_json::to_string(&led_cmd(false)).unwrap(),
            "{\"system\":{\"set_led_off\":{\"off\":1}}}"
        );
        assert_eq!(
            serde_json::to_string(&led_cmd(true)).unwrap(),
            "{\"system\":{\"set_led_off\":{\"off\":0}}}"
        );
        assert_eq!(
            serde_json::to_string(&info_cmd()).unwrap(),
            "{\"system\":{\"get_sysinfo\":{}}}"
        );
        assert_eq!(
            serde_json::to_string(&brightness_cmd(0)).unwrap(),
            "{\"smartlife.iot.dimmer\":{\"set_brightness\":{\"brightness\":0}}}"
        );
        assert_eq!(
            serde_json::to_string(&brightness_cmd(50)).unwrap(),
            "{\"smartlife.iot.dimmer\":{\"set_brightness\":{\"brightness\":50}}}"
        );
        assert_eq!(
            serde_json::to_string(&brightness_cmd(100)).unwrap(),
            "{\"smartlife.iot.dimmer\":{\"set_brightness\":{\"brightness\":100}}}"
        );
    }

    #[test]
    fn test_replies() {
        assert!(serde_json::from_str::<Reply>("").is_err());

        assert_eq!(
            serde_json::from_str::<Reply>(
                r#"{"system":{"set_relay_state":{"err_code":0}}}"#
            )
            .unwrap(),
            Reply::System {
                set_relay_state: Some(ErrorStatus {
                    err_code: 0,
                    err_msg: None
                }),
                set_led_off: None,
                get_sysinfo: None
            }
        );
        assert_eq!(
            serde_json::from_str::<Reply>(
                r#"{"system":{"set_led_off":{"err_code":0}}}"#
            )
            .unwrap(),
            Reply::System {
                set_relay_state: None,
                set_led_off: Some(ErrorStatus {
                    err_code: 0,
                    err_msg: None
                }),
                get_sysinfo: None
            }
        );
        assert_eq!(
            serde_json::from_str::<Reply>(
                r#"{"smartlife.iot.dimmer":{"set_brightness":{"err_code":0}}}"#
            )
            .unwrap(),
            Reply::Dimmer {
                set_brightness: Some(ErrorStatus {
                    err_code: 0,
                    err_msg: None
                }),
            }
        );
        assert_eq!(
            serde_json::from_str::<Reply>(
                r#"{"system":{"get_sysinfo":{"sw_ver":"1.0.3 Build 210202 Rel.190636","hw_ver":"3.0","model":"HS220(US)","deviceId":"1234","oemId":"5678","hwId":"9999","rssi":-32,"latitude_i":90,"longitude_i":0,"alias":"Front Porch","status":"new","mic_type":"IOT.SMARTPLUGSWITCH","feature":"TIM","mac":"AA:AA:AA:AA:AA:AA","updating":0,"led_off":0,"relay_state":0,"brightness":50,"on_time":0,"icon_hash":"","dev_name":"Wi-Fi Smart Dimmer","active_mode":"none","next_action":{"type":-1},"preferred_state":[{"index":0,"brightness":100},{"index":1,"brightness":75},{"index":2,"brightness":50},{"index":3,"brightness":25}],"ntc_state":0,"err_code":0}}}"#
            )
            .unwrap(),
            Reply::System {
                set_relay_state: None,
                set_led_off: None,
                get_sysinfo: Some(InfoReply {
		    sw_ver: "1.0.3 Build 210202 Rel.190636".into(),
		    hw_ver: "3.0".into(),
		    model: "HS220(US)".into(),
		    device_id: "1234".into(),
		    oem_id: "5678".into(),
		    hw_id: "9999".into(),
		    updating: Some(0),
		    led_off: Some(0),
		    relay_state: Some(0),
		    brightness: Some(50),
		    err_code: 0,
		})
            }
        );
    }
}

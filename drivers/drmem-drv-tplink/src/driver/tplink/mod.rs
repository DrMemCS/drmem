// This module defines the commands that can be sent to the TP-Link
// device. It also configures the `serde` crate so these commands are
// converted to the expected JSON layout.

use serde::Deserialize;

mod cmd;
pub mod crypt;

pub use cmd::Cmd;

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
        crypt::decode(buf);
        serde_json::from_slice(buf).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json;

    #[test]
    fn test_cmds() {
        assert_eq!(
            serde_json::to_string(&Cmd::mk_active_cmd(1)).unwrap(),
            "{\"system\":{\"set_relay_state\":{\"state\":1}}}"
        );
        assert_eq!(
            serde_json::to_string(&Cmd::mk_led_cmd(false)).unwrap(),
            "{\"system\":{\"set_led_off\":{\"off\":1}}}"
        );
        assert_eq!(
            serde_json::to_string(&Cmd::mk_led_cmd(true)).unwrap(),
            "{\"system\":{\"set_led_off\":{\"off\":0}}}"
        );
        assert_eq!(
            serde_json::to_string(&Cmd::mk_info_cmd()).unwrap(),
            "{\"system\":{\"get_sysinfo\":{}}}"
        );
        assert_eq!(
            serde_json::to_string(&Cmd::mk_brightness_cmd(0)).unwrap(),
            "{\"smartlife.iot.dimmer\":{\"set_brightness\":{\"brightness\":0}}}"
        );
        assert_eq!(
            serde_json::to_string(&Cmd::mk_brightness_cmd(50)).unwrap(),
            "{\"smartlife.iot.dimmer\":{\"set_brightness\":{\"brightness\":50}}}"
        );
        assert_eq!(
            serde_json::to_string(&Cmd::mk_brightness_cmd(100)).unwrap(),
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

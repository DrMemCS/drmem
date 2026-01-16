use crate::tplink::crypt;
use serde::Serialize;

// This module holds the types that represent values used in the
// request fields.

pub mod value {
    use serde::Serialize;

    // Defines the internal value used by the `set_relay_state`
    // command. Needs to convert to `{"state":value}`.

    #[derive(Serialize, PartialEq, Debug)]
    pub struct Active {
        pub state: u8,
    }

    // Defines the internal value used by the `set_led_off`
    // command. Needs to convert to `{"off":value}`.

    #[derive(Serialize, PartialEq, Debug)]
    pub struct Led {
        pub off: u8,
    }

    // Defines the internal value used by the `get_sysinfo`
    // command. Needs to convert to `{}`.

    #[derive(Serialize, PartialEq, Debug)]
    pub struct Info {}

    // Defines the internal value used by the `Brightness`
    // command. Needs to convert to `{"brightness":value}`.

    #[derive(Serialize, PartialEq, Debug)]
    pub struct Brightness {
        pub brightness: u8,
    }
}

#[derive(Serialize, PartialEq, Debug)]
pub enum Cmd {
    #[serde(rename = "system")]
    System {
        #[serde(skip_serializing_if = "Option::is_none")]
        set_relay_state: Option<value::Active>,
        #[serde(skip_serializing_if = "Option::is_none")]
        get_sysinfo: Option<value::Info>,
        #[serde(skip_serializing_if = "Option::is_none")]
        set_led_off: Option<value::Led>,
    },

    #[serde(rename = "smartlife.iot.dimmer")]
    Dimmer { set_brightness: value::Brightness },
}

impl Cmd {
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(100);

        buf.push(0u8);
        buf.push(0u8);
        buf.push(0u8);
        buf.push(0u8);

        serde_json::to_writer(&mut buf, &self).unwrap();

        let sz = buf.len() - 4;

        buf[0] = (sz >> 24) as u8;
        buf[1] = (sz >> 16) as u8;
        buf[2] = (sz >> 8) as u8;
        buf[3] = sz as u8;

        crypt::encode(&mut buf[4..]);

        buf
    }

    pub fn mk_active_cmd(v: u8) -> Self {
        Self::System {
            set_relay_state: Some(value::Active { state: v }),
            get_sysinfo: None,
            set_led_off: None,
        }
    }

    pub fn mk_brightness_cmd(v: u8) -> Self {
        Self::Dimmer {
            set_brightness: value::Brightness { brightness: v },
        }
    }

    pub fn mk_info_cmd() -> Self {
        Self::System {
            set_relay_state: None,
            get_sysinfo: Some(value::Info {}),
            set_led_off: None,
        }
    }

    pub fn mk_led_cmd(v: bool) -> Self {
        Self::System {
            set_relay_state: None,
            get_sysinfo: None,
            set_led_off: Some(value::Led { off: (!v) as u8 }),
        }
    }
}

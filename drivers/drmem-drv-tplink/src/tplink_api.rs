// This module defines the commands that can be sent to the TP-Link
// device. It also configures the `serde` crate so these commands are
// converted to the expected JSON layout.

use serde::{Deserialize, Serialize, Serializer};
use std::marker::PhantomData;

// Defines the internal value used by the `Active` command. Needs
// to convert to `{"state":value}`.

#[derive(Serialize, PartialEq, Debug)]
pub struct ActiveValue {
    #[serde(rename = "state")]
    pub value: u8,
}

// Defines the internal value used by the `Info` command. Needs
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
    #[serde(rename = "brightness")]
    pub value: u8,
}

#[derive(Serialize, PartialEq, Debug)]
pub enum Cmd {
    #[serde(rename = "system")]
    Active {
        #[serde(rename = "set_relay_state")]
        value: ActiveValue,
    },

    #[serde(rename = "system")]
    Info {
        #[serde(rename = "get_sysinfo")]
        value: InfoValue,
    },

    #[serde(rename = "smartlife.iot.dimmer")]
    Brightness {
        #[serde(rename = "set_brightness")]
        value: BrightnessValue,
    },
}

pub fn active_cmd(v: u8) -> Cmd {
    Cmd::Active {
        value: ActiveValue { value: v },
    }
}

pub fn brightness_cmd(v: u8) -> Cmd {
    Cmd::Brightness {
        value: BrightnessValue { value: v },
    }
}

pub fn info_cmd() -> Cmd {
    Cmd::Info {
        value: InfoValue {
            nothing: PhantomData,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json;

    #[test]
    fn test_cmds() {
        assert_eq!(
            serde_json::to_string(&active_cmd(1)).unwrap(),
            "{\"system\":{\"set_relay_state\":{\"state\":1}}}"
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
}

// This module defines the commands that can be sent to the TP-Link
// device. It also configures the `serde` crate so these commands are
// converted to the expected JSON layout.

use serde::{Deserialize, Serialize, Serializer};
use std::marker::PhantomData;

// Defines the internal value used by the `Active` command. Needs
// to convert to `{"state":value}`.

#[derive(Serialize, PartialEq, Debug)]
pub struct ActiveValue {
    pub state: u8,
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
    },

    #[serde(rename = "smartlife.iot.dimmer")]
    Dimmer {
        #[serde(rename = "set_brightness")]
        set_brightness: BrightnessValue,
    },
}

pub fn active_cmd(v: u8) -> Cmd {
    Cmd::System {
        set_relay_state: Some(ActiveValue { state: v }),
        get_sysinfo: None,
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

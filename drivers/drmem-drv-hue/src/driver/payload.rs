use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub struct HueEvent {
    pub creationtime: String,
    pub data: Vec<ResourceData>,
    #[serde(rename = "type")]
    pub event_type: String,
}

#[derive(Serialize)]
pub struct LightCommand {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on: Option<On>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub dimming: Option<Dimming>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<Color>,
}

// Component Payloads
#[derive(Debug, Deserialize, Serialize)]
pub struct On {
    pub on: bool,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Dimming {
    pub brightness: f32,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Color {
    pub xy: XyCoordinates,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct XyCoordinates {
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Deserialize)]
pub struct ResourceData {
    pub id: String,
    #[serde(rename = "type")]
    pub res_type: String, // 'light', 'button', 'motion', etc.

    // Optional fields for light updates
    pub on: Option<On>,
    pub dimming: Option<Dimming>,
    pub color: Option<Color>,
}

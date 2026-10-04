use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Deserialize)]
pub struct HueResponse {
    // The list of resources returned by the API call.
    pub data: Vec<ResourceData>,
}

#[derive(Serialize, Debug)]
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
    pub xy: Option<XyCoordinates>,

    // Reported by the bridge but never part of a command.
    #[serde(default, skip_serializing)]
    pub gamut: Option<Gamut>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
pub struct XyCoordinates {
    pub x: f32,
    pub y: f32,
}

/// The triangle of xy coordinates a light can actually display.
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct Gamut {
    pub red: XyCoordinates,
    pub green: XyCoordinates,
    pub blue: XyCoordinates,
}

#[derive(Debug, Deserialize)]
pub struct ResourceData {
    pub id: Arc<str>,

    // Optional fields for light updates
    pub on: Option<On>,
    pub dimming: Option<Dimming>,
    pub color: Option<Color>,
}

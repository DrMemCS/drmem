/// Color conversion utilities for Hue CIE XY color space
use palette::{IntoColor, LinSrgb, LinSrgba, Yxy};

/// Convert Hue CIE XY coordinates to RGBA
pub fn cie_xy_to_rgba(x: f32, y: f32) -> LinSrgba<u8> {
    let yxy = Yxy::new(x, y, 1.0);
    let rgb: LinSrgb = yxy.into_color();

    LinSrgba::new(
        (rgb.red.clamp(0.0, 1.0) * 255.0) as u8,
        (rgb.green.clamp(0.0, 1.0) * 255.0) as u8,
        (rgb.blue.clamp(0.0, 1.0) * 255.0) as u8,
        255,
    )
}

/// Convert RGBA to Hue CIE XY coordinates
pub fn rgba_to_cie_xy(rgba: &LinSrgba<u8>) -> (f32, f32) {
    let rgb = LinSrgb::new(
        rgba.red as f32 / 255.0,
        rgba.green as f32 / 255.0,
        rgba.blue as f32 / 255.0,
    );
    let yxy: Yxy = rgb.into_color();
    (yxy.x, yxy.y)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_roundtrip_conversion() {
        let original = LinSrgba::new(255, 128, 64, 255);
        let (x, y) = rgba_to_cie_xy(&original);
        let converted = cie_xy_to_rgba(x, y);

        // Allow small differences due to color space conversion
        assert!((original.red as i32 - converted.red as i32).abs() <= 5);
        assert!((original.green as i32 - converted.green as i32).abs() <= 5);
        assert!((original.blue as i32 - converted.blue as i32).abs() <= 5);
    }
}

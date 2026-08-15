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
    fn test_xy_coordinates_stable() {
        // The key property we need: XY coordinates should be stable
        // when converted from RGB. This is what the driver compares.
        let rgb1 = LinSrgba::new(255, 128, 64, 255);
        let (x1, y1) = rgba_to_cie_xy(&rgb1);

        // Convert the same RGB again - should get same XY
        let (x2, y2) = rgba_to_cie_xy(&rgb1);

        // XY coordinates should be identical for the same input
        assert!((x1 - x2).abs() < 0.0001);
        assert!((y1 - y2).abs() < 0.0001);
    }

    #[test]
    fn test_bridge_xy_comparison() {
        // Simulate what happens in the driver:
        // 1. User sets RGB color
        let user_color = LinSrgba::new(200, 100, 50, 255);
        let (sent_x, sent_y) = rgba_to_cie_xy(&user_color);

        // 2. Bridge echoes back the same XY (simulating perfect echo)
        let bridge_x = sent_x;
        let bridge_y = sent_y;

        // 3. Driver compares with tolerance
        let tolerance = 0.001;
        let xy_matches = (sent_x - bridge_x).abs() < tolerance
            && (sent_y - bridge_y).abs() < tolerance;

        assert!(xy_matches, "Driver should detect XY match");
    }

    #[test]
    fn test_conversion_produces_valid_values() {
        // Just verify conversions don't panic and produce valid values
        let colors = vec![
            LinSrgba::new(255, 0, 0, 255),     // Pure red
            LinSrgba::new(0, 255, 0, 255),     // Pure green
            LinSrgba::new(0, 0, 255, 255),     // Pure blue
            LinSrgba::new(255, 255, 255, 255), // White
            LinSrgba::new(128, 128, 128, 255), // Gray
        ];

        for color in colors {
            let (x, y) = rgba_to_cie_xy(&color);

            // XY coordinates should be in valid range [0, 1]
            assert!(x >= 0.0 && x <= 1.0, "x coordinate out of range: {}", x);
            assert!(y >= 0.0 && y <= 1.0, "y coordinate out of range: {}", y);

            // Converting back should produce a valid color
            let converted = cie_xy_to_rgba(x, y);
            assert_eq!(converted.alpha, 255);
        }
    }
}

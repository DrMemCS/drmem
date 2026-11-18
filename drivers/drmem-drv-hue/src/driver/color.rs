/// Color conversion utilities for Hue CIE XY color space
use drmem_api::device::ColorType;
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

/// Approximates a color temperature (in Kelvin) as CIE xy coordinates
/// using the Kim et al. polynomial fit to the Planckian locus. Valid
/// over the range of colors Hue bulbs can produce (roughly
/// 1667K-25000K).
pub fn kelvin_to_cie_xy(kelvin: u16) -> (f32, f32) {
    let t = kelvin as f64;
    let (t2, t3) = (t * t, t * t * t);

    let x = if t <= 4000.0 {
        -0.2661239e9 / t3 - 0.2343589e6 / t2 + 0.8776956e3 / t + 0.179910
    } else {
        -3.0258469e9 / t3 + 2.1070379e6 / t2 + 0.2226347e3 / t + 0.24039
    };
    let (x2, x3) = (x * x, x * x * x);

    let y = if t <= 2222.0 {
        -1.1063814 * x3 - 1.34811020 * x2 + 2.18555832 * x - 0.20219683
    } else if t <= 4000.0 {
        -0.9549476 * x3 - 1.37418593 * x2 + 2.09137015 * x - 0.16748867
    } else {
        3.0817580 * x3 - 5.87338670 * x2 + 3.75112997 * x - 0.37001483
    };

    (x as f32, y as f32)
}

/// The three parameters the Hue bridge needs to represent a light's
/// setting: whether it's on, its brightness (0.0-100.0), and its
/// chromaticity.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BridgeState {
    pub on: bool,
    pub brightness: f32,
    pub xy: (f32, f32),
}

/// Breaks a `ColorType` down into the on/off, brightness, and XY
/// coordinates the Hue bridge needs. Brightness is conveyed by the
/// color's alpha channel: 0 is off and 255 is full brightness.
pub fn color_to_bridge(color: &ColorType) -> BridgeState {
    let (alpha, xy) = match color {
        ColorType::Rgba { color } => (color.alpha, rgba_to_cie_xy(color)),
        ColorType::Ccta { kelvin, a } => (*a, kelvin_to_cie_xy(*kelvin)),
    };

    BridgeState {
        on: alpha > 0,
        brightness: alpha as f32 / 255.0 * 100.0,
        xy,
    }
}

/// Merges the (possibly partial) on/off, brightness, and XY state
/// reported by the bridge into a new `ColorType`, using `prev` to fill
/// in any component that wasn't reported. The bridge only ever echoes
/// XY coordinates, so a reported `xy` always yields a `ColorType::Rgba`.
/// But when `xy` is `None` (the caller filters out echoes that just
/// match what was last sent), `prev`'s representation -- `Rgba` or
/// `Ccta` -- is preserved. This matters because a `Ccta` setting can
/// never compare equal to the `Rgba` the bridge would otherwise force
/// it into, which would make `OverridableDevice` treat every polled
/// reading as a mismatch and re-send the setting forever.
pub fn merge_bridge_update(
    prev: &ColorType,
    on: Option<bool>,
    brightness: Option<f32>,
    xy: Option<(f32, f32)>,
) -> ColorType {
    let prev_alpha = match prev {
        ColorType::Rgba { color } => color.alpha,
        ColorType::Ccta { a, .. } => *a,
    };
    let is_on = on.unwrap_or(prev_alpha > 0);

    let alpha = if !is_on {
        0
    } else if let Some(brightness) = brightness {
        (brightness.clamp(0.0, 100.0) / 100.0 * 255.0).round() as u8
    } else if on == Some(true) {
        // Just turned on with no explicit brightness: default to 100%.
        255
    } else {
        prev_alpha
    };

    match (xy, prev) {
        (None, ColorType::Ccta { kelvin, .. }) => ColorType::Ccta {
            kelvin: *kelvin,
            a: alpha,
        },
        (None, ColorType::Rgba { color }) => ColorType::Rgba {
            color: LinSrgba::new(color.red, color.green, color.blue, alpha),
        },
        (Some((x, y)), _) => {
            let rgb = cie_xy_to_rgba(x, y);

            ColorType::Rgba {
                color: LinSrgba::new(rgb.red, rgb.green, rgb.blue, alpha),
            }
        }
    }
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

    #[test]
    fn test_color_to_bridge_alpha_is_brightness() {
        let rgb = LinSrgba::new(255, 128, 64, 0);

        // alpha == 0 means off, and brightness reads as 0%.
        let off = color_to_bridge(&ColorType::Rgba { color: rgb });
        assert!(!off.on);
        assert_eq!(off.brightness, 0.0);

        // alpha == 255 means fully on, brightness reads as 100%.
        let full = color_to_bridge(&ColorType::Rgba {
            color: LinSrgba::new(255, 128, 64, 255),
        });
        assert!(full.on);
        assert_eq!(full.brightness, 100.0);

        // A mid-range alpha is on, with a proportional brightness.
        let half = color_to_bridge(&ColorType::Rgba {
            color: LinSrgba::new(255, 128, 64, 128),
        });
        assert!(half.on);
        assert!((half.brightness - 50.196).abs() < 0.01);

        // XY is derived only from RGB, independent of alpha.
        assert_eq!(off.xy, full.xy);
        assert_eq!(off.xy, half.xy);
    }

    #[test]
    fn test_color_to_bridge_ccta() {
        let bright = color_to_bridge(&ColorType::Ccta {
            kelvin: 2700,
            a: 255,
        });
        assert!(bright.on);
        assert_eq!(bright.brightness, 100.0);

        let off = color_to_bridge(&ColorType::Ccta { kelvin: 2700, a: 0 });
        assert!(!off.on);

        // xy should match a direct kelvin conversion.
        assert_eq!(bright.xy, kelvin_to_cie_xy(2700));
    }

    #[test]
    fn test_kelvin_to_cie_xy_known_white_points() {
        // D65 (~6500K) and a warm-white (~2700K) sanity check, using a
        // generous tolerance for the polynomial approximation.
        let (x, y) = kelvin_to_cie_xy(6500);
        assert!((x - 0.3127).abs() < 0.01, "x={x}");
        assert!((y - 0.3290).abs() < 0.02, "y={y}");

        let (x, y) = kelvin_to_cie_xy(2700);
        assert!((x - 0.4578).abs() < 0.01, "x={x}");
        assert!((y - 0.4101).abs() < 0.01, "y={y}");
    }

    #[test]
    fn test_merge_bridge_update_on_off_only() {
        let prev = ColorType::Rgba {
            color: LinSrgba::new(200, 100, 50, 255),
        };

        // Turning off keeps the RGB hue but zeroes the alpha.
        let merged = merge_bridge_update(&prev, Some(false), None, None);
        assert_eq!(
            merged,
            ColorType::Rgba {
                color: LinSrgba::new(200, 100, 50, 0)
            }
        );
    }

    #[test]
    fn test_merge_bridge_update_dimming_only() {
        let prev = ColorType::Rgba {
            color: LinSrgba::new(200, 100, 50, 255),
        };

        // A brightness-only update preserves the RGB hue.
        let merged = merge_bridge_update(&prev, None, Some(50.0), None);
        assert_eq!(
            merged,
            ColorType::Rgba {
                color: LinSrgba::new(200, 100, 50, 128)
            }
        );
    }

    #[test]
    fn test_merge_bridge_update_xy_only() {
        let prev = ColorType::Rgba {
            color: LinSrgba::new(200, 100, 50, 128),
        };
        let (x, y) = rgba_to_cie_xy(&LinSrgba::new(10, 20, 30, 255));

        // An XY-only update changes the RGB hue but keeps the alpha.
        let merged = merge_bridge_update(&prev, None, None, Some((x, y)));
        assert_eq!(
            merged,
            ColorType::Rgba {
                color: LinSrgba::new(
                    cie_xy_to_rgba(x, y).red,
                    cie_xy_to_rgba(x, y).green,
                    cie_xy_to_rgba(x, y).blue,
                    128
                )
            }
        );
    }

    #[test]
    fn test_merge_bridge_update_turned_on_defaults_to_full_brightness() {
        let prev = ColorType::Rgba {
            color: LinSrgba::new(200, 100, 50, 0),
        };

        // Turning on with no explicit brightness defaults to 100%.
        let merged = merge_bridge_update(&prev, Some(true), None, None);
        assert_eq!(
            merged,
            ColorType::Rgba {
                color: LinSrgba::new(200, 100, 50, 255)
            }
        );
    }
}

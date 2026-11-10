//! Straight-alpha sRGB entry conversions shared by colour-editing adapters.
use crate::{Error, Result};
use project::Color;

/// Six RGB or eight RGBA hexadecimal digits, with one optional leading '#'.
/// RGB entry is opaque; alpha is never premultiplied or gamma-converted here.
pub fn parse_hex(value: &str) -> Result<Color> {
    let value = value.trim().strip_prefix('#').unwrap_or(value.trim());
    if !matches!(value.len(), 6 | 8) || !value.is_ascii() {
        return Err(Error::Invalid("hex colour"));
    }
    let mut rgba = [255; 4];
    for (channel, pair) in rgba.iter_mut().zip(value.as_bytes().as_chunks::<2>().0) {
        let text = std::str::from_utf8(pair).map_err(|_| Error::Invalid("hex colour"))?;
        *channel = u8::from_str_radix(text, 16).map_err(|_| Error::Invalid("hex colour"))?;
    }
    Ok(Color(rgba))
}

pub fn hex(color: Color) -> String {
    let [r, g, b, a] = color.0;
    format!("#{r:02X}{g:02X}{b:02X}{a:02X}")
}
/// Resolve an existing colour channel, including composite linear-light RGBA.
pub fn at(
    animations: &project::Animations,
    path: &project::AnimationPath,
    at: time::Tick,
    fallback: Color,
) -> Result<Color> {
    match animations.value_at(path, at)? {
        None => Ok(fallback),
        Some(project::ParamValue::Text(value)) => parse_hex(value.as_str()),
        Some(project::ParamValue::Color(values)) => {
            let [r, g, b, a] = values;
            let [r, g, b] =
                [r, g, b].map(|v| (crate::image::oetf(v as f32, 13) * 255.0).round() as u8);
            Ok(Color([r, g, b, (a * 255.0).round() as u8]))
        }
        _ => Err(Error::Invalid("colour channel")),
    }
}
/// Hue in degrees 0..360, saturation/value/alpha in 0..1, on encoded sRGB.
pub fn to_hsva(color: Color) -> [f64; 4] {
    let [r, g, b, a] = color.0.map(|c| f64::from(c) / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;
    let hue = if delta == 0.0 {
        0.0
    } else if max == r {
        60.0 * ((g - b) / delta).rem_euclid(6.0)
    } else if max == g {
        60.0 * ((b - r) / delta + 2.0)
    } else {
        60.0 * ((r - g) / delta + 4.0)
    };
    [hue, if max == 0.0 { 0.0 } else { delta / max }, max, a]
}
/// Finite entries clamp to their documented bounds; non-finite entries refuse.
pub fn from_hsva([h, s, v, a]: [f64; 4]) -> Result<Color> {
    if [h, s, v, a].iter().any(|c| !c.is_finite()) {
        return Err(Error::Invalid("HSV colour"));
    }
    let h = h.clamp(0.0, 360.0) / 60.0;
    let s = s.clamp(0.0, 1.0);
    let v = v.clamp(0.0, 1.0);
    let c = v * s;
    let x = c * (1.0 - (h.rem_euclid(2.0) - 1.0).abs());
    let m = v - c;
    let rgb = match h as u8 {
        0 | 6 => [c, x, 0.0],
        1 => [x, c, 0.0],
        2 => [0.0, c, x],
        3 => [0.0, x, c],
        4 => [x, 0.0, c],
        _ => [c, 0.0, x],
    };
    let [r, g, b] = rgb.map(|n| ((n + m) * 255.0).round() as u8);
    Ok(Color([r, g, b, (a.clamp(0.0, 1.0) * 255.0).round() as u8]))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hex_hsv_roundtrip_preserves_straight_alpha_and_rejects_hostile_entries() {
        for r in [0, 1, 63, 127, 128, 254, 255] {
            for g in [0, 1, 127, 255] {
                for b in [0, 1, 128, 255] {
                    for a in [0, 1, 128, 254, 255] {
                        let color = Color([r, g, b, a]);
                        assert_eq!(parse_hex(&hex(color)).ok(), Some(color));
                        assert_eq!(from_hsva(to_hsva(color)).ok(), Some(color));
                    }
                }
            }
        }
        assert_eq!(
            parse_hex(" #aAbBcc ").ok(),
            Some(Color([170, 187, 204, 255]))
        );
        for invalid in [
            "",
            "#123",
            "##abcdef",
            "abcdefgg",
            "💚abcd",
            "123456789",
            "#1234567",
        ] {
            assert!(parse_hex(invalid).is_err(), "{invalid}");
        }
        assert_eq!(
            from_hsva([360.0, 2.0, 3.0, -1.0]).ok(),
            Some(Color([255, 0, 0, 0]))
        );
        assert_eq!(
            from_hsva([-1.0, -1.0, -1.0, 2.0]).ok(),
            Some(Color([0, 0, 0, 255]))
        );
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            for index in 0..4 {
                let mut values = [0.5; 4];
                if let Some(component) = values.get_mut(index) {
                    *component = value;
                }
                assert!(from_hsva(values).is_err());
            }
        }
    }

    #[test]
    fn existing_linear_colour_channels_convert_once_without_premultiplying_alpha() {
        let mut animations = project::Animations::default();
        animations
            .set(
                project::AnimationPath::new("params.strokeColor").unwrap(),
                project::Param::new(project::ParamValue::Color([0.5, 0.0, 1.0, 0.5])).unwrap(),
            )
            .unwrap();
        assert_eq!(
            at(
                &animations,
                &project::AnimationPath::new("params.strokeColor").unwrap(),
                time::Tick::ZERO,
                Color([0; 4])
            )
            .unwrap(),
            Color([188, 0, 255, 128])
        );
    }
}

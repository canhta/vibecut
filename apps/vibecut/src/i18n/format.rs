//! Locale formatting for messages, numbers, plurals and dates.

use super::{Locale, Text, text};
use crate::{Error, Result};

/// Creator labels shared by effect tiles, search, Properties and add feedback.
/// `None` selects the kind name; unknown identities use the definition's label.
pub fn effect_label(kind: &str, parameter: Option<&str>) -> Option<Text> {
    Some(match (kind, parameter) {
        ("directional_blur", None) => Text::EffectStreakBlur,
        ("directional_blur", Some("direction")) => Text::EffectDirection,
        ("directional_blur", Some("length")) => Text::EffectLength,
        ("sharpen", None) => Text::EffectSharpen,
        ("sharpen", Some("amount")) => Text::EffectAmount,
        ("find_edges", None) => Text::EffectEdgeLines,
        ("find_edges", Some("invert")) => Text::EffectInvert,
        ("find_edges", Some("blend")) => Text::EffectBlendOriginal,
        ("color_emboss", None) => Text::EffectColourRelief,
        ("color_emboss", Some("direction")) => Text::EffectDirection,
        ("color_emboss", Some("relief")) => Text::EffectRelief,
        ("color_emboss", Some("contrast")) => Text::Contrast,
        ("color_emboss", Some("blend")) => Text::EffectBlendOriginal,
        ("blur", None) => Text::EffectBlur,
        ("color_grade", None) => Text::EffectColorGrade,
        ("chroma_key", None) => Text::ChromaKey,
        ("black_white", None) => Text::EffectBlackWhite,
        ("channel_mix", None) => Text::EffectChannelMixer,
        ("color_pass", None) => Text::EffectKeepColour,
        ("color_replace", None) => Text::EffectSwapColour,
        ("extract", None) => Text::EffectThreshold,
        ("gamma_correction", None) => Text::EffectGamma,
        ("invert", None) => Text::EffectInvert,
        ("levels", None) => Text::EffectLevels,
        ("posterize", None) => Text::EffectPosterise,
        ("strobe", None) => Text::EffectStrobe,
        ("tint", None) => Text::EffectTwoTone,
        ("blur" | "chroma_key", Some("intensity")) => Text::EffectIntensity,
        ("chroma_key", Some("shadow")) => Text::EffectShadow,
        ("color_grade", Some("brightness")) => Text::Brightness,
        ("color_grade", Some("contrast")) => Text::Contrast,
        ("color_grade", Some("saturation")) => Text::Saturation,
        ("color_grade", Some("hue")) => Text::Hue,
        ("color_grade", Some("lift_r")) => Text::EffectLiftRed,
        ("color_grade", Some("lift_g")) => Text::EffectLiftGreen,
        ("color_grade", Some("lift_b")) => Text::EffectLiftBlue,
        ("color_grade", Some("gamma_r")) => Text::EffectGammaRed,
        ("color_grade", Some("gamma_g")) => Text::EffectGammaGreen,
        ("color_grade", Some("gamma_b")) => Text::EffectGammaBlue,
        ("color_grade", Some("gain_r")) => Text::EffectGainRed,
        ("color_grade", Some("gain_g")) => Text::EffectGainGreen,
        ("color_grade", Some("gain_b")) => Text::EffectGainBlue,
        ("color_grade", Some("lut_strength")) => Text::LutStrength,
        ("channel_mix", Some("rr")) => Text::EffectRedFromRed,
        ("channel_mix", Some("rg")) => Text::EffectRedFromGreen,
        ("channel_mix", Some("rb")) => Text::EffectRedFromBlue,
        ("channel_mix", Some("rc")) => Text::EffectRedConstant,
        ("channel_mix", Some("gr")) => Text::EffectGreenFromRed,
        ("channel_mix", Some("gg")) => Text::EffectGreenFromGreen,
        ("channel_mix", Some("gb")) => Text::EffectGreenFromBlue,
        ("channel_mix", Some("gc")) => Text::EffectGreenConstant,
        ("channel_mix", Some("br")) => Text::EffectBlueFromRed,
        ("channel_mix", Some("bg")) => Text::EffectBlueFromGreen,
        ("channel_mix", Some("bb")) => Text::EffectBlueFromBlue,
        ("channel_mix", Some("bc")) => Text::EffectBlueConstant,
        ("color_pass" | "color_replace", Some("similarity")) => Text::EffectSimilarity,
        ("extract", Some("black")) => Text::EffectBlackPoint,
        ("extract", Some("white")) => Text::EffectWhitePoint,
        ("extract", Some("softness")) => Text::EffectSoftness,
        ("gamma_correction" | "levels", Some("gamma")) => Text::EffectGamma,
        ("invert" | "strobe", Some("blend")) => Text::EffectBlendOriginal,
        ("levels", Some("in_black")) => Text::EffectInputBlack,
        ("levels", Some("in_white")) => Text::EffectInputWhite,
        ("levels", Some("out_black")) => Text::EffectOutputBlack,
        ("levels", Some("out_white")) => Text::EffectOutputWhite,
        ("mosaic", None) => Text::EffectPixelate,
        ("mosaic", Some("horizontal")) => Text::EffectHorizontal,
        ("mosaic", Some("vertical")) => Text::EffectVertical,
        ("mosaic", Some("sharp")) => Text::EffectSharp,
        ("mosaic", Some("softness")) => Text::EffectSoftness,
        ("posterize", Some("levels")) => Text::EffectLevels,
        ("strobe", Some("duration")) => Text::SpeedDuration,
        ("strobe", Some("period")) => Text::EffectPeriod,
        ("tint", Some("amount")) => Text::EffectAmount,
        _ => return None,
    })
}

/// Fill `{name}` placeholders in a catalog message in one pass, so text a
/// replacement inserts is never read as another placeholder.
pub fn format(locale: Locale, key: Text, args: &[(&str, &str)]) -> String {
    let mut out = String::new();
    let mut rest = key.text(locale);
    while let Some(open) = rest.find('{') {
        out.push_str(rest.get(..open).unwrap_or(""));
        let Some(relative_close) = rest.get(open..).and_then(|tail| tail.find('}')) else {
            out.push_str(rest.get(open..).unwrap_or(""));
            return out;
        };
        let close = open + relative_close;
        let name = rest.get(open + 1..close).unwrap_or("");
        match args.iter().find(|(argument, _)| *argument == name) {
            Some((_, value)) => out.push_str(value),
            None => out.push_str(rest.get(open..close + 1).unwrap_or("")),
        }
        rest = rest.get(close + 1..).unwrap_or("");
    }
    out.push_str(rest);
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Plural {
    One,
    Other,
}

/// English distinguishes one from other; Vietnamese CLDR has one `other`
/// category, so it always takes the `other` row.
pub const fn plural(locale: Locale, count: u64) -> Plural {
    match locale {
        Locale::En if count == 1 => Plural::One,
        _ => Plural::Other,
    }
}

/// A count message with the locale's plural row and group-formatted number.
pub fn format_count(locale: Locale, count: u64, one: Text, other: Text) -> String {
    let key = match plural(locale, count) {
        Plural::One => one,
        Plural::Other => other,
    };
    format(locale, key, &[("n", &number(locale, count))])
}

fn separators(locale: Locale) -> (char, char) {
    match locale {
        Locale::En => (',', '.'),
        Locale::Vi => ('.', ','),
    }
}
fn group(digits: &str, separator: char) -> String {
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    let len = digits.len();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (len - index).is_multiple_of(3) {
            out.push(separator);
        }
        out.push(digit);
    }
    out
}

/// An unsigned integer with the locale's grouping separator.
pub fn number(locale: Locale, value: u64) -> String {
    group(&value.to_string(), separators(locale).0)
}

/// A signed integer with the locale's grouping separator.
pub fn signed(locale: Locale, value: i64) -> String {
    let (sign, magnitude) = if value < 0 {
        ("-", value.unsigned_abs())
    } else {
        ("", value as u64)
    };
    std::format!("{sign}{}", number(locale, magnitude))
}

/// A finite number with `decimals` digits after the locale's decimal mark.
pub fn decimal(locale: Locale, value: f64, decimals: usize) -> String {
    if !value.is_finite() {
        return value.to_string();
    }
    let (grouping, decimal) = separators(locale);
    let text = std::format!("{value:.decimals$}");
    let (sign, text) = text
        .strip_prefix('-')
        .map_or(("", text.as_str()), |rest| ("-", rest));
    let (integer, fraction) = text.split_once('.').unwrap_or((text, ""));
    let mut out = String::with_capacity(text.len() + 4);
    out.push_str(sign);
    out.push_str(&group(integer, grouping));
    if !fraction.is_empty() {
        out.push(decimal);
        out.push_str(fraction);
    }
    out
}

/// Parse a finite decimal in the same notation as [`decimal`]. Grouping,
/// when present, must use complete groups of three; exponent notation and
/// malformed/mixed separators refuse. Input is bounded to 128 bytes.
pub fn parse_decimal(locale: Locale, input: &str) -> Result<f64> {
    let invalid = || Error::Invalid("localized decimal");
    if input.len() > 128 {
        return Err(invalid());
    }
    let value = input.trim();
    if value.is_empty() {
        return Err(invalid());
    }
    let (grouping, decimal) = separators(locale);
    let (sign, value) = value
        .strip_prefix('-')
        .map_or(("", value), |rest| ("-", rest));
    let mut parts = value.split(decimal);
    let integer = parts.next().ok_or_else(invalid)?;
    let fraction = parts.next();
    if parts.next().is_some()
        || fraction.is_some_and(|s| s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()))
    {
        return Err(invalid());
    }
    let digits = if integer.contains(grouping) {
        let mut groups = integer.split(grouping);
        let first = groups.next().ok_or_else(invalid)?;
        if first.is_empty()
            || first.len() > 3
            || !first.bytes().all(|b| b.is_ascii_digit())
            || groups.any(|g| g.len() != 3 || !g.bytes().all(|b| b.is_ascii_digit()))
        {
            return Err(invalid());
        }
        integer.replace(grouping, "")
    } else {
        if !integer.bytes().all(|b| b.is_ascii_digit())
            || (integer.is_empty() && fraction.is_none())
        {
            return Err(invalid());
        }
        if integer.is_empty() {
            "0".into()
        } else {
            integer.into()
        }
    };
    let value = std::format!("{sign}{digits}.{}", fraction.unwrap_or("0"))
        .parse::<f64>()
        .map_err(|_| invalid())?;
    if !value.is_finite() {
        return Err(invalid());
    }
    Ok(value)
}

/// A short date in the locale's convention, from Unix milliseconds. The app
/// stores timestamps in UTC; this is display only, never edit or timing math.
pub fn date(locale: Locale, unix_ms: u64) -> String {
    let days = i64::try_from(unix_ms / 86_400_000).unwrap_or(i64::MAX);
    let (year, month, day) = civil_from_days(days);
    match locale {
        Locale::En => {
            let names = [
                "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
            ];
            let name = names
                .get(month.saturating_sub(1) as usize)
                .copied()
                .unwrap_or("");
            std::format!("{name} {day}, {year}")
        }
        Locale::Vi => std::format!("{day} thg {month}, {year}"),
    }
}

fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    (
        if month <= 2 { year + 1 } else { year },
        month as u32,
        day as u32,
    )
}

/// Export measurements stay in the catalog alongside their unit and locale.
pub fn export_duration(locale: Locale, seconds: f64) -> String {
    let label = text(locale, Text::ExportDuration);
    let number = export_decimal(locale, seconds, 2);
    match locale {
        Locale::En => format!("{label}: {number} s"),
        Locale::Vi => format!("{label}: {number} giây"),
    }
}
pub fn export_range(locale: Locale, start: f64, end: f64) -> String {
    let label = text(locale, Text::SourceRange);
    format!(
        "{label}: {}–{}",
        export_decimal(locale, start, 3),
        export_decimal(locale, end, 3)
    )
}
fn export_decimal(locale: Locale, value: f64, precision: usize) -> String {
    let number = format!("{value:.precision$}");
    match locale {
        Locale::En => number,
        Locale::Vi => number.replace('.', ","),
    }
}

/// Localized label; persisted blend IDs are locale-independent.
pub fn blend_label(mode: project::BlendMode) -> Text {
    match mode {
        project::BlendMode::Normal => Text::BlendNormal,
        project::BlendMode::Darken => Text::BlendDarken,
        project::BlendMode::Multiply => Text::BlendMultiply,
        project::BlendMode::ColorBurn => Text::BlendColorBurn,
        project::BlendMode::Lighten => Text::BlendLighten,
        project::BlendMode::Screen => Text::BlendScreen,
        project::BlendMode::PlusLighter => Text::BlendPlusLighter,
        project::BlendMode::ColorDodge => Text::BlendColorDodge,
        project::BlendMode::Overlay => Text::BlendOverlay,
        project::BlendMode::SoftLight => Text::BlendSoftLight,
        project::BlendMode::HardLight => Text::BlendHardLight,
        project::BlendMode::Difference => Text::BlendDifference,
        project::BlendMode::Exclusion => Text::BlendExclusion,
        project::BlendMode::Hue => Text::BlendHue,
        project::BlendMode::Saturation => Text::BlendSaturation,
        project::BlendMode::Color => Text::BlendColor,
        project::BlendMode::Luminosity => Text::BlendLuminosity,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn named_arguments_replace_without_leaving_placeholders() {
        let args = [("imported", "2"), ("failed", "1"), ("unfinished", "1")];
        assert_eq!(
            format(Locale::En, Text::ImportSummary, &args),
            "Imported: 2 · Failed: 1 · Unfinished: 1"
        );
        assert_eq!(
            format(Locale::Vi, Text::ImportSummary, &args),
            "Đã nhập: 2 · Thất bại: 1 · Chưa xử lý: 1"
        );
    }

    #[test]
    fn numbers_dates_and_plurals_follow_the_locale() {
        for (locale, input, expected) in [
            (Locale::En, "1,014.500", 1014.5),
            (Locale::Vi, "1.014,500", 1014.5),
            (Locale::En, ".5", 0.5),
            (Locale::Vi, ",5", 0.5),
            (Locale::En, "-1,014.500", -1014.5),
            (Locale::Vi, "-1.014,500", -1014.5),
        ] {
            assert_eq!(parse_decimal(locale, input).unwrap(), expected);
        }
        for locale in [Locale::En, Locale::Vi] {
            for input in ["", "NaN", "inf", "1e3", "1.01,4", "1,,000", "1..000", "--1"] {
                assert!(parse_decimal(locale, input).is_err(), "{locale:?} {input}");
            }
            assert!(parse_decimal(locale, &"1".repeat(129)).is_err());
        }
        assert_eq!(number(Locale::En, 1_234_567), "1,234,567");
        assert_eq!(number(Locale::Vi, 1_234_567), "1.234.567");
        assert_eq!(decimal(Locale::En, 23.976, 3), "23.976");
        assert_eq!(decimal(Locale::Vi, 23.976, 3), "23,976");
        assert_eq!(signed(Locale::Vi, -12_345), "-12.345");
        assert_eq!(date(Locale::En, 1_780_000_000_000), "May 28, 2026");
        assert_eq!(date(Locale::Vi, 1_780_000_000_000), "28 thg 5, 2026");
        assert_eq!(plural(Locale::En, 1), Plural::One);
        assert_eq!(plural(Locale::En, 0), Plural::Other);
        assert_eq!(plural(Locale::Vi, 1), Plural::Other);
        assert_eq!(
            format_count(
                Locale::En,
                1,
                Text::SelectedCountOne,
                Text::SelectedCountOther
            ),
            "1 item selected"
        );
        assert_eq!(
            format_count(
                Locale::En,
                1_234,
                Text::SelectedCountOne,
                Text::SelectedCountOther
            ),
            "1,234 items selected"
        );
        assert_eq!(
            format_count(
                Locale::Vi,
                1_234,
                Text::SelectedCountOne,
                Text::SelectedCountOther
            ),
            "1.234 mục đã chọn"
        );
    }
}

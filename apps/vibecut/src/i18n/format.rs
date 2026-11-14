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
        ("unsharp_mask", None) => Text::EffectDetailBoost,
        ("unsharp_mask", Some("amount")) => Text::EffectAmount,
        ("unsharp_mask", Some("radius")) => Text::EffectRadius,
        ("unsharp_mask", Some("threshold")) => Text::EffectThreshold,
        ("bokeh_blur", None) => Text::EffectLensBlur,
        ("bokeh_blur", Some("amount")) => Text::EffectBlurAmount,
        ("bokeh_blur", Some("shape")) => Text::EffectShape,
        ("bokeh_blur", Some("rotation")) => Text::EffectRotation,
        ("bokeh_blur", Some("highlight")) => Text::EffectHighlightBrightness,
        ("bokeh_blur", Some("threshold")) => Text::EffectHighlightThreshold,
        ("mirror", None) => Text::EffectMirror,
        ("mirror", Some("center_x")) => Text::EffectCentreX,
        ("mirror", Some("center_y")) => Text::EffectCentreY,
        ("mirror", Some("angle")) => Text::EffectReflectionAngle,
        ("offset", None) => Text::EffectWrapShift,
        ("offset", Some("shift_x")) => Text::EffectShiftX,
        ("offset", Some("shift_y")) => Text::EffectShiftY,
        ("offset", Some("blend")) => Text::EffectBlendOriginal,
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
        ("color_grade", Some(parameter)) if parameter.starts_with("hsl_") => {
            if parameter.ends_with("_hue") {
                Text::Hue
            } else if parameter.ends_with("_saturation") {
                Text::Saturation
            } else {
                Text::Lightness
            }
        }
        ("color_grade", Some("brightness")) => Text::Brightness,
        ("color_grade", Some("contrast")) => Text::Contrast,
        ("color_grade", Some("saturation")) => Text::Saturation,
        ("color_grade", Some("hue")) => Text::Hue,
        ("color_grade", Some("temperature")) => Text::EffectTemperature,
        ("color_grade", Some("highlight")) => Text::EffectHighlight,
        ("color_grade", Some("shadow")) => Text::EffectShadow,
        ("color_grade", Some("shine")) => Text::EffectShine,
        ("color_grade", Some("sharpness")) => Text::EffectSharpness,
        ("color_grade", Some("vignette")) => Text::EffectVignette,
        ("color_grade", Some("fade")) => Text::EffectFade,
        ("color_grade", Some("grain")) => Text::EffectGrain,
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
        ("twirl", None) => Text::EffectTwirl,
        ("twirl", Some("angle")) => Text::EffectAngle,
        ("twirl", Some("radius")) => Text::EffectRadius,
        ("twirl", Some("center")) => Text::EffectCentre,
        ("spherize", None) => Text::EffectBulge,
        ("spherize", Some("radius")) => Text::EffectRadius,
        ("spherize", Some("center")) => Text::EffectCentre,
        ("magnify", None) => Text::EffectMagnifier,
        ("magnify", Some("shape")) => Text::EffectShape,
        ("magnify", Some("center")) => Text::EffectCentre,
        ("magnify", Some("magnification")) => Text::EffectMagnification,
        ("magnify", Some("size")) => Text::EffectSize,
        ("magnify", Some("feather")) => Text::EffectFeather,
        ("wave_warp", None) => Text::EffectRipple,
        ("wave_warp", Some("height")) => Text::EffectWaveHeight,
        ("wave_warp", Some("width")) => Text::EffectWaveWidth,
        ("wave_warp", Some("direction")) => Text::EffectDirection,
        ("wave_warp", Some("speed")) => Text::EffectWaveSpeed,
        ("lens_distortion", None) => Text::EffectLensWarp,
        ("lens_distortion", Some("curvature")) => Text::EffectCurvature,
        ("lens_distortion", Some("v_decentering")) => Text::EffectVerticalDecentering,
        ("lens_distortion", Some("h_decentering")) => Text::EffectHorizontalDecentering,
        ("turbulent_displace", None) => Text::EffectHeatHaze,
        ("turbulent_displace", Some("displacement")) => Text::EffectDisplacement,
        ("turbulent_displace", Some("amount")) => Text::EffectAmount,
        ("turbulent_displace", Some("size")) => Text::EffectSize,
        ("turbulent_displace", Some("offset")) => Text::EffectOffset,
        ("turbulent_displace", Some("complexity")) => Text::EffectComplexity,
        ("turbulent_displace", Some("evolution")) => Text::EffectEvolution,
        ("corner_pin", None) => Text::EffectCornerPin,
        ("corner_pin", Some("upper_left")) => Text::EffectUpperLeft,
        ("corner_pin", Some("upper_right")) => Text::EffectUpperRight,
        ("corner_pin", Some("lower_left")) => Text::EffectLowerLeft,
        ("corner_pin", Some("lower_right")) => Text::EffectLowerRight,
        ("basic_3d", None) => Text::EffectTiltCard,
        ("basic_3d", Some("swivel")) => Text::EffectSwivel,
        ("basic_3d", Some("tilt")) => Text::EffectTilt,
        ("basic_3d", Some("distance")) => Text::EffectDistance,
        ("brush_strokes", None) => Text::EffectPaintStrokes,
        ("brush_strokes", Some("angle")) => Text::EffectStrokeAngle,
        ("brush_strokes", Some("size")) => Text::EffectBrushSize,
        ("brush_strokes", Some("length")) => Text::EffectStrokeLength,
        ("brush_strokes", Some("density")) => Text::EffectStrokeDensity,
        ("roughen_edges", None) => Text::EffectTornEdges,
        ("roughen_edges", Some("edge_type")) => Text::EffectEdgeType,
        ("roughen_edges", Some("border")) => Text::EffectEdgeWidth,
        ("roughen_edges", Some("scale")) => Text::EffectScale,
        ("roughen_edges", Some("complexity")) => Text::EffectComplexity,
        ("camera_shake", None) => Text::EffectHandheldShake,
        ("camera_shake", Some("amount")) => Text::EffectPositionAmount,
        ("camera_shake", Some("rotation")) => Text::EffectRotationAmount,
        ("camera_shake", Some("zoom")) => Text::EffectZoomAmount,
        ("camera_shake", Some("frequency")) => Text::EffectFrequency,
        ("posterize_time", None) => Text::EffectLowFrameRate,
        ("posterize_time", Some("rate")) => Text::EffectFrameRate,
        ("echo_glow", None) => Text::EffectEchoGlow,
        ("echo_glow", Some("threshold")) => Text::EffectThreshold,
        ("echo_glow", Some("echoes")) => Text::EffectEchoes,
        ("echo_glow", Some("radius")) => Text::EffectRadius,
        ("echo_glow", Some("spread")) => Text::EffectSpread,
        ("echo_glow", Some("intensity")) => Text::EffectIntensity,
        ("echo_glow", Some("decay")) => Text::EffectDecay,
        ("echo_glow", Some("color")) => Text::EffectGlowColour,
        ("edge_glow", None) => Text::EffectEdgeGlow,
        ("edge_glow", Some("threshold")) => Text::EffectEdgeThreshold,
        ("edge_glow", Some("width")) => Text::EffectEdgeWidth,
        ("edge_glow", Some("radius")) => Text::EffectRadius,
        ("edge_glow", Some("intensity")) => Text::EffectIntensity,
        ("edge_glow", Some("color")) => Text::EffectGlowColour,
        ("edge_glow", Some("only")) => Text::EffectOnly,
        ("glint", None) => Text::EffectSparkle,
        ("glint", Some("threshold")) => Text::EffectThreshold,
        ("glint", Some("rays")) => Text::EffectRays,
        ("glint", Some("length")) => Text::EffectLength,
        ("glint", Some("rotation")) => Text::EffectRotation,
        ("glint", Some("intensity")) => Text::EffectIntensity,
        ("glint", Some("color")) => Text::EffectColour,
        ("glint", Some("colorize")) => Text::EffectColorize,
        ("wonder_glow", None) => Text::EffectBloom,
        ("wonder_glow", Some("threshold")) => Text::EffectThreshold,
        ("wonder_glow", Some("radius")) => Text::EffectRadius,
        ("wonder_glow", Some("intensity")) => Text::EffectIntensity,
        ("wonder_glow", Some("saturation")) => Text::Saturation,
        ("wonder_glow", Some("use_color")) => Text::EffectUseColour,
        ("wonder_glow", Some("color")) => Text::EffectGlowColour,
        ("wonder_glow", Some("mode")) => Text::EffectBlendMode,
        ("volumetric_rays", None) => Text::EffectSunRays,
        ("volumetric_rays", Some("center")) => Text::EffectCentre,
        ("volumetric_rays", Some("threshold")) => Text::EffectThreshold,
        ("volumetric_rays", Some("length")) => Text::EffectLength,
        ("volumetric_rays", Some("intensity")) => Text::EffectIntensity,
        ("volumetric_rays", Some("color")) => Text::EffectGlowColour,
        ("volumetric_rays", Some("only")) => Text::EffectOnly,
        ("light_leaks", None) => Text::EffectLightLeaks,
        ("light_leaks", Some("c1")) => Text::EffectColour1,
        ("light_leaks", Some("c2")) => Text::EffectColour2,
        ("light_leaks", Some("intensity")) => Text::EffectIntensity,
        ("light_leaks", Some("scale")) => Text::EffectScale,
        ("light_leaks", Some("direction")) => Text::EffectDirection,
        ("light_leaks", Some("speed")) => Text::Speed,
        ("light_leaks", Some("seed")) => Text::EffectSeed,
        ("light_leaks", Some("mode")) => Text::EffectBlendMode,
        ("lens_flare", None) => Text::EffectLensFlare,
        ("lens_flare", Some("center")) => Text::EffectCentre,
        ("lens_flare", Some("brightness")) => Text::Brightness,
        ("lens_flare", Some("blend")) => Text::EffectBlendOriginal,
        ("rgb_split", None) => Text::EffectColourSplit,
        ("rgb_split", Some("mode")) => Text::EffectMode,
        ("rgb_split", Some("amount")) => Text::EffectAmount,
        ("rgb_split", Some("angle")) => Text::EffectAngle,
        ("rgb_split", Some("center")) => Text::EffectCentre,
        ("rgb_split", Some("blend")) => Text::EffectBlendOriginal,
        ("drop_shadow", None) => Text::EffectDropShadow,
        ("drop_shadow", Some("color")) => Text::EffectShadowColour,
        ("drop_shadow", Some("opacity")) => Text::EffectOpacity,
        ("drop_shadow", Some("direction")) => Text::EffectDirection,
        ("drop_shadow", Some("distance")) => Text::EffectDistance,
        ("drop_shadow", Some("softness")) => Text::EffectSoftness,
        ("drop_shadow", Some("only")) => Text::EffectOnly,
        ("long_shadow", None) => Text::EffectLongShadow,
        ("long_shadow", Some("angle")) => Text::EffectAngle,
        ("long_shadow", Some("length")) => Text::EffectLength,
        ("long_shadow", Some("color")) => Text::EffectShadowColour,
        ("long_shadow", Some("opacity")) => Text::EffectOpacity,
        ("long_shadow", Some("fade")) => Text::EffectFade,
        ("long_shadow", Some("only")) => Text::EffectOnly,
        ("gradient", None) => Text::EffectGradient,
        ("gradient", Some("start")) => Text::EffectGradientStart,
        ("gradient", Some("start_color")) => Text::EffectStartColour,
        ("gradient", Some("end")) => Text::EffectGradientEnd,
        ("gradient", Some("end_color")) => Text::EffectEndColour,
        ("gradient", Some("shape")) => Text::EffectGradientShape,
        ("four_color_gradient", None) => Text::EffectFourCornerGradient,
        ("four_color_gradient", Some("p1")) => Text::EffectPoint1,
        ("four_color_gradient", Some("c1")) => Text::EffectColour1,
        ("four_color_gradient", Some("p2")) => Text::EffectPoint2,
        ("four_color_gradient", Some("c2")) => Text::EffectColour2,
        ("four_color_gradient", Some("p3")) => Text::EffectPoint3,
        ("four_color_gradient", Some("c3")) => Text::EffectColour3,
        ("four_color_gradient", Some("p4")) => Text::EffectPoint4,
        ("four_color_gradient", Some("c4")) => Text::EffectColour4,
        ("color_key", None) => Text::EffectColourKey,
        ("color_key", Some("color")) => Text::EffectColour,
        ("color_key", Some("tolerance")) => Text::EffectTolerance,
        ("color_key", Some("thin")) => Text::EffectEdgeThin,
        ("color_key", Some("feather")) => Text::EffectEdgeFeather,
        ("luma_key", None) => Text::EffectLumaKey,
        ("luma_key", Some("threshold")) => Text::EffectThreshold,
        ("luma_key", Some("cutoff")) => Text::EffectCutoff,
        ("luma_key", Some("invert")) => Text::EffectKeyBrights,
        _ => return None,
    })
}

/// Creator labels for transition kinds, shared by the catalogue, timeline and
/// Properties; unknown kinds read as the catalogue's own name.
pub fn transition_label(kind: &str) -> Text {
    match kind {
        "cross_dissolve" => Text::TransitionCrossDissolve,
        "dip_to_black" => Text::TransitionDipBlack,
        "dip_to_white" => Text::TransitionDipWhite,
        "dip_to_color" => Text::TransitionDipColour,
        "light_fade" => Text::TransitionLightFade,
        "film_fade" => Text::TransitionFilmFade,
        "soft_focus_fade" => Text::TransitionSoftFocusFade,
        "burn_through" => Text::TransitionBurnThrough,
        "brightness_fade" => Text::TransitionBrightnessFade,
        "pixel_blocks" => Text::TransitionPixelBlocks,
        "straight_wipe" => Text::TransitionStraight,
        "soft_edge_wipe" => Text::TransitionSoftEdge,
        "clock_wipe" => Text::TransitionClock,
        "corner_sweep" => Text::TransitionCornerSweep,
        "star_wipe" => Text::TransitionStar,
        "panel_wipe" => Text::TransitionPanel,
        "stair_wipe" => Text::TransitionStair,
        "stretch_wipe" => Text::TransitionStretch,
        "neon_edge_wipe" => Text::TransitionNeonEdge,
        "circle_reveal" => Text::TransitionCircleReveal,
        "box_reveal" => Text::TransitionBoxReveal,
        "diamond_reveal" => Text::TransitionDiamondReveal,
        "cross_reveal" => Text::TransitionCrossReveal,
        "double_doors" => Text::TransitionDoubleDoors,
        "corner_inset" => Text::TransitionCornerInset,
        "zoom_cross" => Text::TransitionZoomCross,
        "zoom_smear" => Text::TransitionZoomSmear,
        "spin_smear" => Text::TransitionSpinSmear,
        "streak_smear" => Text::TransitionStreakSmear,
        "lens_defocus" => Text::TransitionLensDefocus,
        "bloom" => Text::TransitionBloom,
        "flash" => Text::TransitionFlash,
        "lens_flare" => Text::TransitionLensFlare,
        "light_leak" => Text::TransitionLightLeak,
        "colour_leak" => Text::TransitionColourLeak,
        "colour_burn" => Text::TransitionColourBurn,
        "light_sweep" => Text::TransitionLightSweep,
        "light_rays" => Text::TransitionLightRays,
        "phosphor_trail" => Text::TransitionPhosphorTrail,
        "solarise" => Text::TransitionSolarise,
        "stripes" => Text::TransitionStripes,
        "scramble" => Text::TransitionScramble,
        "digital_glitch" => Text::TransitionDigitalGlitch,
        "tape_damage" => Text::TransitionTapeDamage,
        "flicker_cut" => Text::TransitionFlickerCut,
        "quake" => Text::TransitionQuake,
        "glass_shards" => Text::TransitionGlassShards,
        "rough_edge" => Text::TransitionRoughEdge,
        "kaleidoscope" => Text::TransitionKaleidoscope,
        "liquid_warp" => Text::TransitionLiquidWarp,
        "screen_off" => Text::TransitionScreenOff,
        "push" => Text::TransitionPush,
        "slide" => Text::TransitionSlide,
        "roll" => Text::TransitionRoll,
        "split" => Text::TransitionSplitOpen,
        "stretch" => Text::TransitionStretchIn,
        "film_roll" => Text::TransitionFilmStrip,
        "spring_motion" => Text::TransitionSpring,
        "wave" => Text::TransitionWave,
        "slice" => Text::TransitionSlices,
        "louver" => Text::TransitionBlinds,
        "frame" => Text::TransitionPictureFrame,
        "roll_3d" => Text::TransitionCylinderRoll,
        "flip_motion" => Text::TransitionCardFlip,
        "fold_motion" => Text::TransitionFoldOpen,
        "spin_motion" => Text::TransitionSpin,
        "spin_3d" => Text::TransitionTurntable,
        "spinback_3d" => Text::TransitionTurntableBack,
        "page_peel" => Text::TransitionPagePeel,
        "mirror_transition" => Text::TransitionMirrorFlip,
        "block_motion" => Text::TransitionTileSlide,
        "whip" => Text::TransitionWhipPan,
        "motion_camera" => Text::TransitionCameraMove,
        "travel_motion" => Text::TransitionZoomTravel,
        "motion_tween" => Text::TransitionMatchScale,
        "pull_motion" => Text::TransitionPull,
        "pop_motion" => Text::TransitionPop,
        "shape_dissolve" => Text::TransitionShapeDissolve,
        "shape_flow" => Text::TransitionShapeFlow,
        _ => Text::Transitions,
    }
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

/// The label of one clip fit mode.
pub fn fit_label(mode: project::FitMode) -> Text {
    match mode {
        project::FitMode::Fit => Text::FitModeFit,
        project::FitMode::Fill => Text::FitModeFill,
        project::FitMode::Stretch => Text::FitModeStretch,
        project::FitMode::None => Text::FitModeOriginal,
        project::FitMode::Manual => Text::FitModeManual,
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

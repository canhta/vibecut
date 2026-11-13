//! Owns the properties gesture drafts a drag or typed edit holds.
use crate::editor::{ItemId, Text};

/// One scrub field's unit contract. Pixels per step are recorded in the app
/// README; the inventory leaves the exact sensitivity unverified.
#[derive(Clone, Copy, Debug)]
pub(super) struct ScrubSpec {
    pub(super) field: &'static str,
    pub(super) label: Text,
    pub(super) min: f64,
    pub(super) max: f64,
    pub(super) step: f64,
    pub(super) pixels_per_step: f32,
    pub(super) decimals: usize,
    pub(super) suffix: &'static str,
    /// Shown value = model value × scale, in whole units, with `suffix`. Opacity and the
    /// scale fields use 100 for 0–100 %. The model value, step and drag stay unscaled.
    pub(super) scale: f64,
}
impl ScrubSpec {
    pub(in crate::editor) fn quantize(self, value: f64) -> f64 {
        if !value.is_finite() {
            return self.min;
        }
        let steps = ((value.clamp(self.min, self.max) - self.min) / self.step).round();
        (self.min + steps * self.step).clamp(self.min, self.max)
    }
    pub(in crate::editor) fn format(self, value: f64) -> String {
        format!("{:.*}", self.decimals, self.quantize(value))
    }
}
pub(super) const OPACITY_SCRUB: ScrubSpec = ScrubSpec {
    field: project::BuiltinPath::Opacity.as_str(),
    label: Text::Opacity,
    min: 0.0,
    max: 1.0,
    step: 0.01,
    pixels_per_step: 4.0,
    decimals: 2,
    suffix: "%",
    scale: 100.0,
};
pub(super) const TEXT_SIZE_SCRUB: ScrubSpec = ScrubSpec {
    field: "text_size",
    label: Text::TextSize,
    min: 1.0,
    max: 4096.0,
    step: 1.0,
    pixels_per_step: 2.0,
    decimals: 0,
    suffix: "",
    scale: 1.0,
};
pub(super) const SPEED_SCRUB: ScrubSpec = ScrubSpec {
    field: "speed",
    label: Text::Speed,
    min: 0.01,
    max: 5.0,
    step: 0.01,
    pixels_per_step: 2.0,
    decimals: 2,
    suffix: "x",
    scale: 1.0,
};
/// Transition duration, 0.1-5 s as the reference's field; the engine further
/// clamps it to the handles of both neighbouring clips.
pub(super) const TRANSITION_DURATION_SCRUB: ScrubSpec = ScrubSpec {
    field: "transition_duration",
    label: Text::TransitionDuration,
    min: 0.1,
    max: 5.0,
    step: 0.1,
    pixels_per_step: 8.0,
    decimals: 1,
    suffix: "s",
    scale: 1.0,
};
pub(super) const AUDIO_GAIN_SCRUB: ScrubSpec = ScrubSpec {
    field: "audio_gain",
    label: Text::AudioGain,
    min: -60.0,
    max: 20.0,
    step: 0.01,
    pixels_per_step: 4.0,
    decimals: 2,
    suffix: "dB",
    scale: 1.0,
};
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum ScrubTarget {
    Item(&'static str),
    Effect {
        effect: String,
        key: String,
        inspector: bool,
    },
}
impl ScrubTarget {
    pub(in crate::editor) fn draft_key(&self) -> String {
        match self {
            Self::Item(field) => (*field).to_owned(),
            Self::Effect { effect, key, .. } => format!("effects.{effect}.param.{key}"),
        }
    }
    pub(super) fn field_id(&self) -> String {
        match self {
            Self::Item(field) => format!("inspector.{field}"),
            Self::Effect {
                inspector: true, ..
            } => "inspector.lut.strength".into(),
            Self::Effect { effect, key, .. } => format!("effects.{effect}.param.{key}"),
        }
    }
    pub(in crate::editor) fn widget_id(&self) -> String {
        format!("{}.scrub", self.field_id())
    }
}
#[derive(Clone, Debug)]
pub(super) struct ScrubDrag {
    pub(super) item: ItemId,
    pub(super) target: ScrubTarget,
    pub(super) start: f64,
    pub(super) last: f64,
}
/// The effect row whose drag handle currently holds a reorder.
#[derive(Clone, Debug)]
pub(in crate::editor) struct EffectReorder {
    pub(in crate::editor) item: ItemId,
    pub(in crate::editor) effect: String,
}

/// One open wheel or curve gesture; `key` names the wheel or curve channel.
#[derive(Clone, Debug)]
pub(super) struct GraphDrag {
    pub(super) item: ItemId,
    pub(super) effect: String,
    pub(super) key: String,
}

/// The scrub contract of one effect parameter, from its admitted range.
pub(super) fn effect_scrub_spec(parameter: &project::EffectParamDefinition) -> ScrubSpec {
    let (min, max) = match parameter.kind {
        project::EffectParamKind::Float {
            soft_min, soft_max, ..
        } => (soft_min, soft_max),
        _ => parameter.scalar_range().unwrap_or((0.0, 1.0)),
    };
    let step = if max - min <= 1.0 { 0.01 } else { 1.0 };
    ScrubSpec {
        field: "effect_param",
        label: Text::EffectIntensity,
        min,
        max,
        step,
        pixels_per_step: 2.0,
        decimals: if step < 1.0 { 2 } else { 0 },
        suffix: "",
        scale: 1.0,
    }
}

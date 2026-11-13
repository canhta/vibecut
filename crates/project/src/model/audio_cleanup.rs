//! Typed, bounded audio cleanup chain of one audio or video item, validated on admission.
use crate::{Error, Result};
use serde::{Deserialize, Serialize};

/// Most effects one item's cleanup chain holds.
pub const MAX_CLEANUP_EFFECTS: usize = 8;
/// Most bands one parametric equaliser holds.
pub const MAX_EQ_BANDS: usize = 8;
/// Most harmonics one hum removal covers.
pub const MAX_HUM_HARMONICS: u8 = 10;
/// Graphic equaliser centre frequencies in Hz, octave spaced.
pub const GRAPHIC_EQ_FREQUENCIES_HZ: [f32; 10] = [
    31.5, 63.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0,
];

/// The ordered cleanup chain of an item. The source media is never changed.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AudioCleanup {
    pub effects: Vec<CleanupEffect>,
}

/// One cleanup effect with a stable id unique within its item.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CleanupEffect {
    pub id: u32,
    pub enabled: bool,
    /// Dry/wet mix in percent: 0 passes the source through, 100 keeps only the processed signal.
    pub wet_percent: f32,
    pub kind: CleanupKind,
    /// The preset that added this effect, if any. A marker only: the effect is edited, bypassed
    /// and removed like any other.
    pub preset: Option<CleanupPreset>,
}

/// A named group of cleanup effects applied together.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CleanupPreset {
    /// The one-click voice enhancement chain, see [`voice_enhancement_effects`].
    VoiceEnhancement,
}

/// Cleanup effect kinds and their parameters.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CleanupKind {
    /// Notch stages at the mains frequency and its harmonics.
    Hum {
        mains: MainsFrequency,
        /// 1 to `MAX_HUM_HARMONICS`.
        harmonics: u8,
        /// Notch quality, 1 to 100.
        q: f32,
    },
    /// Second-order high-pass at the cutoff, Butterworth Q.
    Highpass { cutoff_hz: f32 },
    /// Second-order low-pass at the cutoff, Butterworth Q.
    Lowpass { cutoff_hz: f32 },
    /// Low shelf at 200 Hz.
    Bass { boost_db: f32 },
    /// High shelf at 4000 Hz.
    Treble { boost_db: f32 },
    /// Ten octave-spaced peaking bands at `GRAPHIC_EQ_FREQUENCIES_HZ`.
    GraphicEq { gains_db: [f32; 10] },
    /// Up to `MAX_EQ_BANDS` bands applied in order.
    ParametricEq { bands: Vec<EqBand> },
    /// Broadband noise reduction by spectral gating. The noise floor of each frequency bin is
    /// tracked continuously, so there is no learning step.
    NoiseReduction {
        /// How far gated bins are attenuated, in dB: 0 to 40.
        reduction_db: f32,
        /// How far a bin must rise above the tracked floor to pass, in dB: 0 to 20.
        threshold_db: f32,
        /// How slowly the gate opens and closes, in percent: 0 to 100.
        smoothing_percent: f32,
    },
    /// Late-reverberation suppression: the reverberant tail predicted from an exponential decay
    /// model is removed by spectral subtraction.
    Dereverb {
        /// Over-subtraction and gain floor, in percent: 0 to 100.
        amount_percent: f32,
        /// Decay time of the room to remove, in seconds: 0.1 to 5.
        rt60_seconds: f32,
    },
    /// De-esser: a dynamic high shelf cuts the band above `frequency_hz` while it stands out
    /// from the broadband level.
    Deess {
        /// Sibilance band start and shelf corner, in Hz: 2000 to 12000.
        frequency_hz: f32,
        /// How far the band may stand above the broadband level before it is cut, in dB:
        /// -40 to 0.
        threshold_db: f32,
        /// Deepest cut, in dB: 0 to 24.
        reduction_db: f32,
    },
    /// Click removal: outliers of a linear-prediction residual are replaced by interpolation.
    Declick {
        /// Detection threshold, 1 to 100: higher finds fewer, stronger clicks.
        threshold: f32,
        /// Model order and longest repaired click, 1 to 100.
        complexity: f32,
    },
}

/// Noise reduction values a plain on/off switch applies.
pub const NOISE_REDUCTION_DEFAULT: CleanupKind = CleanupKind::NoiseReduction {
    reduction_db: 12.0,
    threshold_db: 6.0,
    smoothing_percent: 50.0,
};

/// Number of effects the voice enhancement preset adds.
pub const VOICE_ENHANCEMENT_EFFECTS: usize = 5;

/// The one-click voice enhancement preset, with ids `first_id` onward: an 80 Hz high-pass
/// against rumble, de-reverberation, spectral noise reduction at `NOISE_REDUCTION_DEFAULT`
/// strength, a de-esser, then a parametric equaliser with a 2.5 kHz presence boost and a
/// 10 kHz air shelf (the structure reference's SpeechEnhance bands for a low voice, without its
/// 250 Hz cut, which took the voice's own low harmonics: it lowered the measured speech-to-noise
/// ratio by 1.2 dB).
/// De-reverberation and noise reduction run before the boosts so they cannot lift what those
/// stages remove. Every effect is an ordinary chain effect carrying
/// [`CleanupPreset::VoiceEnhancement`] as its marker: tune, disable or remove any of them with
/// the `audio.*_cleanup_effect` commands. The preset has no gain stage, so the project's loudness
/// normalizer, which measures the chain's output, never levels twice.
pub fn voice_enhancement_effects(first_id: u32) -> [CleanupEffect; VOICE_ENHANCEMENT_EFFECTS] {
    let band = |kind, frequency_hz, gain_db, q| EqBand {
        kind,
        enabled: true,
        frequency_hz,
        gain_db,
        q,
    };
    let effect = |offset: u32, kind| CleanupEffect {
        id: first_id + offset,
        enabled: true,
        wet_percent: 100.0,
        kind,
        preset: Some(CleanupPreset::VoiceEnhancement),
    };
    [
        effect(0, CleanupKind::Highpass { cutoff_hz: 80.0 }),
        effect(
            1,
            CleanupKind::Dereverb {
                amount_percent: 40.0,
                rt60_seconds: 0.3,
            },
        ),
        effect(2, NOISE_REDUCTION_DEFAULT),
        effect(
            3,
            CleanupKind::Deess {
                frequency_hz: 6000.0,
                threshold_db: -12.0,
                reduction_db: 8.0,
            },
        ),
        effect(
            4,
            CleanupKind::ParametricEq {
                bands: vec![
                    band(EqBandKind::Peaking, 2500.0, 4.0, 1.0),
                    band(
                        EqBandKind::HighShelf,
                        10_000.0,
                        2.0,
                        std::f32::consts::FRAC_1_SQRT_2,
                    ),
                ],
            },
        ),
    ]
}

/// Mains supply frequency of a hum removal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MainsFrequency {
    Hz50,
    Hz60,
}

impl MainsFrequency {
    pub fn hertz(self) -> f32 {
        match self {
            Self::Hz50 => 50.0,
            Self::Hz60 => 60.0,
        }
    }
}

/// One parametric band.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EqBand {
    pub kind: EqBandKind,
    pub enabled: bool,
    pub frequency_hz: f32,
    pub gain_db: f32,
    pub q: f32,
}

/// Shape of a parametric band.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EqBandKind {
    Peaking,
    LowShelf,
    HighShelf,
    Notch,
}

fn finite_in(value: f32, min: f32, max: f32) -> bool {
    value.is_finite() && (min..=max).contains(&value)
}

impl CleanupEffect {
    fn validate(&self) -> Result<()> {
        if !finite_in(self.wet_percent, 0.0, 100.0) {
            return Err(Error::Invalid("cleanup mix"));
        }
        match &self.kind {
            CleanupKind::Hum { harmonics, q, .. } => {
                if !(1..=MAX_HUM_HARMONICS).contains(harmonics) || !finite_in(*q, 1.0, 100.0) {
                    return Err(Error::Invalid("hum parameters"));
                }
            }
            CleanupKind::Highpass { cutoff_hz } | CleanupKind::Lowpass { cutoff_hz } => {
                if !finite_in(*cutoff_hz, 20.0, 20_000.0) {
                    return Err(Error::Invalid("filter cutoff"));
                }
            }
            CleanupKind::Bass { boost_db } | CleanupKind::Treble { boost_db } => {
                if !finite_in(*boost_db, -24.0, 24.0) {
                    return Err(Error::Invalid("shelf boost"));
                }
            }
            CleanupKind::GraphicEq { gains_db } => {
                if gains_db.iter().any(|g| !finite_in(*g, -24.0, 24.0)) {
                    return Err(Error::Invalid("graphic eq gain"));
                }
            }
            CleanupKind::ParametricEq { bands } => {
                if bands.len() > MAX_EQ_BANDS {
                    return Err(Error::Invalid("parametric eq bands"));
                }
                for band in bands {
                    if !finite_in(band.frequency_hz, 20.0, 20_000.0)
                        || !finite_in(band.gain_db, -30.0, 30.0)
                        || !finite_in(band.q, 0.1, 30.0)
                    {
                        return Err(Error::Invalid("parametric eq band"));
                    }
                }
            }
            CleanupKind::NoiseReduction {
                reduction_db,
                threshold_db,
                smoothing_percent,
            } => {
                if !finite_in(*reduction_db, 0.0, 40.0)
                    || !finite_in(*threshold_db, 0.0, 20.0)
                    || !finite_in(*smoothing_percent, 0.0, 100.0)
                {
                    return Err(Error::Invalid("noise reduction parameters"));
                }
            }
            CleanupKind::Dereverb {
                amount_percent,
                rt60_seconds,
            } => {
                if !finite_in(*amount_percent, 0.0, 100.0) || !finite_in(*rt60_seconds, 0.1, 5.0) {
                    return Err(Error::Invalid("dereverb parameters"));
                }
            }
            CleanupKind::Deess {
                frequency_hz,
                threshold_db,
                reduction_db,
            } => {
                if !finite_in(*frequency_hz, 2000.0, 12_000.0)
                    || !finite_in(*threshold_db, -40.0, 0.0)
                    || !finite_in(*reduction_db, 0.0, 24.0)
                {
                    return Err(Error::Invalid("deess parameters"));
                }
            }
            CleanupKind::Declick {
                threshold,
                complexity,
            } => {
                if !finite_in(*threshold, 1.0, 100.0) || !finite_in(*complexity, 1.0, 100.0) {
                    return Err(Error::Invalid("declick parameters"));
                }
            }
        }
        Ok(())
    }
}

impl AudioCleanup {
    /// Refuses a chain with too many effects, duplicate ids or out-of-range parameters.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Invalid`] naming the first refused field.
    pub fn validate(&self) -> Result<()> {
        if self.effects.len() > MAX_CLEANUP_EFFECTS {
            return Err(Error::Invalid("cleanup effect count"));
        }
        for (index, effect) in self.effects.iter().enumerate() {
            if self
                .effects
                .iter()
                .take(index)
                .any(|other| other.id == effect.id)
            {
                return Err(Error::Invalid("cleanup effect id"));
            }
            effect.validate()?;
        }
        Ok(())
    }

    /// True when no enabled effect would change the source, so the chain is skipped.
    pub fn is_bypassed(&self) -> bool {
        self.effects
            .iter()
            .all(|effect| !effect.enabled || effect.wet_percent <= 0.0)
    }
}

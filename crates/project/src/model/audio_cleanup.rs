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

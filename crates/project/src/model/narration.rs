//! Narration provenance: the provider, voice, text fingerprint and consent of a generated take.
use super::project::text;
use crate::{Error, Result};
use captions::NfcText;
use serde::{Deserialize, Serialize};
use time::Tick;

/// Longest ducking attack or release, in milliseconds.
pub const MAX_DUCK_MS: u32 = 5_000;
/// Deepest ducking, in tenths of a decibel.
pub const MAX_DUCK_TENTHS_DB: u16 = 600;
/// ElevenLabs `voice_settings.speed` bounds, in thousandths.
pub const PROVIDER_SPEED_THOUSANDTHS: std::ops::RangeInclusive<u16> = 700..=1200;

/// The upper-bound price the user approved for one take.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NarrationCost {
    pub currency: NfcText,
    /// Currency millionths.
    pub micros: u64,
}

/// What the user saw and approved before the text left the machine. Descriptive only:
/// it never authorizes a later call.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NarrationConsent {
    pub policy_url: NfcText,
    pub retention: NfcText,
    pub training: NfcText,
    pub voice_disclosure: NfcText,
    /// `None` when the price was unknown and acknowledged as such.
    pub upper_bound: Option<NarrationCost>,
}

/// Automation of the original audio while a take plays. Depth is positive attenuation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NarrationDucking {
    pub enabled: bool,
    /// Attenuation in tenths of a decibel (`120` is the default -12 dB).
    pub depth_tenths_db: u16,
    pub attack_ms: u32,
    pub release_ms: u32,
}
impl NarrationDucking {
    /// -12 dB, 120 ms attack, 300 ms release.
    pub const DEFAULT: Self = Self {
        enabled: true,
        depth_tenths_db: 120,
        attack_ms: 120,
        release_ms: 300,
    };
    fn validate(&self) -> Result<()> {
        if self.depth_tenths_db > MAX_DUCK_TENTHS_DB
            || self.attack_ms > MAX_DUCK_MS
            || self.release_ms > MAX_DUCK_MS
        {
            return Err(Error::Invalid("narration ducking"));
        }
        Ok(())
    }
}
impl Default for NarrationDucking {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Provenance carried with an audio item generated from text. The take itself is an
/// ordinary audio asset; this record keeps who produced it and what was approved.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Narration {
    pub provider: NfcText,
    pub model: NfcText,
    pub voice: NfcText,
    pub language: NfcText,
    /// ISO 15924 script of the request.
    pub script: NfcText,
    /// ElevenLabs `voice_settings.speed` in thousandths, when it was part of the request.
    pub provider_speed_thousandths: Option<u16>,
    /// The adapter fingerprint: model, voice, language and settings.
    pub fingerprint: NfcText,
    /// Lower-case hex SHA-256 of the NFC text that was synthesized.
    pub text_sha256: NfcText,
    /// Extended grapheme clusters of that text.
    pub characters: u64,
    pub consent: NarrationConsent,
    /// The output time the cue allowed; the take may overflow it only within the fit limit.
    pub cue_duration: Tick,
    pub ducking: NarrationDucking,
}
impl Narration {
    /// Admit provenance before it is stored in a project. No IO.
    ///
    /// # Errors
    ///
    /// Returns `Invalid` for blank or oversized text, a malformed hash, an out-of-range
    /// provider speed or ducking, or a non-positive cue duration.
    pub fn validate(&self) -> Result<()> {
        for value in [
            &self.provider,
            &self.model,
            &self.voice,
            &self.language,
            &self.script,
            &self.fingerprint,
            &self.consent.policy_url,
            &self.consent.retention,
            &self.consent.training,
            &self.consent.voice_disclosure,
        ] {
            text(value)?;
            if value.as_str().trim().is_empty() || value.as_str().len() > 4096 {
                return Err(Error::Invalid("narration text"));
            }
        }
        let hash = self.text_sha256.as_str();
        if hash.len() != 64 || !hash.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
            return Err(Error::Invalid("narration text hash"));
        }
        if self.characters == 0 {
            return Err(Error::Invalid("narration characters"));
        }
        if self
            .provider_speed_thousandths
            .is_some_and(|speed| !PROVIDER_SPEED_THOUSANDTHS.contains(&speed))
        {
            return Err(Error::Invalid("narration provider speed"));
        }
        if let Some(cost) = &self.consent.upper_bound {
            text(&cost.currency)?;
            if cost.currency.as_str().is_empty() {
                return Err(Error::Invalid("narration cost"));
            }
        }
        if self.cue_duration <= Tick::ZERO {
            return Err(Error::Invalid("narration cue duration"));
        }
        self.ducking.validate()
    }
}

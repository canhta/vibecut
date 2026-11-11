//! Interprets retained caption workflow results.
use crate::session::wire::{CaptionsResult, CostView, TranslationsResult};
use crate::{Command, EditError, NewItem, Placement, Result};
use captions::CueTrack;
use time::Tick;

/// Largest retained output of either job; also the job's result reservation.
pub(crate) const MAX_RESULT_BYTES: usize = 8 << 20;
/// Most cues one caption item holds, as `translate_selected` and the desktop insert bound.
pub(crate) const MAX_CUES: usize = crate::session::wire::MAX_TRANSLATION_CUES;

fn bounded(bytes: &[u8]) -> Result<&[u8]> {
    if bytes.len() > MAX_RESULT_BYTES {
        return Err(EditError::Control(
            crate::session::wire::ControlError::Limit,
        ));
    }
    Ok(bytes)
}

/// Decode the retained bytes of a finished `captions.generate` job.
pub fn decode_captions(bytes: &[u8]) -> Result<CaptionsResult> {
    serde_json::from_slice(bounded(bytes)?)
        .map_err(|_| EditError::Invalid("transcription output is malformed"))
}

/// Decode the retained bytes of a finished `captions.translate` job.
pub fn decode_translations(bytes: &[u8]) -> Result<TranslationsResult> {
    serde_json::from_slice(bounded(bytes)?)
        .map_err(|_| EditError::Invalid("translation output is malformed"))
}

/// The ordinary insert that `captions.apply` runs: one caption item over the
/// transcribed range, its cues clipped to that range and placed automatically.
pub(crate) fn apply_command(result: CaptionsResult, start: Tick) -> Result<Command> {
    let cues = captions::clip_track(
        &CueTrack {
            styles: Vec::new(),
            cues: result.cues,
        },
        result.range,
    )?;
    if cues.cues.is_empty() || cues.cues.len() > MAX_CUES {
        return Err(EditError::Control(
            crate::session::wire::ControlError::Limit,
        ));
    }
    Ok(Command::InsertItem {
        placement: Placement::Auto,
        item: NewItem {
            retime: Default::default(),
            animations: Default::default(),
            source_asset: Some(result.asset),
            source_range: result.range,
            start,
            content: project::ItemParams::Caption(project::CaptionParams { cues }),
        },
    })
}

impl From<&crate::providers::Cost> for CostView {
    fn from(cost: &crate::providers::Cost) -> Self {
        match cost {
            crate::providers::Cost::Free => Self::Free,
            crate::providers::Cost::UpperBound { currency, micros } => Self::UpperBound {
                currency: currency.clone(),
                micros: *micros,
            },
            crate::providers::Cost::Unknown => Self::Unknown,
        }
    }
}

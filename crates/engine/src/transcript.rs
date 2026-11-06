//! Acquires source-timed cues from the user's chosen providers.
mod checkpoints;
mod cue_translation;
mod cues;
mod operation;
mod speech;
mod translation_run;
pub(crate) use checkpoints::{hex, wait_for_job};
pub use cue_translation::{
    SubtitleTranslator, TRANSLATE_ATTEMPTS, TranslationSelection, translate_selected,
};
pub(crate) use cue_translation::{translate_cues, translate_track};
pub(crate) use cues::{build_session, caption_item_id};
pub use operation::{Cancellation, Progress, SilentProgress, Stage, Transcription};
pub(crate) use speech::media_duration;
pub use speech::transcribe;
pub mod translation {
    //! Applies selected translations through bounded attempts.
    pub use super::translation_run::{TranslationRun, TranslationTarget, window};
}

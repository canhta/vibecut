//! Acquires source-timed cues from the user's chosen providers.
mod checkpoints;
mod cue_translation;
mod cues;
mod error;
mod operation;
mod speech;
mod translation_run;
pub(crate) use checkpoints::hex;
pub use checkpoints::wait_for_job;
pub(crate) use cue_translation::translate_cues;
pub use cue_translation::translate_track;
pub use cue_translation::{
    SubtitleTranslator, TRANSLATE_ATTEMPTS, TranslationSelection, translate_selected,
};
pub use cues::build_session;
pub(crate) use cues::caption_item_id;
pub use error::Error;
pub use operation::{Cancellation, Progress, SilentProgress, Stage, Transcription};
pub(crate) use speech::media_duration;
pub use speech::{RESULT_BYTES, WORK_BYTES, transcribe};
pub(crate) use translation_run::translation_reference;
pub mod translation {
    //! Applies selected translations through bounded attempts.
    pub use super::translation_run::{TranslationRun, TranslationTarget, window};
}

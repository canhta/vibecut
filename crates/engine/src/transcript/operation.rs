//! Holds the state one transcript acquisition shares with its caller.
use crate::EditError;
use crate::jobs::JobSnapshot;
use crate::providers::Cost;
use captions::LanguageTag;
use captions::{Cue, Transcript};
use std::{
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};
/// Operation-wide cooperative cancellation shared by extraction, every submitted
/// job and the later assembly/translation/export stages.
#[derive(Clone, Default)]
pub struct Cancellation {
    flag: Arc<AtomicBool>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Extract,
    Transcribe,
    Align,
    Assemble,
    Segment,
    SuggestClips,
    Snap,
    Translate,
    ExportSrt,
    ExportBurn,
}
/// Progress is presentation only; the engine owns job state and checkpoint truth.
pub trait Progress {
    fn stage(&self, stage: Stage);
    fn job(&self, snapshot: &JobSnapshot) {
        let _ = snapshot;
    }
}
pub struct SilentProgress;
/// Resolved speech recognition plus the source-timed cues shared by the subtitle
/// and clip workflows. No translation or file publication happens here.
pub struct Transcription {
    pub source_language: LanguageTag,
    pub transcript: Transcript,
    pub cues: Vec<Cue>,
    pub cost: Cost,
    pub checkpoints: usize,
    pub elapsed: Duration,
}
impl Cancellation {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn cancel(&self) {
        self.flag.store(true, std::sync::atomic::Ordering::Release);
    }
    pub fn cancelled(&self) -> bool {
        self.flag.load(std::sync::atomic::Ordering::Acquire)
    }
    pub(crate) fn media_flag(&self) -> Arc<AtomicBool> {
        self.flag.clone()
    }
    pub(crate) fn checkpoint(&self) -> Result<(), EditError> {
        if self.cancelled() {
            Err(EditError::Cancelled)
        } else {
            Ok(())
        }
    }
}
impl Progress for SilentProgress {
    fn stage(&self, _: Stage) {}
}

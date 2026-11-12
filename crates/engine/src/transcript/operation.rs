//! Holds the state one transcript acquisition shares with its caller.
use crate::jobs::JobSnapshot;
use crate::providers::Cost;
use crate::runtime::{Error as RuntimeError, Result as RuntimeResult};
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
    /// Share this operation's cancellation flag with bounded media decoding.
    pub fn media_flag(&self) -> Arc<AtomicBool> {
        self.flag.clone()
    }
    /// Refuse the next operation stage after cooperative cancellation.
    pub fn checkpoint(&self) -> RuntimeResult<()> {
        if self.cancelled() {
            Err(RuntimeError::Cancelled)
        } else {
            Ok(())
        }
    }
}
impl Progress for SilentProgress {
    fn stage(&self, _: Stage) {}
}

impl ai::ExecutionHooks for Cancellation {
    fn cancelled(&self) -> bool {
        self.cancelled()
    }
    fn stage(&self, _: ai::Stage) {}
}
impl Cancellation {
    /// Share a job's cancellation flag, so a workflow running inside the job observes `job_cancel`.
    pub fn from_flag(flag: Arc<AtomicBool>) -> Self {
        Self { flag }
    }
}
impl Progress for crate::jobs::JobContext {
    fn stage(&self, stage: Stage) {
        use crate::jobs::JobStage;
        let stage = match stage {
            Stage::Extract => JobStage::Extract,
            Stage::Transcribe => JobStage::Transcribe,
            Stage::Align => JobStage::Align,
            Stage::Assemble => JobStage::Assemble,
            Stage::Segment => JobStage::Segment,
            Stage::Translate => JobStage::Translate,
            Stage::SuggestClips => JobStage::SuggestClips,
            Stage::Snap => JobStage::Process,
            Stage::ExportSrt | Stage::ExportBurn => JobStage::Render,
        };
        self.progress(stage, 0.0).ok();
    }
}

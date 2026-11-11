//! Defines the transport-neutral wire contract of one session.
mod contract;
mod decode;
mod envelope;
mod json;
mod ledger;
mod limits;
mod results;
mod views;

pub use contract::{contract_schema, reference_markdown, registry_hash};
pub(crate) use decode::decode_command;
pub use envelope::{
    API_VERSION, ErrorCode, Event, Failure, JobFailurePolicy, Payload, Request, Response,
    ResponseValue, Revision,
};
pub use json::{bounded_json, parse_frame};
pub(crate) use json::{budget, close_schema, validate_wire};
pub use ledger::{Cursor, Snapshot};
pub(crate) use ledger::{Ledger, Receipt};
pub use limits::*;
pub use results::{ControlResult, JobOutput, JobView, ProviderView};
pub use views::{
    CaptionsResult, ChangeKind, ChapterSidecars, CostView, DetectionResult, FillerCandidate,
    FontEntry, FontLoad, FontLoadState, FontOrigin, FontState, JobId, JobKind, JobStage, JobState,
    LoudnessResult, Measured, OutputSpan, PauseCandidates, PreviewFrame, PreviewResult,
    SavedSoundEntry, SceneCandidate, SnapshotResult, SoundEntry, SoundPage, SoundSource,
    TranslationsResult,
};

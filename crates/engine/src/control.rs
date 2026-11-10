//! Admits transport-neutral commands and runs control-only commands, the top engine layer.
use crate::jobs::{JobId, JobRunner};
use crate::{Command, Result, Session};

pub use crate::session::wire::{
    API_VERSION, ControlResult, Cursor, ErrorCode, Event, Failure, FillerCandidate, FontEntry,
    FontLoad, FontLoadState, FontOrigin, FontState, JobOutput, JobView, LIMITS, Limits,
    MAX_BATCH_COMMANDS, MAX_BATCH_DEPTH, MAX_COLLECTION, MAX_DEPTH, MAX_EVENTS, MAX_FRAME_BYTES,
    MAX_KEY_BYTES, MAX_NODES, MAX_PATH_BYTES, MAX_PAUSE_MS, MAX_PAUSE_SPANS, MAX_PREVIEW_CLIPS,
    MAX_PREVIEW_EDGE, MAX_PREVIEW_FRAMES, MAX_RECEIPT_BYTES, MAX_RECEIPTS, MAX_RESPONSE_BYTES,
    MAX_SAFE_INTEGER, MAX_TEXT_BYTES, OutputSpan, PREVIEW_QUALITY, PauseCandidates, Payload,
    PreviewFrame, PreviewResult, ProviderView, RECEIPT_RESERVATION, Request, Response,
    ResponseValue, Revision, SavedSoundEntry, Snapshot, SoundEntry, SoundPage, SoundSource,
    bounded_json, contract_schema, parse_frame, reference_markdown, registry_hash,
};

mod admission;
mod dispatch;
mod media;
mod output;
mod receipts;
mod resync;
pub use media::MediaAccess;

/// Supplied by the trusted composition root, never deserialized from client JSON.
/// Implementations admit source reads, destination writes/overwrite and provider
/// consent through their owners. The engine calls this on every child before IO.
pub trait Policy {
    /// Trusted snapshot replacement grant; the request itself has no overwrite flag.
    /// Without a grant, staging and final publication both use atomic no-clobber.
    fn snapshot_overwrite(&self) -> bool {
        false
    }

    fn authorize(&self, command: &Command, project: &project::Project) -> Result<()>;
}
/// The three font operations the process lends to control commands. It is composition state
/// borrowed for one request, never project state: the kernel discovers no fonts, owns no store
/// and touches no network. Admission, grants, receipts and revision checks run before any
/// operation is invoked, and dispatch builds the response itself.
#[derive(Clone, Copy)]
pub struct FontAccess<'a> {
    list: &'a dyn Fn(Option<&str>) -> Result<Vec<FontEntry>>,
    load: &'a dyn Fn(&str) -> Result<FontLoad>,
    download: &'a dyn Fn(&mut JobRunner, &str, bool) -> Result<JobId>,
}
impl<'a> FontAccess<'a> {
    /// `list` serves `fonts.list`, `load` serves `fonts.load`, and `download` submits onto the
    /// supplied runner and returns only the accepted job; cancellation addresses the runner by id.
    pub fn new(
        list: &'a dyn Fn(Option<&str>) -> Result<Vec<FontEntry>>,
        load: &'a dyn Fn(&str) -> Result<FontLoad>,
        download: &'a dyn Fn(&mut JobRunner, &str, bool) -> Result<JobId>,
    ) -> Self {
        Self {
            list,
            load,
            download,
        }
    }
}
/// The three sound-library operations the process lends to control commands. The
/// access object owns the library root and the provider credential, so the command
/// carries neither; it chooses remote or local search by the credential's presence.
/// Admission, grants and revision checks run before any operation is invoked.
#[derive(Clone, Copy)]
pub struct SoundAccess<'a> {
    search: &'a dyn Fn(&SoundQuery) -> Result<SoundPage>,
    save: &'a dyn Fn(&SoundEntry) -> Result<SavedSoundEntry>,
    unsave: &'a dyn Fn(u64) -> Result<bool>,
}
impl<'a> SoundAccess<'a> {
    /// `search` serves `sounds.search` and degrades to the saved local library when
    /// the credential is absent, `save` serves `sounds.save`, and `unsave` serves
    /// `sounds.unsave`, returning whether an entry was removed.
    pub fn new(
        search: &'a dyn Fn(&SoundQuery) -> Result<SoundPage>,
        save: &'a dyn Fn(&SoundEntry) -> Result<SavedSoundEntry>,
        unsave: &'a dyn Fn(u64) -> Result<bool>,
    ) -> Self {
        Self {
            search,
            save,
            unsave,
        }
    }
}
/// One bounded sound-search request in the form the lent library takes.
#[derive(Clone, Debug)]
pub struct SoundQuery {
    pub query: String,
    pub page: u32,
    pub page_size: u32,
    pub commercial_only: bool,
}
pub struct Caller<'a> {
    pub principal: &'a str,
    pub external: bool,
    pub policy: &'a dyn Policy,
    /// The process font operations behind `fonts.list`, `fonts.load` and `fonts.download`.
    /// Without them those commands are refused with [`crate::FontError::Unavailable`].
    pub fonts: Option<&'a FontAccess<'a>>,
    /// The process sound library behind `sounds.search`, `sounds.save` and
    /// `sounds.unsave`. Without it those commands refuse `Unsupported`.
    pub sounds: Option<&'a SoundAccess<'a>>,
    /// The process artifact caches behind `media.generate_waveform` and
    /// `media.generate_clip_thumbnails`. Without them those commands refuse.
    /// One mutex instance is shared by the desktop timeline display and the
    /// in-process MCP inbox, so generation and display read the same entries.
    pub media: Option<&'a std::sync::Mutex<MediaAccess>>,
    /// The process transcript-text adapter and the human approval it consults.
    /// Without it `chapters.generate` and `fillers.classify` refuse with
    /// [`crate::providers::Error::ConsentRequired`].
    pub chapters: Option<&'a crate::providers::TextAccess>,
}

/// Fills the request envelope from the running session and parses one command.
pub fn build_request(session: &Session, mut value: serde_json::Value) -> Result<Request> {
    let serde_json::Value::Object(fields) = &mut value else {
        return Err(crate::EditError::InvalidParams);
    };
    fields
        .entry("api_version")
        .or_insert_with(|| serde_json::json!(API_VERSION));
    fields
        .entry("registry_hash")
        .or_insert_with(|| serde_json::json!(registry_hash()));
    fields
        .entry("session_id")
        .or_insert_with(|| serde_json::json!(session.session_id()));
    serde_json::from_value(value).map_err(|_| crate::EditError::InvalidParams)
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        reason = "injected capacity and generation exhaustion"
    )]
    use super::*;
    use crate::{EditError, Session, jobs::JobRunner};
    use serde_json::json;
    struct Allow;
    impl Policy for Allow {
        fn authorize(&self, _: &Command, _: &project::Project) -> Result<()> {
            Ok(())
        }
    }
    #[test]
    fn byte_capacity_is_reserved_before_mutation_and_generation_exhaustion_is_atomic() {
        let mut session = Session::new(
            project::Project::empty(project::ProjectId(0), "before".into()).unwrap(),
            10,
        )
        .unwrap();
        let mut runner = JobRunner::new(
            crate::jobs::RunnerConfig {
                concurrency: [1, 1, 1],
                queue: 1,
                retained: 1,
                network_attempts: 1,
                backoff: std::time::Duration::from_millis(1),
                progress_interval: std::time::Duration::from_millis(10),
            },
            crate::ByteBudget::new(1 << 20).unwrap(),
            std::sync::Arc::new(crate::jobs::SystemClock::default()),
        )
        .unwrap();
        let before = session.project().clone();
        session.ledger.receipt_bytes = MAX_RECEIPT_BYTES - RECEIPT_RESERVATION + 1;
        let request = Request {
            api_version: API_VERSION,
            registry_hash: Some(registry_hash().into()),
            session_id: session.session_id().into(),
            request_id: "capacity".into(),
            idempotency_key: Some("new".into()),
            base_rev: Some(session.revision()),
            command: "project.rename".into(),
            params: json!({"name":"never"}),
        };
        let response = session
            .control_request(
                request,
                Caller {
                    principal: "test",
                    external: true,
                    policy: &Allow,
                    fonts: None,
                    sounds: None,
                    media: None,
                    chapters: None,
                },
                &mut runner,
            )
            .unwrap();
        assert!(matches!(
            response.value,
            ResponseValue::Error(ErrorCode::ReceiptCapacity)
        ));
        assert_eq!(session.project(), &before);
        assert_eq!(session.history_len(), (0, 0));
        session.generation = u64::MAX;
        assert!(matches!(
            session.apply(Command::RenameProject {
                name: "never".into()
            }),
            Err(EditError::Limit)
        ));
        assert_eq!(session.project(), &before);
        session.generation = u64::MAX - 2;
        let result = session.transaction(|s| {
            s.apply(Command::RenameProject {
                name: "preview".into(),
            })?;
            s.apply(Command::RenameProject {
                name: "never".into(),
            })
        });
        assert!(matches!(result, Err(EditError::Limit)));
        assert_eq!(session.project(), &before);
        assert_eq!(session.history_len(), (0, 0));
        assert_eq!(session.generation, u64::MAX);
    }
}

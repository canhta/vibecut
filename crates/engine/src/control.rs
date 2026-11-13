//! Admits transport-neutral commands and runs control-only commands, the top engine layer.
use crate::jobs::{JobId, JobRunner};
use crate::{Command, Session};

pub use crate::session::wire::{
    API_VERSION, BrandColorView, BrandEntryState, BrandKitView, BrandMediaView, CaptionsResult,
    ClipAdjustment, ClipCandidateView, ClipEdge, ClipEnergy,
    ClipExportShort, ClipPieceRange, ClipPieceSpan, ClipRefusal, ClipSubtitleFile, ClipsResult,
    ControlError, ControlResult, CostView, Cursor, ErrorCode, Event, Failure, FillerCandidate,
    FontEntry, FontLoad, FontLoadState, FontOrigin, FontState, JobOutput, JobView, LIMITS, Limits,
    MAX_BATCH_COMMANDS, MAX_BATCH_DEPTH, MAX_COLLECTION, MAX_DEPTH, MAX_EVENTS, MAX_FRAME_BYTES,
    MAX_KEY_BYTES, MAX_NODES, MAX_PATH_BYTES, MAX_PAUSE_MS, MAX_PAUSE_SPANS, MAX_PREVIEW_CLIPS,
    MAX_PREVIEW_EDGE, MAX_PREVIEW_FRAMES, MAX_RECEIPT_BYTES, MAX_RECEIPTS, MAX_RESPONSE_BYTES,
    MAX_SAFE_INTEGER, MAX_TEXT_BYTES, MusicEntry, MusicPage, MusicSource, OutputSpan,
    PREVIEW_QUALITY, PauseCandidates, Payload, PreviewFrame, PreviewResult, ProviderView,
    RECEIPT_RESERVATION, ReframeReason, ReframeReport, ReframeShotReport, ReframeTarget, Request,
    Response, ResponseValue, Revision, SavedMusicEntry, SavedSoundEntry, Snapshot, SoundEntry,
    SoundPage, SoundSource, StickerCategory, StickerEntry, StickerPage, TranslationsResult,
    bounded_json, contract_schema, parse_frame,
    reference_markdown, registry_hash,
};

use crate::{
    AnalysisError, ArtifactError, EditError, FontError, JobFailurePolicy, MusicError, RuntimeError,
    SessionError, SoundError, library, luts, models, providers, recovery, subtitles, transcript,
};

/// Safe diagnostic categories: never include parameter values or user text.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Models(#[from] models::Error),
    #[error(transparent)]
    Lut(#[from] luts::Error),
    #[error(transparent)]
    Library(#[from] library::Error),
    #[error(transparent)]
    Edit(#[from] EditError),
    #[error(transparent)]
    Control(#[from] ControlError),

    #[error(transparent)]
    Recovery(#[from] recovery::Error),
    #[error(transparent)]
    Runtime(#[from] RuntimeError),
    #[error(transparent)]
    Session(#[from] SessionError),
    #[error("render execution failed")]
    Render(#[from] render::Error),
    #[error("media operation failed")]
    Media(#[from] ::media::MediaError),
    #[error("AI execution failed")]
    Ai(#[from] ai::Error),
    #[error("sound library failed: {0}")]
    Sounds(#[from] SoundError),
    #[error("music library failed: {0}")]
    Music(#[from] MusicError),
    #[error("font catalog operation failed")]
    Font(#[from] FontError),
    #[error("brand kit operation failed")]
    BrandKit(#[from] crate::BrandKitError),
    #[error(transparent)]
    Providers(#[from] providers::Error),
    #[error(transparent)]
    Transcript(#[from] transcript::Error),
    #[error(transparent)]
    Subtitles(#[from] subtitles::Error),
    #[error("job/cache IO failed")]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Artifacts(#[from] ArtifactError),
    #[error(transparent)]
    Analysis(#[from] AnalysisError),
}
pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    fn classification(&self) -> (ErrorCode, bool) {
        use ErrorCode as C;
        let code = match self {
            Self::Models(inner) => return (inner.code(), inner.transient()),
            Self::Lut(inner) => return (inner.code(), inner.transient()),
            Self::Library(inner) => return (inner.code(), inner.transient()),
            Self::Artifacts(inner) => return (inner.code(), inner.transient()),
            Self::Analysis(inner) => return (inner.code(), inner.transient()),
            Self::Runtime(inner) => return (inner.code(), inner.transient()),
            Self::Session(inner) => return (inner.code(), inner.transient()),
            Self::Control(inner) => return (inner.code(), inner.transient()),
            Self::Recovery(inner) => return (inner.code(), inner.transient()),
            Self::Sounds(inner) => inner.code(),
            Self::Music(inner) => inner.code(),
            Self::Font(inner) => inner.code(),
            Self::BrandKit(inner) => inner.code(),
            Self::Providers(inner) => return (inner.code(), inner.transient()),
            Self::Transcript(inner) => return (inner.code(), inner.transient()),
            Self::Subtitles(inner) => return (inner.code(), inner.transient()),
            Self::Render(inner) => inner.code(),
            Self::Media(error) => error.code(),
            Self::Ai(inner) => return (inner.code(), inner.transient()),
            Self::Io(_) => C::Io,
            Self::Edit(inner) => return (inner.code(), inner.transient()),
        };
        (code, false)
    }
}
impl Failure for Error {
    fn job_policy(&self) -> JobFailurePolicy {
        match self {
            Self::Transcript(error) => error.job_policy(),
            Self::Subtitles(error) => error.job_policy(),
            Self::Artifacts(error) => error.job_policy(),
            Self::Analysis(error) => error.job_policy(),
            Self::Recovery(error) => error.job_policy(),
            Self::Providers(error) => error.job_policy(),
            Self::Models(error) => error.job_policy(),
            Self::Lut(error) => error.job_policy(),
            Self::Library(error) => error.job_policy(),
            Self::Edit(error) => error.job_policy(),
            Self::Runtime(error) => error.job_policy(),
            Self::Session(error) => error.job_policy(),
            Self::Control(error) => error.job_policy(),
            Self::Ai(error) => error.job_policy(),
            _ => JobFailurePolicy::default(),
        }
    }
    fn code(&self) -> ErrorCode {
        self.classification().0
    }
    fn transient(&self) -> bool {
        self.classification().1
    }
}

impl From<project::Error> for Error {
    fn from(error: project::Error) -> Self {
        Self::Edit(error.into())
    }
}

impl From<time::TimeError> for Error {
    fn from(error: time::TimeError) -> Self {
        Self::Edit(error.into())
    }
}

impl From<captions::Error> for Error {
    fn from(error: captions::Error) -> Self {
        Self::Edit(error.into())
    }
}

mod admission;
mod clips;
mod decode;
mod dispatch;
mod media;
mod output;
mod receipts;
mod resync;
pub use clips::{
    ClipWorkflow, ClipsExportRequest, ClipsSuggestRequest, MAX_CLIPS_RESULT_BYTES, decode_clips,
    encode_clips,
};
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
    import: &'a dyn Fn(&mut JobRunner, &std::path::Path, bool) -> Result<JobId>,
}
impl<'a> FontAccess<'a> {
    /// `list` and `load` serve reads; `download` and `import` submit onto the
    /// supplied runner and returns only the accepted job; cancellation addresses the runner by id.
    pub fn new(
        list: &'a dyn Fn(Option<&str>) -> Result<Vec<FontEntry>>,
        load: &'a dyn Fn(&str) -> Result<FontLoad>,
        download: &'a dyn Fn(&mut JobRunner, &str, bool) -> Result<JobId>,
        import: &'a dyn Fn(&mut JobRunner, &std::path::Path, bool) -> Result<JobId>,
    ) -> Self {
        Self {
            list,
            load,
            download,
            import,
        }
    }
}
/// The brand kit operation the process lends to control commands: the active kit's
/// contents, or `None` when no kit is active. It is composition state borrowed for one
/// request; the kernel owns no store and copies nothing into a project.
#[derive(Clone, Copy)]
pub struct BrandAccess<'a> {
    list: &'a dyn Fn() -> Result<Option<BrandKitView>>,
}
impl<'a> BrandAccess<'a> {
    pub fn new(list: &'a dyn Fn() -> Result<Option<BrandKitView>>) -> Self {
        Self { list }
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
/// The two sticker-catalog operations the process lends to control commands. The catalog and its
/// bundled assets belong to the composition root; the kernel only stores the resolved id and
/// intrinsic size and holds no sticker data of its own.
#[derive(Clone, Copy)]
pub struct StickerAccess<'a> {
    search: &'a dyn Fn(&StickerQuery) -> Result<StickerPage>,
    resolve: &'a dyn Fn(&str) -> Result<StickerEntry>,
}
impl<'a> StickerAccess<'a> {
    /// `search` serves `stickers.search`; `resolve` turns the id given to `stickers.insert` into
    /// its entry, or refuses an id the catalog does not know.
    pub fn new(
        search: &'a dyn Fn(&StickerQuery) -> Result<StickerPage>,
        resolve: &'a dyn Fn(&str) -> Result<StickerEntry>,
    ) -> Self {
        Self { search, resolve }
    }
}
/// One bounded sticker search in the form the lent catalog takes.
#[derive(Clone, Debug)]
pub struct StickerQuery {
    pub query: String,
    pub category: StickerCategory,
    pub limit: Option<u32>,
}
/// A loaded detector with its adapter's complete `VisionFingerprint`.
type LoadedVision = (Box<dyn ai::vision::VisionDetector>, [u8; 32]);

/// The local detector factory behind `reframe.generate`. The composition root
/// owns the pinned model files and the user-initiated download; engine owns the
/// analysis job and its artifact cache. A call returns a ready detector and the
/// loaded adapter's complete `VisionFingerprint`, so no model name enters the key.
pub struct VisionAccess {
    detector: Box<dyn Fn() -> Result<LoadedVision> + Send + Sync>,
}
impl VisionAccess {
    pub fn new(detector: impl Fn() -> Result<LoadedVision> + Send + Sync + 'static) -> Self {
        Self {
            detector: Box::new(detector),
        }
    }
    pub(crate) fn load(&self) -> Result<LoadedVision> {
        (self.detector)()
    }
}
/// The three music-library operations the process lends to control commands. The
/// access object owns the library root and the optional user credential, so the
/// command carries neither; it chooses remote or local search by the request and
/// the approval, never by a self-granted consent in JSON.
#[derive(Clone, Copy)]
pub struct MusicAccess<'a> {
    search: &'a dyn Fn(&MusicQuery) -> Result<MusicPage>,
    save: &'a dyn Fn(&MusicEntry) -> Result<SavedMusicEntry>,
    unsave: &'a dyn Fn(&str) -> Result<bool>,
}
impl<'a> MusicAccess<'a> {
    /// `search` serves `music.search` and reads the saved local library when the
    /// request asks for it, `save` serves `music.save`, and `unsave` serves
    /// `music.unsave`, returning whether an entry was removed.
    pub fn new(
        search: &'a dyn Fn(&MusicQuery) -> Result<MusicPage>,
        save: &'a dyn Fn(&MusicEntry) -> Result<SavedMusicEntry>,
        unsave: &'a dyn Fn(&str) -> Result<bool>,
    ) -> Self {
        Self {
            search,
            save,
            unsave,
        }
    }
}
/// One bounded music-search request in the form the lent library takes.
#[derive(Clone, Debug)]
pub struct MusicQuery {
    pub query: String,
    pub tag: Option<String>,
    pub page: u32,
    pub page_size: u32,
    pub saved: bool,
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
    /// The process music library behind `music.search`, `music.save` and
    /// `music.unsave`. Without it those commands refuse `Unsupported`.
    pub music: Option<&'a MusicAccess<'a>>,
    /// The process sticker catalog behind `stickers.search` and `stickers.insert`. Without it
    /// those commands refuse `Unsupported`.
    pub stickers: Option<&'a StickerAccess<'a>>,
    /// The process brand kit store behind `brand.list`. Without it that command refuses
    /// [`crate::BrandKitError::Unavailable`].
    pub brand: Option<&'a BrandAccess<'a>>,
    /// The process artifact caches behind `media.generate_waveform` and
    /// `media.generate_clip_thumbnails`. Without them those commands refuse.
    /// One mutex instance is shared by the desktop timeline display and the
    /// in-process MCP inbox, so generation and display read the same entries.
    pub media: Option<&'a std::sync::Mutex<MediaAccess>>,
    /// The process transcript-text adapter and the human approval it consults.
    /// Without it `chapters.generate` and `fillers.classify` refuse with
    /// [`crate::providers::Error::ConsentRequired`].
    pub chapters: Option<&'a crate::providers::TextAccess>,
    /// Resolved caption providers, human consent and the shared checkpoint cache.
    pub captions: Option<&'a crate::subtitles::CaptionAccess>,
    /// The local vision detector and its fingerprint behind `reframe.generate`.
    /// Without it the command refuses `Unsupported` before any analysis work.
    pub vision: Option<&'a VisionAccess>,
}

/// Fills the request envelope from the running session and parses one command.
pub fn build_request(
    session: &Session,
    mut value: serde_json::Value,
) -> std::result::Result<Request, ControlError> {
    let serde_json::Value::Object(fields) = &mut value else {
        return Err(ControlError::InvalidParams);
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
    serde_json::from_value(value).map_err(|_| ControlError::InvalidParams)
}

/// The string entry of the closed catalog, for callers that hold only a command id and
/// JSON parameters (the CLI `exec` subcommand, wire-semantics tests). It decodes against
/// the registry schemas, then runs the command with wire validation: malformed or
/// oversized parameters fail without leaking user text or changing session state.
/// In-process code holds a typed [`Command`] and uses [`Session::apply`] or
/// [`Session::control_command`] instead.
pub fn run_wire(
    session: &mut Session,
    id: &str,
    params: serde_json::Value,
) -> std::result::Result<crate::Outcome, EditError> {
    let command = decode::decode_command(id, params)?;
    session.apply_control(command)
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
                    music: None,
                    stickers: None,
                    brand: None,
                    media: None,
                    chapters: None,
                    captions: None,
                    vision: None,
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
            Err(EditError::Control(
                crate::session::wire::ControlError::Limit
            ))
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
        assert!(matches!(
            result,
            Err(EditError::Control(
                crate::session::wire::ControlError::Limit
            ))
        ));
        assert_eq!(session.project(), &before);
        assert_eq!(session.history_len(), (0, 0));
        assert_eq!(session.generation, u64::MAX);
    }
}

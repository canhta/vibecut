#![allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    dead_code
)]
use captions::{Fingerprint, LanguageTag, Segment, Transcript};
use engine::{
    ByteBudget, Command, Session,
    cache::ArtifactCache,
    control::{
        ClipExportShort, ClipWorkflow, ClipsExportRequest, ClipsResult, ClipsSuggestRequest,
        CostView,
    },
    jobs::{JobBytes, JobContext, JobId, JobKind, JobRunner, JobSpec, ResourceClass},
    providers::{ConsentPrompt, Cost, Credentials, Providers, TranscriptionProvider},
    subtitles::CaptionAccess,
};
use engine::{ClipProposal, ClipRole, ClipSource, ClipSourceSegment};
use media::{AssetId, SourceAsset, TimeBase};
use project::{Project, ProjectId};
use shorts::clip_boundaries::ClipBoundarySettings;
use std::sync::{
    Arc, Mutex, OnceLock,
    atomic::{AtomicUsize, Ordering},
};
use time::{FrameRate, SourceRange, TICKS_PER_SECOND, Tick};
use vibecut::{
    preferences::Preferences,
    services::{ai_access::Opener, playback::Sources},
    shorts::{Candidate, Importer, ReviewSource, State as Shorts},
};
pub fn tick(n: i64) -> Tick {
    Tick(n * TICKS_PER_SECOND)
}
pub fn range(start: i64, end: i64) -> SourceRange {
    SourceRange::new(tick(start), tick(end)).unwrap()
}
pub fn source() -> ReviewSource {
    source_at(0)
}
/// The described source the fixture suggestions come from; no file is decoded.
pub fn asset_at(offset: i64) -> SourceAsset {
    testkit::source::asset(1, std::env::temp_dir().join("vibecut-shorts-missing.mp4"))
        .format("fixture")
        .duration(tick(offset + 20))
        .stream(
            testkit::source::video()
                .time_base(TimeBase::new(1, 30).unwrap())
                .duration(tick(offset + 20))
                .start(Tick::ZERO)
                .frame_count(((offset + 20) * 30) as u64)
                .variable_frame_rate(false)
                .size(320, 180)
                .build(),
        )
        .build()
}
pub fn source_at(offset: i64) -> ReviewSource {
    let asset = asset_at(offset);
    let mut session = Session::new(
        Project::empty(ProjectId(1), "Interview".into()).unwrap(),
        100,
    )
    .unwrap();
    session.apply(Command::AddAsset { asset }).unwrap();
    let language = LanguageTag::parse("vi").unwrap();
    let transcript = Transcript {
        source_lang: language,
        fingerprint: Fingerprint {
            provider: "fixture".into(),
            model: "fixture".into(),
            settings: "1".into(),
        },
        segments: vec![
            Segment {
                id: "context".into(),
                range: range(offset + 1, offset + 3),
                text: "Một câu chuyện.".into(),
                words: Vec::new(),
                spans: None,
            },
            Segment {
                id: "hook".into(),
                range: range(offset + 10, offset + 12),
                text: "Điều đáng nhớ.".into(),
                words: Vec::new(),
                spans: None,
            },
        ],
    };
    let pieces = vec![
        ClipSourceSegment {
            range: range(offset + 10, offset + 13),
            role: ClipRole::Hook,
            source_order: 1,
            repair: None,
        },
        ClipSourceSegment {
            range: range(offset, offset + 4),
            role: ClipRole::Context,
            source_order: 0,
            repair: None,
        },
    ];
    let cues = captions::CueTrack {
        styles: Vec::new(),
        cues: transcript
            .segments
            .iter()
            .map(|segment| captions::Cue {
                word_timing: captions::CueWordTiming::Unverified,
                id: segment.id.clone(),
                range: segment.range,
                source_lang: transcript.source_lang.clone(),
                texts: std::collections::BTreeMap::from([(
                    transcript.source_lang.clone(),
                    captions::CueText {
                        text: segment.text.clone(),
                        edited: false,
                        translation_ref: None,
                    },
                )]),
                word_ranges: Vec::new(),
                style: None,
            })
            .collect(),
    };
    ReviewSource {
        project: session.project().as_ref().clone(),
        asset: AssetId(1),
        transcript,
        cues,
        energy: Vec::new(),
        boundaries: ClipBoundarySettings {
            source_range: range(0, offset + 20),
            frame_rate: FrameRate::FPS_30,
            tolerance: Tick(TICKS_PER_SECOND / 2),
            silence_rms: 0.01,
            min_duration: tick(1),
            max_duration: tick(20),
            min_segment_duration: Tick(TICKS_PER_SECOND / 2),
            max_segments: 4,
        },
        candidates: vec![
            Candidate {
                proposal: ClipProposal {
                    segments: Vec::new(),
                    title: "Điều đáng nhớ từ cuộc trò chuyện".into(),
                    hook_text: "Điều đáng nhớ.".into(),
                    reason: "A complete thought, led by the later hook.".into(),
                    score: 0.8,
                },
                segments: pieces.clone(),
                source: ClipSource::Heuristic,
                duration: tick(7),
                adjustments: Vec::new(),
            },
            Candidate {
                proposal: ClipProposal {
                    segments: Vec::new(),
                    title: "A second idea".into(),
                    hook_text: "".into(),
                    reason: "Another complete thought.".into(),
                    score: 0.6,
                },
                segments: vec![ClipSourceSegment {
                    source_order: 0,
                    ..pieces[0].clone()
                }],
                source: ClipSource::Llm,
                duration: tick(3),
                adjustments: Vec::new(),
            },
        ],
    }
}
type Run =
    dyn Fn(&dyn ConsentPrompt, &JobContext) -> engine::control::Result<ReviewSource> + Send + Sync;
/// The suggestion half of the lent clip workflow, standing in for ASR and ranking: its
/// job runs `run` (which may ask the shared consent) and returns the fixture as a
/// `clips.suggest` result. Export is the real `ClipJobs`.
struct Staged {
    run: Arc<Run>,
}
/// The wire result a finished suggestion job retains for `source`.
pub fn result_of(source: &ReviewSource) -> ClipsResult {
    ClipsResult {
        asset: source.asset,
        source_language: source.transcript.source_lang.clone(),
        transcription_provider: "fixture".into(),
        transcription_model: "fixture".into(),
        provider: "heuristic".into(),
        model: "fixture".into(),
        clips: source
            .candidates
            .iter()
            .map(shorts::clip_suggest::candidate_view)
            .collect(),
        rejected: Vec::new(),
        dropped: Vec::new(),
        cues: source.cues.cues.clone(),
        energy: source
            .energy
            .iter()
            .map(|window| engine::control::ClipEnergy(window.range, window.rms as f32))
            .collect(),
        transcript_segments: source.transcript.segments.len(),
        checkpoints: 0,
        transcription_cost: CostView::from(&Cost::Free),
        suggestion_cost: CostView::from(&Cost::Free),
        elapsed_seconds: 0.0,
    }
}
impl ClipWorkflow for Staged {
    fn suggest(
        &self,
        runner: &mut JobRunner,
        access: &CaptionAccess,
        session: &Session,
        _: ClipsSuggestRequest,
        input: engine::cache::CacheKey,
    ) -> engine::control::Result<JobId> {
        let (_, consent) = access.share()?;
        let run = self.run.clone();
        let (id, _) = runner.submit_for_session(
            session,
            JobSpec {
                kind: JobKind::SuggestClips,
                class: ResourceClass::Cpu,
                input,
                bytes: JobBytes {
                    result: engine::control::MAX_CLIPS_RESULT_BYTES as u64,
                    ..Default::default()
                },
            },
            move |hooks| -> engine::control::Result<Vec<u8>> {
                hooks.check_cancelled()?;
                let source = run(consent.as_ref(), hooks)?;
                Ok(serde_json::to_vec(&result_of(&source)).unwrap())
            },
        )?;
        Ok(id)
    }
    fn export(
        &self,
        runner: &mut JobRunner,
        project: &Project,
        request: ClipsExportRequest,
    ) -> engine::control::Result<Vec<ClipExportShort>> {
        shorts::ClipJobs::new(Credentials::new(None, None, None)).export(runner, project, request)
    }
}
/// A transcriber that is never asked: the staged workflow replaces recognition.
struct NoAsr(ai::ProviderConfig);
impl ai::Transcriber for NoAsr {
    fn config(&self) -> &ai::ProviderConfig {
        &self.0
    }
    fn capabilities(&self, _: &ai::Scope) -> ai::Capabilities {
        ai::Capabilities {
            model: self.0.model().clone(),
            mode: "transcribe".into(),
            operation: ai::Operation::Transcribe,
            languages: Vec::new(),
            target_languages: Vec::new(),
            timing: std::collections::BTreeSet::new(),
            diarization: ai::Support::Unsupported,
            live_audio_input: ai::Support::Unsupported,
            file_result_stream: ai::Support::Unsupported,
            max_upload_bytes: None,
            max_duration_micros: None,
            evidence: ai::Evidence::FixtureVerified,
            revision: "fixture".into(),
        }
    }
    fn estimate(&self, _: &ai::Scope) -> ai::CostEstimate {
        ai::CostEstimate::Free
    }
    fn transcribe(
        &self,
        _: ai::AudioWindow<'_>,
        _: &ai::Request<'_>,
        _: &dyn ai::ExecutionHooks,
    ) -> Result<Vec<ai::RecognitionSegment>, ai::Error> {
        Err(ai::Error::Unsupported)
    }
}
fn checkpoint_cache() -> Arc<Mutex<ArtifactCache>> {
    static ROOT: OnceLock<tempfile::TempDir> = OnceLock::new();
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = ROOT
        .get_or_init(|| tempfile::tempdir().unwrap())
        .path()
        .join(NEXT.fetch_add(1, Ordering::Relaxed).to_string());
    std::fs::create_dir_all(&dir).unwrap();
    let (cache, _) = ArtifactCache::open(&dir, ByteBudget::new(64 << 20).unwrap(), 8).unwrap();
    Arc::new(Mutex::new(cache))
}
/// The lent-access seam of the window: each run's access holds fixture providers, the
/// run's consent and a scratch cache, with `run` as the suggestion job body.
pub fn staged(
    run: impl Fn(&dyn ConsentPrompt, &JobContext) -> engine::control::Result<ReviewSource>
    + Send
    + Sync
    + 'static,
) -> Opener {
    let workflow: Arc<dyn ClipWorkflow> = Arc::new(Staged { run: Arc::new(run) });
    Arc::new(move |_, consent| {
        let config = ai::ProviderConfig::new(
            "fixture".into(),
            "fixture".into(),
            "fixture-v1".into(),
            ai::Location::Local {
                backend: "fixture".into(),
                model_ref: "fixture".into(),
            },
        )?;
        let providers = Providers::from_parts(
            TranscriptionProvider::Whisper {
                model: "fixture".into(),
            },
            "transcribe".into(),
            Arc::new(NoAsr(config)),
            None,
            None,
            None,
        )?;
        Ok(
            CaptionAccess::new(Arc::new(providers), consent, checkpoint_cache())
                .with_clips(workflow.clone()),
        )
    })
}
/// Suggests `source` once; the returned slot stays `Some` while no job has run.
pub fn once(source: ReviewSource) -> (Opener, Arc<Mutex<Option<ReviewSource>>>) {
    let slot = Arc::new(Mutex::new(Some(source)));
    let taken = slot.clone();
    let opener = staged(move |_, _| {
        taken
            .lock()
            .unwrap()
            .take()
            .ok_or(engine::control::Error::Edit(engine::EditError::Invalid(
                "fixture already consumed",
            )))
    });
    (opener, slot)
}
/// The media boundary: the described asset instead of a decoded file.
pub fn importer_at(offset: i64) -> Importer {
    Arc::new(move |_| Ok(asset_at(offset)))
}
pub fn state(look: vibecut::Look, opener: Opener, offset: i64) -> Shorts {
    let mut app = Shorts::with_access(
        look,
        ByteBudget::new(512 << 20).unwrap(),
        opener,
        importer_at(offset),
        Arc::new(Pixels),
    )
    .unwrap();
    // The review bounds come from the request; the 3 to 7 s fixture clips need 1 to 20 s.
    app.request.min_seconds = 1;
    app.request.max_seconds = 20;
    app
}
pub struct Pixels;
impl Sources for Pixels {
    fn create(&self) -> Box<dyn render::SourceProvider> {
        Box::new(Pixels)
    }
}
impl render::SourceProvider for Pixels {
    fn status(
        &mut self,
        _: &SourceAsset,
        _: bool,
        _: &dyn render::ExecutionHooks,
    ) -> render::Result<Option<render::UnavailableReason>> {
        Ok(None)
    }
    fn video(
        &mut self,
        _: &SourceAsset,
        _: project::ItemId,
        at: Tick,
        hooks: &dyn render::ExecutionHooks,
    ) -> render::Result<Option<render::Image>> {
        let color = if at >= tick(10) {
            [0.08, 0.35, 0.16, 1.0]
        } else {
            [0.12, 0.18, 0.42, 1.0]
        };
        Ok(Some(render::Image::new(320, 180, color, hooks)?))
    }
    fn audio(
        &mut self,
        _: &SourceAsset,
        _: project::ItemId,
        _: i64,
        frames: usize,
        rate: u32,
        hooks: &dyn render::ExecutionHooks,
    ) -> render::Result<render::AudioBlock> {
        render::AudioBlock::new(rate, 2, frames, render::MemoryKind::Decoded, hooks)
    }
}
pub fn app() -> Shorts {
    let mut app = state(Preferences::default().look(), once(source()).0, 0);
    app.captions = false;
    app.pick_video(std::env::temp_dir().join("interview.mp4"))
        .unwrap();
    app
}
pub fn wait(app: &mut Shorts) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while app.busy() {
        app.tick();
        assert!(std::time::Instant::now() < deadline, "shorts timeout");
        std::thread::yield_now();
    }
    app.tick();
}
pub fn ready() -> Shorts {
    let mut a = app();
    a.generate().unwrap();
    wait(&mut a);
    assert!(a.error().is_none(), "{:?}", a.error());
    a
}

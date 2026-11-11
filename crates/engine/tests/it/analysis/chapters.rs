//! Verifies transcript-to-chapter mapping, the consent gate and job lifecycle.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
use engine::chapters::{ChapterJob, ChapterRequest, decode};
use engine::jobs::{JobRunner, JobState, RunnerConfig, SystemClock};
use engine::providers::{ConsentPrompt, TextAccess, UploadRequest};
use engine::{ByteBudget, Error as EngineError};
use project::{
    CaptionParams, Item, ItemId, ItemParams, Project, ProjectId, Track, TrackId, TrackKind,
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use time::{SourceRange, TICKS_PER_SECOND, Tick};

fn second(n: i64) -> Tick {
    Tick(n * TICKS_PER_SECOND)
}

fn cue(id: &str, start: i64, end: i64, language: &str, text: &str) -> captions::Cue {
    captions::Cue {
        id: id.into(),
        range: SourceRange::new(second(start), second(end)).unwrap(),
        source_lang: captions::LanguageTag::parse(language).unwrap(),
        texts: [(
            captions::LanguageTag::parse(language).unwrap(),
            captions::CueText {
                text: text.into(),
                edited: false,
                translation_ref: None,
            },
        )]
        .into_iter()
        .collect(),
        word_ranges: Vec::new(),
        style: None,
    }
}

/// Three one-second cues in a caption item that starts one second into output.
fn project() -> Arc<Project> {
    let mut data = Project::empty(ProjectId(1), "Chương".into())
        .unwrap()
        .into_data();
    data.assets.push(
        testkit::source::asset(0, std::env::temp_dir().join("vibecut-chapters-source.mp4"))
            .format("mp4")
            .duration(second(4))
            .content_hash(media::ContentHash::from_bytes(b"chapters"))
            .stream(
                testkit::source::video()
                    .time_base(media::TimeBase::new(1, 30000).unwrap())
                    .duration(second(4))
                    .build(),
            )
            .build(),
    );
    data.scene_mut().timeline.overlays.push(Track {
        id: TrackId(4),
        name: "Captions".into(),
        kind: TrackKind::Text,
        muted: false,
        hidden: false,
        locked: false,
        items: vec![Item {
            hidden: false,
            blend_mode: Default::default(),
            retime: Default::default(),
            masks: Vec::new(),
            effects: Vec::new(),
            id: ItemId(4),
            animations: Default::default(),
            source_asset: Some(media::AssetId(0)),
            source_range: SourceRange::new(Tick::ZERO, second(3)).unwrap(),
            start: second(1),
            content: ItemParams::Caption(CaptionParams {
                cues: captions::CueTrack {
                    styles: Vec::new(),
                    cues: vec![
                        cue("cue-a", 0, 1, "en", "First"),
                        cue("cue-b", 1, 2, "en", "Second"),
                        cue("cue-c", 2, 3, "en", "Third"),
                    ],
                },
            }),
        }],
    });
    Arc::new(Project::new(data).unwrap())
}

fn runner() -> JobRunner {
    JobRunner::new(
        RunnerConfig {
            concurrency: [1, 1, 1],
            queue: 8,
            retained: 8,
            network_attempts: 1,
            backoff: std::time::Duration::from_millis(1),
            progress_interval: std::time::Duration::from_millis(10),
        },
        ByteBudget::new(64 << 20).unwrap(),
        Arc::new(SystemClock::default()),
    )
    .unwrap()
}

fn config() -> ai::ProviderConfig {
    ai::chapters::OpenAiChapterGenerator::provider_config("gpt-6-luna").unwrap()
}

fn capabilities(scope: &ai::Scope, config: &ai::ProviderConfig) -> ai::Capabilities {
    ai::Capabilities {
        model: config.model().clone(),
        mode: ai::chapters::MODE.into(),
        operation: ai::Operation::GenerateChapters,
        languages: vec![scope.source_language.clone()],
        target_languages: Vec::new(),
        timing: Default::default(),
        diarization: ai::Support::Unsupported,
        live_audio_input: ai::Support::Unsupported,
        file_result_stream: ai::Support::Unsupported,
        max_upload_bytes: Some(ai::chapters::MAX_WIRE),
        max_duration_micros: None,
        evidence: ai::Evidence::Advertised,
        revision: "chapters-v1".into(),
    }
}

/// Records the input it received and proposes the first and third cues.
struct FixtureChapters {
    config: ai::ProviderConfig,
    seen: Arc<Mutex<Option<ai::chapters::ChapterInput>>>,
    calls: Arc<AtomicUsize>,
}
impl ai::chapters::ChapterGenerator for FixtureChapters {
    fn config(&self) -> &ai::ProviderConfig {
        &self.config
    }
    fn capabilities(&self, scope: &ai::Scope) -> ai::Capabilities {
        capabilities(scope, &self.config)
    }
    fn estimate(&self, _: &ai::Scope) -> ai::CostEstimate {
        ai::CostEstimate::Free
    }
    fn propose(
        &self,
        input: &ai::chapters::ChapterInput,
        _: &ai::Request<'_>,
        _: &dyn ai::ExecutionHooks,
    ) -> Result<ai::chapters::ChapterProposal, ai::Error> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        *self.seen.lock().unwrap() = Some(input.clone());
        let first = input.segments.first().ok_or(ai::Error::Schema)?;
        let last = input.segments.get(2).ok_or(ai::Error::Schema)?;
        Ok(ai::chapters::ChapterProposal {
            title: "A journey".into(),
            description: "Three short chapters.".into(),
            chapters: vec![
                ai::chapters::ChapterProposalEntry {
                    start_segment_id: first.id.clone(),
                    title: "Opening".into(),
                },
                ai::chapters::ChapterProposalEntry {
                    start_segment_id: last.id.clone(),
                    title: "Closing".into(),
                },
            ],
        })
    }
}

struct FixedError;
impl ai::chapters::ChapterGenerator for FixedError {
    fn config(&self) -> &ai::ProviderConfig {
        static CONFIG: std::sync::OnceLock<ai::ProviderConfig> = std::sync::OnceLock::new();
        CONFIG.get_or_init(config)
    }
    fn capabilities(&self, scope: &ai::Scope) -> ai::Capabilities {
        capabilities(scope, self.config())
    }
    fn estimate(&self, _: &ai::Scope) -> ai::CostEstimate {
        ai::CostEstimate::Free
    }
    fn propose(
        &self,
        _: &ai::chapters::ChapterInput,
        _: &ai::Request<'_>,
        _: &dyn ai::ExecutionHooks,
    ) -> Result<ai::chapters::ChapterProposal, ai::Error> {
        Err(ai::Error::Auth)
    }
}

struct Waiting;
impl ai::chapters::ChapterGenerator for Waiting {
    fn config(&self) -> &ai::ProviderConfig {
        static CONFIG: std::sync::OnceLock<ai::ProviderConfig> = std::sync::OnceLock::new();
        CONFIG.get_or_init(config)
    }
    fn capabilities(&self, scope: &ai::Scope) -> ai::Capabilities {
        capabilities(scope, self.config())
    }
    fn estimate(&self, _: &ai::Scope) -> ai::CostEstimate {
        ai::CostEstimate::Free
    }
    fn propose(
        &self,
        _: &ai::chapters::ChapterInput,
        _: &ai::Request<'_>,
        hooks: &dyn ai::ExecutionHooks,
    ) -> Result<ai::chapters::ChapterProposal, ai::Error> {
        while !hooks.cancelled() {
            std::thread::yield_now();
        }
        Err(ai::Error::Cancelled)
    }
}

struct Grant {
    allowed: Arc<AtomicBool>,
}
impl ConsentPrompt for Grant {
    fn approve(&self, _: &UploadRequest) -> engine::providers::Result<()> {
        if self.allowed.load(Ordering::SeqCst) {
            Ok(())
        } else {
            Err(engine::providers::Error::ConsentRequired.into())
        }
    }
}

/// Untouched filler adapter: these tests only exercise chapter preparation.
struct UnusedFillers {
    config: ai::ProviderConfig,
}
impl ai::fillers::FillerClassifier for UnusedFillers {
    fn config(&self) -> &ai::ProviderConfig {
        &self.config
    }
    fn capabilities(&self, scope: &ai::Scope) -> ai::Capabilities {
        ai::Capabilities {
            model: self.config.model().clone(),
            mode: ai::fillers::MODE.into(),
            operation: ai::Operation::ClassifyFillers,
            languages: vec![scope.source_language.clone()],
            target_languages: Vec::new(),
            timing: Default::default(),
            diarization: ai::Support::Unsupported,
            live_audio_input: ai::Support::Unsupported,
            file_result_stream: ai::Support::Unsupported,
            max_upload_bytes: Some(ai::fillers::MAX_WIRE),
            max_duration_micros: None,
            evidence: ai::Evidence::Advertised,
            revision: "fillers-v1".into(),
        }
    }
    fn estimate(&self, _: &ai::Scope) -> ai::CostEstimate {
        ai::CostEstimate::Free
    }
    fn classify(
        &self,
        _: &ai::fillers::FillerInput,
        _: &ai::Request<'_>,
        _: &dyn ai::ExecutionHooks,
    ) -> Result<ai::fillers::FillerProposal, ai::Error> {
        Err(ai::Error::Unsupported)
    }
}

fn access(
    generator: Arc<dyn ai::chapters::ChapterGenerator>,
    allowed: Arc<AtomicBool>,
) -> TextAccess {
    let config = ai::fillers::OpenAiFillerClassifier::provider_config("gpt-6-luna").unwrap();
    TextAccess::new(
        generator,
        Arc::new(UnusedFillers { config }),
        Box::new(Grant { allowed }),
    )
}

fn wait(runner: &mut JobRunner, job: &ChapterJob) -> JobState {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        runner.tick().unwrap();
        let state = runner.snapshot(job.id).unwrap().state;
        if matches!(
            state,
            JobState::Completed | JobState::Failed | JobState::Cancelled
        ) {
            return state;
        }
        assert!(std::time::Instant::now() < deadline, "job deadline");
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

#[test]
fn generation_maps_cues_to_output_time_and_decodes_the_document() {
    let project = project();
    let request =
        ChapterRequest::new(&project, captions::LanguageTag::parse("en").unwrap(), 4).unwrap();
    let input = request.input();
    assert_eq!(input.segments.len(), 3);
    assert_eq!(input.segments[0].id.as_str(), "4:cue-a");
    assert_eq!(input.segments[0].range.start(), second(1));
    assert_eq!(input.segments[1].range.start(), second(2));
    assert_eq!(input.segments[2].range.end(), second(4));
    assert_eq!(input.settings.duration, second(4));

    let seen = Arc::new(Mutex::new(None));
    let calls = Arc::new(AtomicUsize::new(0));
    let allowed = Arc::new(AtomicBool::new(true));
    let access = access(
        Arc::new(FixtureChapters {
            config: config(),
            seen: seen.clone(),
            calls: calls.clone(),
        }),
        allowed,
    );
    let mut runner = runner();
    let job = request.submit(&mut runner, &access).unwrap();
    assert_eq!(wait(&mut runner, &job), JobState::Completed);
    let document = job.finish(&mut runner).unwrap().unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(seen.lock().unwrap().as_ref().unwrap().segments.len(), 3);
    assert_eq!(document.title().as_str(), "A journey");
    assert_eq!(document.language().as_str(), "en");
    assert_eq!(document.duration(), second(4));
    assert_eq!(document.chapters().len(), 2);
    assert_eq!(document.chapters()[0].start, second(1));
    assert_eq!(document.chapters()[0].segment_id.as_str(), "4:cue-a");
    assert_eq!(document.chapters()[1].start, second(3));

    let bytes = serde_json::to_vec(&document).unwrap();
    assert_eq!(decode(&bytes).unwrap(), document);
    assert!(matches!(
        job.finish(&mut runner),
        Err(EngineError::Runtime(engine::RuntimeError::JobNotRetained))
    ));
    runner.shutdown();
}

#[test]
fn empty_unsupported_and_hostile_inputs_refuse_before_a_job() {
    let empty = Arc::new(Project::empty(ProjectId(2), "Empty".into()).unwrap());
    let en = captions::LanguageTag::parse("en").unwrap();
    assert!(matches!(
        ChapterRequest::new(&empty, en.clone(), 4),
        Err(EngineError::Ai(ai::Error::Chapters(
            ai::chapters::ChapterRefusal::EmptyTranscript
        )))
    ));
    let fr = captions::LanguageTag::parse("fr").unwrap();
    assert!(matches!(
        ChapterRequest::new(&empty, fr, 4),
        Err(EngineError::Ai(ai::Error::Chapters(
            ai::chapters::ChapterRefusal::UnsupportedLanguage
        )))
    ));
    for max in [0, 51] {
        assert!(matches!(
            ChapterRequest::new(&project(), en.clone(), max),
            Err(EngineError::Ai(ai::Error::Chapters(
                ai::chapters::ChapterRefusal::InvalidSettings
            )))
        ));
    }
    let vi = captions::LanguageTag::parse("vi").unwrap();
    assert!(matches!(
        ChapterRequest::new(&project(), vi, 4),
        Err(EngineError::Ai(ai::Error::Chapters(
            ai::chapters::ChapterRefusal::EmptyTranscript
        )))
    ));

    let mut data = project().as_ref().clone().into_data();
    if let ItemParams::Caption(params) = &mut data
        .scene_mut()
        .timeline
        .overlays
        .first_mut()
        .unwrap()
        .items
        .first_mut()
        .unwrap()
        .content
    {
        params.cues.cues[1].texts.get_mut(&en).unwrap().text = "  ".into();
    }
    let hostile = Arc::new(Project::new(data).unwrap());
    assert!(matches!(
        ChapterRequest::new(&hostile, en, 4),
        Err(EngineError::Ai(ai::Error::Chapters(
            ai::chapters::ChapterRefusal::InvalidSegments
        )))
    ));
}

#[test]
fn consent_is_required_before_the_adapter_and_cancel_discards_the_result() {
    let project = project();
    let en = captions::LanguageTag::parse("en").unwrap();
    let request = ChapterRequest::new(&project, en, 4).unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = Arc::new(Mutex::new(None));
    let mut runner = runner();
    let denied = access(
        Arc::new(FixtureChapters {
            config: config(),
            seen,
            calls: calls.clone(),
        }),
        Arc::new(AtomicBool::new(false)),
    );
    assert!(matches!(
        request.submit(&mut runner, &denied),
        Err(EngineError::Providers(
            engine::providers::Error::ConsentRequired
        ))
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 0);

    let allowed = access(Arc::new(Waiting), Arc::new(AtomicBool::new(true)));
    let job = request.submit(&mut runner, &allowed).unwrap();
    job.cancel.cancel();
    assert_eq!(wait(&mut runner, &job), JobState::Cancelled);
    assert!(matches!(
        job.finish(&mut runner),
        Err(EngineError::Runtime(engine::RuntimeError::Cancelled))
    ));
    runner.shutdown();
}

#[test]
fn provider_failures_are_typed_job_failures() {
    let project = project();
    let request =
        ChapterRequest::new(&project, captions::LanguageTag::parse("en").unwrap(), 4).unwrap();
    let mut runner = runner();
    let access = access(Arc::new(FixedError), Arc::new(AtomicBool::new(true)));
    let job = request.submit(&mut runner, &access).unwrap();
    assert_eq!(wait(&mut runner, &job), JobState::Failed);
    assert!(matches!(
        job.finish(&mut runner),
        Err(EngineError::Runtime(engine::RuntimeError::Failed(error))) if matches!((error.as_ref() as &(dyn std::error::Error + 'static)).downcast_ref::<EngineError>(), Some(EngineError::Ai(ai::Error::Auth)))
    ));
    runner.shutdown();
}

//! Offline clip workflow contracts: ASR-to-input snapping, explicit provider
//! choice and the registry export across hostile selections and queue pressure.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "integration fixtures"
)]
use ai::{
    Capabilities, CostEstimate, Error, Evidence, ExecutionHooks, Location, Operation,
    ProviderConfig, RecognitionSegment, Request, SampleRange, Scope, Support, TimingUnit,
    Transcriber, Translation, TranslationWindow, Translator,
};
use engine::providers::{
    AllowAll, Credentials, DenyUploads, Providers, TranscriptionProvider, TranslationProvider,
};
use engine::subtitles::LanguageTag;
use engine::transcript::{Cancellation, SilentProgress};
use captions::CueTrack;
use engine::control::{ClipWorkflow as _, ClipsExportRequest};
use engine::{
    ByteBudget, ClipSettings,
    jobs::{JobRunner, RunnerConfig, SystemClock},
};
use shorts::{
    ClipJobs, ClipRequest, SnapSettings,
    clip_suggest::{self, SuggestionProvider},
};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};
use time::{FrameRate, Rounding, Tick};

fn lang(value: &str) -> LanguageTag {
    LanguageTag::parse(value).unwrap()
}

fn fixture() -> Option<PathBuf> {
    static FIXTURE: OnceLock<Option<PathBuf>> = OnceLock::new();
    FIXTURE
        .get_or_init(|| {
            testkit::oracle::generate(
                &testkit::fixture::dir("clip-suggest"),
                "clip suggestion fixture",
                "clips-2s.mp4",
                &[
                    "-f",
                    "lavfi",
                    "-i",
                    "color=c=black:s=320x240:r=30:d=2",
                    "-f",
                    "lavfi",
                    "-i",
                    "sine=frequency=440:sample_rate=48000:duration=2",
                    "-c:v",
                    "libx264",
                    "-pix_fmt",
                    "yuv420p",
                    "-c:a",
                    "aac",
                    "-shortest",
                ],
            )
        })
        .clone()
}

fn config(provider: &str, model: &str, location: Location) -> ProviderConfig {
    ProviderConfig::new(
        provider.into(),
        model.into(),
        format!("{provider}-settings-v1").into(),
        location,
    )
    .unwrap()
}

/// Three sentences across the whole extracted window, so the heuristic has
/// deterministic sentence boundaries with no real model.
struct FakeAsr {
    config: ProviderConfig,
    calls: Arc<std::sync::atomic::AtomicUsize>,
}
impl FakeAsr {
    fn new() -> Self {
        Self {
            calls: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            config: config(
                "fake-asr",
                "fake-asr-model",
                Location::Local {
                    backend: "fixture".into(),
                    model_ref: "fixture".into(),
                },
            ),
        }
    }
}
impl Transcriber for FakeAsr {
    fn config(&self) -> &ProviderConfig {
        &self.config
    }
    fn capabilities(&self, _: &Scope) -> Capabilities {
        Capabilities {
            model: self.config.model().clone(),
            mode: "transcribe".into(),
            operation: Operation::Transcribe,
            languages: vec![lang("vi"), lang("en"), lang("zh")],
            target_languages: Vec::new(),
            timing: BTreeSet::from([TimingUnit::Segment]),
            diarization: Support::Unsupported,
            live_audio_input: Support::Unsupported,
            file_result_stream: Support::Unsupported,
            max_upload_bytes: None,
            max_duration_micros: None,
            evidence: Evidence::FixtureVerified,
            revision: "fake-asr-1".into(),
        }
    }
    fn estimate(&self, _: &Scope) -> CostEstimate {
        CostEstimate::Free
    }
    fn transcribe(
        &self,
        audio: ai::AudioWindow<'_>,
        request: &Request<'_>,
        _: &dyn ExecutionHooks,
    ) -> Result<Vec<RecognitionSegment>, Error> {
        self.calls
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let text = request.scope.source_language.as_str();
        Ok(vec![RecognitionSegment {
            id: "fake:0".into(),
            text: format!("one complete sentence in {text}.").into(),
            range: Some(SampleRange::new(audio.range.start(), audio.range.end()).unwrap()),
            timing: Vec::new(),
            raw_tokens: Vec::new(),
        }])
    }
}

struct FakeTranslator {
    config: ProviderConfig,
}
impl FakeTranslator {
    fn new() -> Self {
        Self {
            config: config(
                "fake-translator",
                "fake-translator-model",
                Location::Remote {
                    backend: "fixture".into(),
                    base_url: "https://fixture.invalid".into(),
                    auth_ref: "fixture".into(),
                },
            ),
        }
    }
}
impl Translator for FakeTranslator {
    fn config(&self) -> &ProviderConfig {
        &self.config
    }
    fn capabilities(&self, _: &Scope) -> Capabilities {
        Capabilities {
            model: self.config.model().clone(),
            mode: "context-window".into(),
            operation: Operation::Translate,
            languages: vec![lang("vi"), lang("en"), lang("zh")],
            target_languages: vec![lang("vi"), lang("en"), lang("zh")],
            timing: BTreeSet::new(),
            diarization: Support::Unsupported,
            live_audio_input: Support::Unsupported,
            file_result_stream: Support::Unsupported,
            max_upload_bytes: None,
            max_duration_micros: None,
            evidence: Evidence::FixtureVerified,
            revision: "fake-translator-1".into(),
        }
    }
    fn estimate(&self, _: &Scope) -> CostEstimate {
        CostEstimate::Free
    }
    fn translate(
        &self,
        window: &TranslationWindow,
        _: &Request<'_>,
        _: &dyn ExecutionHooks,
    ) -> Result<Translation, Error> {
        let texts = window
            .segments
            .iter()
            .map(|segment| {
                (
                    segment.id.clone(),
                    format!(
                        "{} [{}]",
                        segment.text.as_str(),
                        window.target_language.as_str()
                    )
                    .into(),
                )
            })
            .collect();
        Ok(Translation {
            source_language: window.source_language.clone(),
            target_language: window.target_language.clone(),
            texts,
            number_warnings: Vec::new(),
        })
    }
}

fn providers(translation: bool) -> Providers {
    Providers::from_parts(
        TranscriptionProvider::Whisper {
            model: "fake-asr-model".into(),
        },
        "transcribe".into(),
        Arc::new(FakeAsr::new()),
        None,
        translation.then(|| TranslationProvider::OpenAi {
            model: "fake-translator-model".into(),
        }),
        translation.then(|| Arc::new(FakeTranslator::new()) as Arc<dyn Translator>),
    )
    .unwrap()
}

fn runner() -> (JobRunner, ByteBudget) {
    runner_with_exports(1, 8, 8)
}

fn runner_with_exports(exports: usize, queue: usize, retained: usize) -> (JobRunner, ByteBudget) {
    let budget = ByteBudget::new(2 << 30).unwrap();
    (
        JobRunner::new(
            RunnerConfig {
                concurrency: [1, exports, 2],
                queue,
                retained,
                network_attempts: 1,
                backoff: Duration::from_millis(10),
                progress_interval: Duration::from_millis(10),
            },
            budget.clone(),
            Arc::new(SystemClock::default()),
        )
        .unwrap(),
        budget,
    )
}

fn settings() -> ClipSettings {
    ClipSettings {
        count: 2,
        min_duration: Tick::from_seconds_f64(0.3, Rounding::Nearest).unwrap(),
        max_duration: Tick::from_seconds_f64(2.0, Rounding::Nearest).unwrap(),
        max_segments: 2,
        min_segment_duration: Tick::from_seconds_f64(0.1, Rounding::Nearest).unwrap(),
        overlap_tolerance: 0.5,
        language: lang("en"),
        keywords: Vec::new(),
    }
}

fn request(input: &Path) -> ClipRequest {
    ClipRequest {
        input: input.to_path_buf(),
        source_language: Some(lang("en")),
        settings: settings(),
        snap: SnapSettings {
            frame_rate: Some(FrameRate::FPS_30),
            tolerance: Tick::from_seconds_f64(0.5, Rounding::Nearest).unwrap(),
            silence_rms: 0.01,
        },
        provider: SuggestionProvider::Heuristic,
    }
}

fn suggest(
    input: &Path,
    providers: &Providers,
    runner: &mut JobRunner,
) -> shorts::ClipSuggestionOutcome {
    let root = tempfile::tempdir().unwrap();
    let (mut cache, _) =
        engine::cache::ArtifactCache::open(root.path(), ByteBudget::new(1 << 30).unwrap(), 8)
            .unwrap();
    let credentials = Credentials::new(None, None, None);
    clip_suggest::suggest(
        &request(input),
        &credentials,
        providers,
        runner,
        &mut cache,
        &DenyUploads,
        &SilentProgress,
        &Cancellation::new(),
    )
    .unwrap()
}

#[test]
fn heuristic_suggestion_snaps_inside_the_source() {
    let Some(input) = fixture() else {
        return;
    };
    let providers = providers(false);
    let (mut runner, _budget) = runner();
    let outcome = suggest(&input, &providers, &mut runner);
    assert_eq!(outcome.source_language, lang("en"));
    assert_eq!(outcome.provider, "heuristic");
    assert_eq!(outcome.model, "sentence-pause-keyword");
    assert!(outcome.transcript_segments >= 1);
    assert!(!outcome.clips.is_empty());
    let source_end = Tick::from_seconds_f64(2.0, Rounding::Nearest).unwrap();
    for clip in &outcome.clips {
        assert!(clip.duration > Tick::ZERO);
        assert!(clip.duration <= settings().max_duration);
        assert_eq!(clip.adjustments.len(), clip.segments.len());
        for segment in &clip.segments {
            assert!(segment.range.end() <= source_end);
            assert!(segment.range.start() >= Tick::ZERO);
        }
    }
}
#[test]
fn maximum_model_ids_run_shorts_asr_with_explicit_unknown_price_and_isolated_checkpoints() {
    use ai::elevenlabs::ElevenLabsTranscriber;
    use ai::transport::ApiKey;
    use engine::providers::{ConsentPrompt, Cost, UploadRequest};
    use transport::{HttpRequest, HttpResponse, HttpTransport};
    struct Transport {
        model: String,
        calls: Mutex<usize>,
    }
    impl HttpTransport for Transport {
        fn send(&self, request: HttpRequest) -> Result<HttpResponse, transport::Error> {
            assert_eq!(request.url, ai::elevenlabs::ENDPOINT);
            assert!(
                request
                    .body
                    .windows(self.model.len())
                    .any(|s| s == self.model.as_bytes())
            );
            *self.calls.lock().unwrap() += 1;
            Ok(HttpResponse { status: 200, headers: vec![], body: br#"{"language_code":"eng","text":"Hello world.","words":[{"text":"Hello","start":0.1,"end":0.5,"type":"word","logprob":0},{"text":" ","start":0.5,"end":0.5,"type":"spacing"},{"text":"world.","start":0.5,"end":1.5,"type":"word","logprob":0}]}"#.to_vec() })
        }
    }
    struct Approve {
        config: ProviderConfig,
        acknowledgements: Mutex<usize>,
    }
    impl ConsentPrompt for Approve {
        fn approve(&self, request: &UploadRequest) -> engine::providers::Result<()> {
            assert_eq!(request.config, self.config);
            assert_eq!(request.model, self.config.model().as_str());
            assert_eq!(request.cost, Cost::Unknown);
            Ok(())
        }
        fn approve_unknown_price(&self, request: &UploadRequest) -> engine::providers::Result<()> {
            self.approve(request)?;
            *self.acknowledgements.lock().unwrap() += 1;
            Ok(())
        }
    }
    let input = fixture().expect("synthetic media oracle must be available");
    let root = tempfile::tempdir().unwrap();
    let (mut runner, budget) = runner();
    let (mut cache, _) = engine::cache::ArtifactCache::open(root.path(), budget, 8).unwrap();
    for suffix in ['a', 'b'] {
        let model = format!("{}{suffix}", "x".repeat(255));
        let config = ElevenLabsTranscriber::provider_config(&model).unwrap();
        let transport = Arc::new(Transport {
            model: model.clone(),
            calls: Mutex::new(0),
        });
        let provider = Arc::new(
            ElevenLabsTranscriber::new(
                config.clone(),
                ApiKey::new("fixture").unwrap(),
                transport.clone(),
            )
            .unwrap(),
        );
        let providers = Providers::from_parts(
            TranscriptionProvider::ElevenLabs { model },
            ai::elevenlabs::MODE.into(),
            provider,
            None,
            None,
            None,
        )
        .unwrap();
        let approve = Approve {
            config,
            acknowledgements: Mutex::new(0),
        };
        let request = request(&input);
        let credentials = Credentials::new(None, None, None);
        let cancel = Cancellation::new();
        let error = clip_suggest::suggest(
            &request,
            &credentials,
            &providers,
            &mut runner,
            &mut cache,
            &AllowAll,
            &SilentProgress,
            &cancel,
        )
        .err()
        .unwrap();
        assert!(
            matches!(
                error,
                shorts::Error::Engine(engine::control::Error::Transcript(
                    engine::transcript::Error::Providers(engine::providers::Error::Ai(Error::Cost))
                ))
            ),
            "{error:?}"
        );
        assert_eq!(*transport.calls.lock().unwrap(), 0);
        for checkpoints in [0, 1] {
            let outcome = clip_suggest::suggest(
                &request,
                &credentials,
                &providers,
                &mut runner,
                &mut cache,
                &approve,
                &SilentProgress,
                &cancel,
            )
            .unwrap();
            assert_eq!(outcome.checkpoints, checkpoints);
            assert!(!outcome.clips.is_empty());
            assert_eq!(*transport.calls.lock().unwrap(), 1);
            assert_eq!(*approve.acknowledgements.lock().unwrap(), 1);
        }
    }
}

#[test]
fn suggestion_refuses_shared_asr_budget_pressure_before_provider_work() {
    let Some(input) = fixture() else { return };
    for available in [64 << 20, (80 << 20) - 1] {
        let root = tempfile::tempdir().unwrap();
        let (mut runner, budget) = runner();
        let (mut cache, _) =
            engine::cache::ArtifactCache::open(root.path(), budget.clone(), 8).unwrap();
        let held = budget
            .reserve(engine::ByteOwner::Export, (2 << 30) - available)
            .unwrap();
        let asr = FakeAsr::new();
        let calls = asr.calls.clone();
        let providers = Providers::from_parts(
            TranscriptionProvider::Whisper {
                model: "fake-asr-model".into(),
            },
            "transcribe".into(),
            Arc::new(asr),
            None,
            None,
            None,
        )
        .unwrap();
        let before = budget.snapshot().unwrap().used;
        let result = clip_suggest::suggest(
            &request(&input),
            &Credentials::new(None, None, None),
            &providers,
            &mut runner,
            &mut cache,
            &DenyUploads,
            &SilentProgress,
            &Cancellation::new(),
        );
        assert!(matches!(
            result,
            Err(shorts::Error::Engine(engine::control::Error::Runtime(
                engine::RuntimeError::BudgetFull
            )))
        ));
        assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 0);
        assert_eq!(budget.snapshot().unwrap().used, before);
        drop(held);
        let result = clip_suggest::suggest(
            &request(&input),
            &Credentials::new(None, None, None),
            &providers,
            &mut runner,
            &mut cache,
            &DenyUploads,
            &SilentProgress,
            &Cancellation::new(),
        )
        .unwrap();
        assert!(!result.clips.is_empty());
        assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 1);
        drop(runner);
        drop(cache);
        assert_eq!(budget.snapshot().unwrap().used, 0);
    }
}

#[test]
fn registry_export_submits_one_job_per_selected_short() {
    let Some(input) = fixture() else {
        return;
    };
    let providers = providers(false);
    let (mut runner, _budget) = runner();
    let mut outcome = suggest(&input, &providers, &mut runner);
    let jobs = ClipJobs::new(Credentials::new(None, None, None));
    let directory = tempfile::tempdir().unwrap();
    let request = |outcome: &shorts::ClipSuggestionOutcome,
                   select: Vec<usize>,
                   languages: Vec<LanguageTag>,
                   directory: &Path| ClipsExportRequest {
        result: clip_suggest::clips_result(outcome),
        cues: CueTrack {
            styles: Vec::new(),
            cues: outcome.source_cues.clone(),
        },
        select,
        languages,
        formats: Vec::new(),
        directory: directory.to_path_buf(),
        overwrite: false,
        allow_degraded: false,
    };
    // Hostile selections and a caption language the cues lack are refused before any job.
    for (select, languages) in [
        (vec![outcome.clips.len()], Vec::new()),
        (vec![0, 0], Vec::new()),
        (Vec::new(), Vec::new()),
        (vec![0], vec![lang("vi")]),
    ] {
        let refused = jobs.export(
            &mut runner,
            &outcome.source_project,
            request(&outcome, select, languages, directory.path()),
        );
        assert!(refused.is_err());
        assert_eq!(runner.counts().0, 0, "a refused export admitted a job");
    }

    // Table row: nine selected clips run with the configured export concurrency and
    // each short owns its job. Three frames per clip cover scheduling without encoding
    // nine full videos.
    let template = outcome.clips[0].clone();
    outcome.clips = (0..9)
        .map(|index| {
            let mut clip = template.clone();
            clip.proposal.title = format!("nine-{index}").into();
            clip.segments.truncate(1);
            let segment = &mut clip.segments[0];
            segment.range = time::SourceRange::new(
                segment.range.start(),
                segment.range.start() + Tick::from_seconds_f64(0.1, Rounding::Nearest).unwrap(),
            )
            .unwrap();
            clip.duration = segment.range.duration();
            clip
        })
        .collect();
    let (mut runner, _) = runner_with_exports(2, 9, 9);
    let shorts = jobs
        .export(
            &mut runner,
            &outcome.source_project,
            request(&outcome, (0..9).collect(), Vec::new(), directory.path()),
        )
        .unwrap();
    assert_eq!(shorts.len(), 9);
    let mut peak = 0;
    for _ in 0..60_000 {
        runner.tick().unwrap();
        let (pending, active, _, _) = runner.counts();
        peak = peak.max(active);
        if pending == 0 && active == 0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(peak, 2, "runner export capacity was underused");
    for short in &shorts {
        assert_eq!(
            runner.snapshot(short.job).unwrap().state,
            engine::jobs::JobState::Completed
        );
        assert!(short.video.is_file());
    }

    // A queue smaller than the selection refuses the whole export and leaves no admitted
    // job and no file behind.
    let queued = tempfile::tempdir().unwrap();
    let (mut congested, budget) = runner_with_exports(2, 1, 8);
    let refused = jobs
        .export(
            &mut congested,
            &outcome.source_project,
            request(&outcome, (0..3).collect(), Vec::new(), queued.path()),
        )
        .unwrap_err();
    assert!(matches!(
        refused,
        engine::control::Error::Runtime(engine::RuntimeError::QueueFull)
    ));
    for _ in 0..2_000 {
        congested.tick().unwrap();
        let (pending, active, _, _) = congested.counts();
        if pending == 0 && active == 0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!((congested.counts().0, congested.counts().1), (0, 0));
    assert_eq!(std::fs::read_dir(queued.path()).unwrap().count(), 0);
    drop(congested);
    assert_eq!(budget.snapshot().unwrap().used, 0);
}

#[test]
fn provider_choice_never_falls_back() {
    let options = clip_suggest::suggestion_options();
    let heuristic = options
        .iter()
        .find(|option| option.id == "heuristic")
        .unwrap();
    assert!(heuristic.local);
    assert!(heuristic.policy_url.is_none());
    let cases = [
        ("heuristic", None, Some(SuggestionProvider::Heuristic)),
        (
            "openai",
            None,
            Some(SuggestionProvider::OpenAi {
                model: "gpt-6-luna".into(),
            }),
        ),
        (
            "openai",
            Some("gpt-6.1-sol"),
            Some(SuggestionProvider::OpenAi {
                model: "gpt-6.1-sol".into(),
            }),
        ),
        (
            "openai",
            Some("fixture-custom"),
            Some(SuggestionProvider::OpenAi {
                model: "fixture-custom".into(),
            }),
        ),
        ("openai", Some(""), None),
        ("unknown", None, None),
    ];
    for (provider, model, expected) in cases {
        assert_eq!(
            clip_suggest::choose_suggestion(provider, model).ok(),
            expected,
            "{provider}/{model:?}"
        );
    }
    let credentials = Credentials::new(None, None, None);
    assert!(
        clip_suggest::check_suggestion_credentials(&SuggestionProvider::Heuristic, &credentials)
            .is_ok()
    );
    assert!(
        clip_suggest::check_suggestion_credentials(
            &SuggestionProvider::OpenAi {
                model: "gpt-6-luna".into()
            },
            &credentials
        )
        .is_err()
    );
}

//! Offline clip workflow contracts: ASR-to-input snapping, explicit provider
//! choice and export preparation across source/translated captions and hostile
//! selections.
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
use engine::{
    ByteBudget, ClipSettings,
    jobs::{JobRunner, RunnerConfig, SystemClock},
};
use shorts::{
    ClipExportRequest, ClipRequest, SnapSettings,
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
            Err(shorts::Error::Engine(engine::EditError::Runtime(
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
fn export_preparation_covers_source_and_translated_captions() {
    let Some(input) = fixture() else {
        return;
    };
    let providers = providers(true);
    let (mut runner, _budget) = runner();
    let mut outcome = suggest(&input, &providers, &mut runner);
    let directory = tempfile::tempdir().unwrap();
    let mut prepare = |select: Vec<usize>, languages: Vec<LanguageTag>| {
        clip_suggest::prepare_export(
            &outcome,
            ClipExportRequest {
                directory: directory.path().to_path_buf(),
                select,
                languages,
                formats: Vec::new(),
                overwrite: false,
                allow_degraded: false,
                max_output_bytes: 1 << 30,
            },
            &providers,
            &mut runner,
            &AllowAll,
            &SilentProgress,
            &Cancellation::new(),
        )
    };
    let source = prepare(vec![0], vec![lang("en")]).unwrap();
    assert_eq!(source.batch.len(), 1);
    assert_eq!(
        source
            .batch
            .paths(0)
            .unwrap()
            .video
            .extension()
            .and_then(|value| value.to_str()),
        Some("mp4")
    );
    let settings = source.batch.settings(0).unwrap();
    assert_eq!(
        (settings.render.width, settings.render.height),
        (1080, 1920)
    );

    let translated = prepare(vec![0], vec![lang("vi")]).unwrap();
    assert_eq!(translated.batch.len(), 1);
    assert!(matches!(
        translated.translation_cost,
        engine::providers::Cost::Free
    ));

    for select in [vec![outcome.clips.len()], vec![0, 0], vec![]] {
        assert!(prepare(select, Vec::new()).is_err());
    }
    drop(prepare);

    // Table row: nine selected clips exceed the CLI's eight-slot runner queue.
    // Keep the configured export workers busy without overflowing the queue.
    let template = outcome.clips[0].clone();
    outcome.clips = (0..9)
        .map(|index| {
            let mut clip = template.clone();
            clip.proposal.title = format!("nine-{index}").into();
            // Three frames cover scheduling without encoding nine full videos.
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
    let (mut runner, _) = runner_with_exports(2, 8, 8);
    let progress = ExportProgress::default();
    let exported = clip_suggest::export(
        &outcome,
        ClipExportRequest {
            directory: directory.path().to_path_buf(),
            select: (0..9).collect(),
            languages: Vec::new(),
            formats: Vec::new(),
            overwrite: false,
            allow_degraded: false,
            max_output_bytes: 1 << 30,
        },
        &providers,
        &mut runner,
        &DenyUploads,
        &progress,
        &Cancellation::new(),
    )
    .unwrap();
    assert_eq!(exported.exports.len(), 9);
    assert_eq!(progress.peak(), 2, "runner export capacity was underused");
    let (pending, active, _, _) = runner.counts();
    assert_eq!(
        (pending, active),
        (0, 0),
        "nine-clip export left admitted jobs pending or active"
    );

    // Window two / pending queue one necessarily refuses the second unpumped
    // submission. The workflow must pump and retry that exact admission error.
    let queued_directory = tempfile::tempdir().unwrap();
    let (mut congested, budget) = runner_with_exports(2, 1, 8);
    let exported = clip_suggest::export(
        &outcome,
        ClipExportRequest {
            directory: queued_directory.path().to_path_buf(),
            select: (0..3).collect(),
            languages: Vec::new(),
            formats: Vec::new(),
            overwrite: false,
            allow_degraded: false,
            max_output_bytes: 1 << 30,
        },
        &providers,
        &mut congested,
        &DenyUploads,
        &SilentProgress,
        &Cancellation::new(),
    )
    .unwrap();
    assert_eq!(exported.exports.len(), 3);
    assert_eq!(
        std::fs::read_dir(queued_directory.path()).unwrap().count(),
        3
    );
    assert_eq!((congested.counts().0, congested.counts().1), (0, 0));
    assert_eq!(budget.snapshot().unwrap().used, 0);

    let cancelled_directory = tempfile::tempdir().unwrap();
    let cancellation = Cancellation::new();
    let progress = ExportProgress {
        cancel: Some(cancellation.clone()),
        ..Default::default()
    };
    let result = clip_suggest::export(
        &outcome,
        ClipExportRequest {
            directory: cancelled_directory.path().to_path_buf(),
            select: (0..9).collect(),
            languages: Vec::new(),
            formats: Vec::new(),
            overwrite: false,
            allow_degraded: false,
            max_output_bytes: 1 << 30,
        },
        &providers,
        &mut runner,
        &DenyUploads,
        &progress,
        &cancellation,
    );
    assert!(matches!(
        result,
        Err(shorts::Error::Engine(engine::EditError::Cancelled))
    ));
    let (pending, active, _, _) = runner.counts();
    assert_eq!(
        (pending, active),
        (0, 0),
        "cancelled exports were not drained"
    );
}

#[derive(Default)]
struct ExportProgress {
    state: Mutex<(BTreeSet<engine::jobs::JobId>, usize)>,
    cancel: Option<Cancellation>,
}
impl ExportProgress {
    fn peak(&self) -> usize {
        self.state.lock().unwrap().1
    }
}
impl engine::transcript::Progress for ExportProgress {
    fn stage(&self, _: engine::transcript::Stage) {}
    fn job(&self, event: &engine::jobs::JobSnapshot) {
        let mut state = self.state.lock().unwrap();
        match event.state {
            engine::jobs::JobState::Running => {
                state.0.insert(event.id);
            }
            engine::jobs::JobState::Completed
            | engine::jobs::JobState::Failed
            | engine::jobs::JobState::Cancelled => {
                state.0.remove(&event.id);
            }
            _ => {}
        }
        state.1 = state.1.max(state.0.len());
        if state.0.len() == 2
            && let Some(cancel) = &self.cancel
        {
            cancel.cancel();
        }
    }
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
        ("openai", Some("other"), None),
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

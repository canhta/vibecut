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
use engine::subtitles::{
    AllowAll, Cancellation, Credentials, DenyUploads, LanguageTag, Providers, SilentProgress,
    TranscriptionProvider, TranslationProvider,
};
use engine::{
    ByteBudget, ClipExportRequest, ClipRequest, ClipSettings, SnapSettings,
    clip_suggest::{self, SuggestionProvider},
    jobs::{JobRunner, RunnerConfig, SystemClock},
};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
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
            let directory =
                Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/fixtures/engine");
            std::fs::create_dir_all(&directory).ok()?;
            let path = directory.join("clips-2s.mp4");
            if std::fs::metadata(&path).is_ok_and(|meta| meta.len() > 0) {
                return Some(path);
            }
            let output = std::process::Command::new("ffmpeg")
                .env_remove("DYLD_LIBRARY_PATH")
                .env_remove("LD_LIBRARY_PATH")
                .args([
                    "-y",
                    "-hide_banner",
                    "-loglevel",
                    "error",
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
                ])
                .arg(&path)
                .output()
                .ok()?;
            if !output.status.success() {
                assert_ne!(
                    std::env::var("VIBECUT_REQUIRE_ORACLES").as_deref(),
                    Ok("1"),
                    "ffmpeg fixture generation failed: {}",
                    String::from_utf8_lossy(&output.stderr)
                );
                return None;
            }
            Some(path)
        })
        .clone()
}

fn config(provider: &str, model: &str) -> ProviderConfig {
    ProviderConfig::new(
        provider.into(),
        model.into(),
        format!("{provider}-settings-v1").into(),
        Location::Local {
            backend: "fixture".into(),
            model_ref: "fixture".into(),
        },
    )
    .unwrap()
}

/// Three sentences across the whole extracted window, so the heuristic has
/// deterministic sentence boundaries with no real model.
struct FakeAsr {
    config: ProviderConfig,
}
impl FakeAsr {
    fn new() -> Self {
        Self {
            config: config("fake-asr", "fake-asr-model"),
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
            config: config("fake-translator", "fake-translator-model"),
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
    let budget = ByteBudget::new(2 << 30).unwrap();
    (
        JobRunner::new(
            RunnerConfig {
                concurrency: [1, 1, 2],
                queue: 8,
                retained: 8,
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
) -> engine::ClipSuggestionOutcome {
    let root = tempfile::tempdir().unwrap();
    let (mut cache, _) =
        engine::cache::ArtifactCache::open(root.path(), ByteBudget::new(1 << 30).unwrap(), 8)
            .unwrap();
    let credentials = Credentials::new(None, None, None).unwrap();
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
fn export_preparation_covers_source_and_translated_captions() {
    let Some(input) = fixture() else {
        return;
    };
    let providers = providers(true);
    let (mut runner, _budget) = runner();
    let outcome = suggest(&input, &providers, &mut runner);
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
        engine::subtitles::Cost::Free
    ));

    for select in [vec![outcome.clips.len()], vec![0, 0], vec![]] {
        assert!(prepare(select, Vec::new()).is_err());
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
    let credentials = Credentials::new(None, None, None).unwrap();
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

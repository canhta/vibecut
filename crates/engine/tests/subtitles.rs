//! Offline subtitle workflow contracts: six directions, bilingual output, consent,
//! checkpoint reuse and the production SRT/burn paths on a generated fixture.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
use ai::{
    Capabilities, CostEstimate, Error, Evidence, ExecutionHooks, Location, Operation,
    ProviderConfig, RecognitionSegment, Request, Scope, Support, TimingUnit, Transcriber,
    Translation, TranslationWindow, Translator,
};
use engine::{
    ByteBudget,
    jobs::{JobRunner, RunnerConfig, SystemClock},
    subtitles::{
        self, AllowAll, Credentials, DenyUploads, LanguageTag, Progress, Providers, Runtime,
        SilentProgress, SubtitleRequest, TranscriptionProvider, TranslationProvider,
    },
};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::{
        Arc, OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

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
            let path = directory.join("subtitle-2s.mp4");
            if std::fs::metadata(&path).is_ok_and(|meta| meta.len() > 0) {
                return Some(path);
            }
            let output = std::process::Command::new("ffmpeg")
                .args([
                    "-y",
                    "-hide_banner",
                    "-loglevel",
                    "error",
                    "-f",
                    "lavfi",
                    "-i",
                    "color=c=black:s=320x240:r=10:d=2",
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

struct FakeAsr {
    config: ProviderConfig,
    calls: Arc<AtomicUsize>,
}
impl FakeAsr {
    fn new(calls: Arc<AtomicUsize>) -> Self {
        Self {
            config: config("fake-asr", "fake-asr-model"),
            calls,
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
        self.calls.fetch_add(1, Ordering::Relaxed);
        let text = match request.scope.source_language.as_str() {
            "vi" => "xin chào",
            "zh" => "你好",
            _ => "hello",
        };
        Ok(vec![RecognitionSegment {
            id: format!("fake:{}", audio.range.start()).into(),
            text: text.into(),
            range: Some(audio.range),
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
            languages: vec![
                lang("vi"),
                lang("en"),
                lang("zh"),
                lang("zh-Hans"),
                lang("zh-Hant"),
            ],
            target_languages: vec![
                lang("vi"),
                lang("en"),
                lang("zh"),
                lang("zh-Hans"),
                lang("zh-Hant"),
            ],
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

fn providers(calls: Arc<AtomicUsize>, remote: bool, translation: bool) -> Providers {
    let transcription = if remote {
        TranscriptionProvider::OpenAi {
            model: "fake-asr-model".into(),
        }
    } else {
        TranscriptionProvider::Whisper {
            model: "fake-asr-model".into(),
        }
    };
    Providers::from_parts(
        transcription,
        Arc::new(FakeAsr::new(calls)),
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
                concurrency: [1, 1, 1],
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

fn request(
    input: &Path,
    source: &str,
    target: &str,
    directory: &Path,
    burn: bool,
) -> SubtitleRequest {
    SubtitleRequest {
        input: input.to_path_buf(),
        source_language: Some(lang(source)),
        target_language: lang(target),
        bilingual: false,
        srt: Some(directory.join(format!("{source}-{target}.srt"))),
        burn: burn.then(|| directory.join(format!("{source}-{target}.mp4"))),
        portrait: None,
        overwrite: false,
    }
}

#[test]
fn six_directions_and_every_bilingual_pair() {
    let Some(input) = fixture() else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let providers = providers(calls.clone(), false, true);
    let (mut runner, budget) = runner();
    let (mut cache, _) =
        engine::cache::ArtifactCache::open(&directory.path().join("cache"), budget, 16).unwrap();
    let pairs = [
        ("vi", "en"),
        ("en", "vi"),
        ("vi", "zh-Hans"),
        ("zh", "vi"),
        ("en", "zh-Hans"),
        ("zh", "en"),
    ];
    for (source, target) in pairs {
        for bilingual in [false, true] {
            let mut request = request(&input, source, target, directory.path(), false);
            request.bilingual = bilingual;
            request.srt = Some(
                directory
                    .path()
                    .join(format!("{source}-{target}-{bilingual}.srt")),
            );
            let outcome = subtitles::run(
                &request,
                &providers,
                &mut runner,
                &mut cache,
                &AllowAll,
                &SilentProgress,
            )
            .unwrap();
            assert_eq!(outcome.source_language.as_str(), source);
            assert_eq!(outcome.target_language.as_str(), target);
            assert!(outcome.cues > 0);
            let text = std::fs::read_to_string(request.srt.as_ref().unwrap()).unwrap();
            let source_text = match source {
                "vi" => "xin chào",
                "zh" => "你好",
                _ => "hello",
            };
            assert!(text.contains(source_text), "{source}->{target}: {text}");
            assert!(
                text.contains(&format!("[{target}]")),
                "{source}->{target}: {text}"
            );
            if bilingual {
                assert!(
                    text.contains(source_text) && text.contains(&format!("[{target}]")),
                    "bilingual {source}->{target}: {text}"
                );
            }
        }
    }
}

#[test]
fn output_languages_are_distinct() {
    let Some(input) = fixture() else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let providers = providers(calls, false, true);
    let (mut runner, budget) = runner();
    let (mut cache, _) =
        engine::cache::ArtifactCache::open(&directory.path().join("cache"), budget, 16).unwrap();
    let mut request = request(&input, "vi", "en", directory.path(), false);
    request.bilingual = true;
    let outcome = subtitles::run(
        &request,
        &providers,
        &mut runner,
        &mut cache,
        &AllowAll,
        &SilentProgress,
    )
    .unwrap();
    assert!(outcome.translation.is_some());
    let text = std::fs::read_to_string(request.srt.as_ref().unwrap()).unwrap();
    assert!(text.contains("xin chào"));
    assert!(text.contains("[en]"));
}

#[test]
fn srt_and_burn_are_produced_from_one_run() {
    let Some(input) = fixture() else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let providers = providers(calls, false, true);
    let (mut runner, budget) = runner();
    let (mut cache, _) =
        engine::cache::ArtifactCache::open(&directory.path().join("cache"), budget, 16).unwrap();
    let request = request(&input, "vi", "en", directory.path(), true);
    let outcome = subtitles::run(
        &request,
        &providers,
        &mut runner,
        &mut cache,
        &AllowAll,
        &SilentProgress,
    )
    .unwrap();
    assert!(outcome.srt.is_some());
    assert!(outcome.burn.is_some());
    let srt = std::fs::read_to_string(request.srt.as_ref().unwrap()).unwrap();
    assert!(srt.contains("xin chào"));
    assert!(
        std::fs::metadata(request.burn.as_ref().unwrap())
            .unwrap()
            .len()
            > 0
    );
}

#[test]
fn second_run_reuses_completed_asr_chunks() {
    let Some(input) = fixture() else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let providers = providers(calls.clone(), false, true);
    let (mut runner, budget) = runner();
    let (mut cache, _) =
        engine::cache::ArtifactCache::open(&directory.path().join("cache"), budget, 16).unwrap();
    let mut request = request(&input, "vi", "en", directory.path(), false);
    request.srt = Some(directory.path().join("first.srt"));
    let first = subtitles::run(
        &request,
        &providers,
        &mut runner,
        &mut cache,
        &AllowAll,
        &SilentProgress,
    )
    .unwrap();
    assert_eq!(first.checkpoints, 0);
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    request.srt = Some(directory.path().join("second.srt"));
    let second = subtitles::run(
        &request,
        &providers,
        &mut runner,
        &mut cache,
        &AllowAll,
        &SilentProgress,
    )
    .unwrap();
    assert_eq!(second.checkpoints, 1);
    assert_eq!(calls.load(Ordering::Relaxed), 1, "ASR was not re-executed");
}

#[test]
fn remote_upload_is_refused_without_consent() {
    let Some(input) = fixture() else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let providers = providers(calls.clone(), true, true);
    let (mut runner, budget) = runner();
    let (mut cache, _) =
        engine::cache::ArtifactCache::open(&directory.path().join("cache"), budget, 16).unwrap();
    let request = request(&input, "vi", "en", directory.path(), false);
    let error = subtitles::run(
        &request,
        &providers,
        &mut runner,
        &mut cache,
        &DenyUploads,
        &SilentProgress,
    )
    .unwrap_err();
    assert!(matches!(error, engine::EditError::ConsentRequired));
    assert_eq!(calls.load(Ordering::Relaxed), 0);
}

#[test]
fn provider_catalog_filters_operation_and_language() {
    let vi = lang("vi");
    let transcribe =
        subtitles::provider_options_for(subtitles::CatalogOperation::Transcribe, Some(&vi));
    assert!(transcribe.iter().any(|option| option.id == "whisper.cpp"));
    assert!(transcribe.iter().any(|option| option.id == "gipformer"));
    assert!(transcribe.iter().any(|option| option.id == "openai"));
    let english =
        subtitles::provider_options_for(subtitles::CatalogOperation::Transcribe, Some(&lang("en")));
    assert!(!english.iter().any(|option| option.id == "gipformer"));
    let translate = subtitles::provider_options_for(
        subtitles::CatalogOperation::Translate,
        Some(&lang("zh-Hant")),
    );
    assert_eq!(translate.len(), 2);
    assert!(matches!(
        subtitles::choose_transcription("gipformer", None),
        Ok(TranscriptionProvider::Gipformer { .. })
    ));
    assert!(subtitles::choose_transcription("openai", Some("not-a-model")).is_err());
}

#[test]
fn missing_remote_credentials_are_typed() {
    let directory = tempfile::tempdir().unwrap();
    let budget = ByteBudget::new(2 << 30).unwrap();
    let (mut cache, _) =
        engine::cache::ArtifactCache::open(&directory.path().join("cache"), budget, 16).unwrap();
    let error = Providers::open(
        TranscriptionProvider::OpenAi {
            model: "gpt-transcribe".into(),
        },
        None,
        &mut cache,
        &Credentials::new(None, None, None).unwrap(),
        &Runtime::default(),
    )
    .err()
    .unwrap();
    assert!(matches!(
        error,
        engine::EditError::CredentialMissing("OPENAI_API_KEY")
    ));
}

#[test]
fn progress_is_presentation_only_and_optional() {
    // SilentProgress is the documented no-op implementation for non-interactive use.
    SilentProgress.stage(subtitles::Stage::Extract);
}

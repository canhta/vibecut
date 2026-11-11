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
    providers::{
        AllowAll, Credentials, DenyUploads, Providers, Runtime, TranscriptionProvider,
        TranslationProvider,
    },
    subtitles::{self, LanguageTag, SubtitleRequest},
    transcript::{Cancellation, Progress, SilentProgress},
};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::Duration,
};

fn lang(value: &str) -> LanguageTag {
    LanguageTag::parse(value).unwrap()
}

fn fixture() -> Option<PathBuf> {
    static FIXTURE: OnceLock<Option<PathBuf>> = OnceLock::new();
    FIXTURE
        .get_or_init(|| {
            testkit::oracle::generate(
                &testkit::fixture::dir("engine"),
                "subtitle fixture",
                "subtitle-2s.mp4",
                &[
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
                ],
            )
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

fn remote_config(provider: &str, model: &str) -> ProviderConfig {
    ProviderConfig::new(
        provider.into(),
        model.into(),
        format!("{provider}-settings-v1").into(),
        Location::Remote {
            backend: "fixture".into(),
            base_url: "https://fixture.invalid".into(),
            auth_ref: "fixture".into(),
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
            config: remote_config("fake-translator", "fake-translator-model"),
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

pub(crate) fn providers(calls: Arc<AtomicUsize>, remote: bool, translation: bool) -> Providers {
    let mut asr = FakeAsr::new(calls);
    if remote {
        asr.config = remote_config("fake-asr", "fake-asr-model");
    }
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
        Arc::new(asr),
        None,
        translation.then(|| TranslationProvider::OpenAi {
            model: "fake-translator-model".into(),
        }),
        translation.then(|| Arc::new(FakeTranslator::new()) as Arc<dyn Translator>),
    )
    .unwrap()
}

fn runner() -> (JobRunner, ByteBudget) {
    runner_with_limit(2 << 30)
}

fn runner_with_limit(limit: u64) -> (JobRunner, ByteBudget) {
    let budget = ByteBudget::new(limit).unwrap();
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
                &engine::transcript::Cancellation::new(),
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
        &engine::transcript::Cancellation::new(),
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
        &engine::transcript::Cancellation::new(),
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
        &engine::transcript::Cancellation::new(),
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
        &engine::transcript::Cancellation::new(),
    )
    .unwrap();
    assert_eq!(second.checkpoints, 1);
    assert_eq!(calls.load(Ordering::Relaxed), 1, "ASR was not re-executed");
}

#[test]
fn blocking_generation_refuses_shared_budget_pressure_before_asr_and_releases_admission() {
    let Some(input) = fixture() else { return };
    for (limit, occupied, alignment) in [
        ((64 << 20) - 1, 0, false),
        ((80 << 20) - 1, 0, false),
        (2 << 30, (2 << 30) - (64 << 20) + 1, false),
        (2 << 30, (2 << 30) - (80 << 20) + 1, false),
        (2 << 30, (2 << 30) - (80 << 20) + 1, true),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let (mut runner, budget) = runner_with_limit(limit);
        let held = budget.reserve(engine::ByteOwner::Export, occupied).unwrap();
        let (mut cache, _) =
            engine::cache::ArtifactCache::open(&directory.path().join("cache"), budget.clone(), 16)
                .unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let align_calls = Arc::new(AtomicUsize::new(0));
        let providers = if alignment {
            text_only_providers(
                TranscriptionProvider::OpenAi {
                    model: "fake-asr-model".into(),
                },
                calls.clone(),
                Arc::new(ProbeAligner::new(align_calls.clone())),
            )
        } else {
            providers(calls.clone(), true, false)
        };
        let req = request(&input, "en", "en", directory.path(), false);
        let before = budget.snapshot().unwrap().used;
        let result = subtitles::run(
            &req,
            &providers,
            &mut runner,
            &mut cache,
            &AllowAll,
            &SilentProgress,
            &Cancellation::new(),
        );
        assert_eq!(
            calls.load(Ordering::Relaxed),
            0,
            "pressure must refuse before upload"
        );
        assert!(matches!(
            result,
            Err(engine::EditError::Runtime(
                engine::RuntimeError::BudgetFull | engine::RuntimeError::ArtifactTooLarge
            ))
        ));
        assert!(!req.srt.as_ref().unwrap().exists());
        assert_eq!(align_calls.load(Ordering::Relaxed), 0);
        assert_eq!(
            budget.snapshot().unwrap().used,
            before,
            "partial admission must roll back"
        );
        drop(held);
        if limit < 80 << 20 {
            continue;
        }
        let outcome = subtitles::run(
            &req,
            &providers,
            &mut runner,
            &mut cache,
            &AllowAll,
            &SilentProgress,
            &Cancellation::new(),
        )
        .unwrap();
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert_eq!(outcome.checkpoints, 0);
        assert_eq!(align_calls.load(Ordering::Relaxed), usize::from(alignment));
        drop(runner);
        drop(cache);
        assert_eq!(budget.snapshot().unwrap().used, 0);
    }
}

#[test]
fn registry_generation_uses_its_outer_admission_without_charging_blocking_work_twice() {
    use crate::support::control::finish;
    let Some(input) = fixture() else { return };
    let dir = tempfile::tempdir().unwrap();
    let (mut r, budget) = runner_with_limit(72 << 20);
    let (cache, _) =
        engine::cache::ArtifactCache::open(&dir.path().join("cache"), budget.clone(), 16).unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let access = subtitles::CaptionAccess::new(
        Arc::new(providers(calls.clone(), false, false)),
        Arc::new(AllowAll),
        Arc::new(std::sync::Mutex::new(cache)),
    );
    let mut s = registry_session(&input);
    let before = s.project().clone();
    let job = caption_submit(
        &mut s,
        &mut r,
        &access,
        "captions.generate",
        serde_json::json!({"asset":1,"source_language":"en"}),
        "generate",
    );
    finish(&mut r, job);
    let snap = r.snapshot(job).unwrap();
    assert_eq!(
        snap.state,
        engine::jobs::JobState::Completed,
        "{:?}",
        snap.error
    );
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    assert_eq!(s.project(), &before);
    assert!(
        !subtitles::decode_captions(r.output_bytes(job).unwrap())
            .unwrap()
            .cues
            .is_empty()
    );
    drop(r);
    drop(access);
    assert_eq!(budget.snapshot().unwrap().used, 0);
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
        &engine::transcript::Cancellation::new(),
    )
    .unwrap_err();
    assert!(matches!(
        error,
        engine::EditError::Providers(engine::providers::Error::ConsentRequired)
    ));
    assert_eq!(calls.load(Ordering::Relaxed), 0);
}

#[test]
fn checkpoint_envelope_miss_renews_consent_but_bad_json_never_reruns_asr() {
    let Some(input) = fixture() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let providers = providers(calls.clone(), true, false);
    let (mut runner, budget) = runner();
    let root = dir.path().join("cache");
    let (mut cache, _) = engine::cache::ArtifactCache::open(&root, budget.clone(), 16).unwrap();
    let mut req = request(&input, "en", "en", dir.path(), false);
    let first = subtitles::run(
        &req,
        &providers,
        &mut runner,
        &mut cache,
        &AllowAll,
        &SilentProgress,
        &Cancellation::new(),
    )
    .unwrap();
    assert_eq!(first.transcription_cost, engine::providers::Cost::Free);
    let old_output = req.srt.clone().unwrap();
    let preserved = std::fs::read(&old_output).unwrap();
    let path = std::fs::read_dir(&root)
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.file_name().unwrap() != ".cache.lock")
        .unwrap();
    let mut bytes = std::fs::read(&path).unwrap();
    bytes[72] ^= 1;
    std::fs::write(&path, bytes).unwrap();
    req.srt = Some(dir.path().join("regenerated.srt"));
    assert!(matches!(
        subtitles::run(
            &req,
            &providers,
            &mut runner,
            &mut cache,
            &DenyUploads,
            &SilentProgress,
            &Cancellation::new()
        ),
        Err(engine::EditError::Providers(
            engine::providers::Error::ConsentRequired
        ))
    ));
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    assert!(!req.srt.as_ref().unwrap().exists());
    let regenerated = subtitles::run(
        &req,
        &providers,
        &mut runner,
        &mut cache,
        &AllowAll,
        &SilentProgress,
        &Cancellation::new(),
    )
    .unwrap();
    assert_eq!(regenerated.checkpoints, 0);
    assert_eq!(
        regenerated.transcription_cost,
        engine::providers::Cost::Free
    );
    assert_eq!(calls.load(Ordering::Relaxed), 2);
    drop(cache);
    let second_output = req.srt.clone().unwrap();
    let second_preserved = std::fs::read(&second_output).unwrap();
    let mut envelope = std::fs::read(&path).unwrap();
    envelope.truncate(72);
    envelope[40..72].copy_from_slice(blake3::hash(b"not JSON").as_bytes());
    envelope.extend_from_slice(b"not JSON");
    std::fs::write(&path, &envelope).unwrap();
    let (mut cache, diagnostics) =
        engine::cache::ArtifactCache::open(&root, budget.clone(), 16).unwrap();
    assert_eq!(diagnostics, 0);
    req.srt = Some(dir.path().join("must-not-publish.srt"));
    assert!(matches!(
        subtitles::run(
            &req,
            &providers,
            &mut runner,
            &mut cache,
            &AllowAll,
            &SilentProgress,
            &Cancellation::new()
        ),
        Err(engine::EditError::Invalid(
            "cached recognition is malformed"
        ))
    ));
    assert_eq!(calls.load(Ordering::Relaxed), 2);
    assert_eq!(std::fs::read(&path).unwrap(), envelope);
    assert_eq!(std::fs::read(old_output).unwrap(), preserved);
    assert_eq!(std::fs::read(second_output).unwrap(), second_preserved);
    assert!(!req.srt.as_ref().unwrap().exists());
    let cancel = Cancellation::new();
    cancel.cancel();
    assert!(matches!(
        subtitles::run(
            &req,
            &providers,
            &mut runner,
            &mut cache,
            &AllowAll,
            &SilentProgress,
            &cancel
        ),
        Err(engine::EditError::Cancelled)
    ));
    assert_eq!(calls.load(Ordering::Relaxed), 2);
    assert_eq!(cache.clear().unwrap(), 0);
    drop(runner);
    assert_eq!(budget.snapshot().unwrap().used, 0);
}

#[test]
fn provider_catalog_filters_operation_and_language() {
    let vi = lang("vi");
    let transcribe = engine::providers::provider_options_for(
        engine::providers::CatalogOperation::Transcribe,
        Some(&vi),
    );
    assert!(transcribe.iter().any(|option| option.id == "whisper.cpp"));
    assert!(transcribe.iter().any(|option| option.id == "gipformer"));
    assert!(transcribe.iter().any(|option| option.id == "openai"));
    let english = engine::providers::provider_options_for(
        engine::providers::CatalogOperation::Transcribe,
        Some(&lang("en")),
    );
    assert!(!english.iter().any(|option| option.id == "gipformer"));
    let translate = engine::providers::provider_options_for(
        engine::providers::CatalogOperation::Translate,
        Some(&lang("zh-Hant")),
    );
    assert_eq!(translate.len(), 2);
    assert!(matches!(
        engine::providers::choose_transcription("gipformer", None),
        Ok(TranscriptionProvider::Gipformer { .. })
    ));
    assert!(engine::providers::choose_transcription("openai", Some("not-a-model")).is_err());
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
        &Credentials::new(None, None, None),
        &Runtime::default(),
    )
    .err()
    .unwrap();
    assert!(matches!(
        error,
        engine::EditError::Providers(engine::providers::Error::CredentialMissing(
            "OPENAI_API_KEY"
        ))
    ));
}

struct TextOnlyAsr(FakeAsr);
impl Transcriber for TextOnlyAsr {
    fn config(&self) -> &ProviderConfig {
        self.0.config()
    }
    fn capabilities(&self, scope: &Scope) -> Capabilities {
        let mut value = self.0.capabilities(scope);
        value.timing.clear();
        value
    }
    fn estimate(&self, _: &Scope) -> CostEstimate {
        CostEstimate::Free
    }
    fn transcribe(
        &self,
        audio: ai::AudioWindow<'_>,
        request: &Request<'_>,
        hooks: &dyn ExecutionHooks,
    ) -> Result<Vec<RecognitionSegment>, Error> {
        let mut segments = self.0.transcribe(audio, request, hooks)?;
        for segment in &mut segments {
            segment.range = None;
        }
        Ok(segments)
    }
}

struct ProbeAligner {
    config: ProviderConfig,
    calls: Arc<AtomicUsize>,
    fail_first: bool,
}
impl ProbeAligner {
    fn new(calls: Arc<AtomicUsize>) -> Self {
        Self {
            config: config("probe-align", "probe"),
            calls,
            fail_first: false,
        }
    }
    fn failing_once(calls: Arc<AtomicUsize>) -> Self {
        Self {
            config: config("probe-align", "probe"),
            calls,
            fail_first: true,
        }
    }
}
impl ai::Aligner for ProbeAligner {
    fn config(&self) -> &ProviderConfig {
        &self.config
    }
    fn capabilities(&self, _: &Scope) -> Capabilities {
        Capabilities {
            model: self.config.model().clone(),
            mode: ai::alignment::MODE.into(),
            operation: Operation::Align,
            languages: vec![lang("en")],
            target_languages: Vec::new(),
            timing: BTreeSet::from([TimingUnit::Segment, TimingUnit::Word]),
            diarization: Support::Unsupported,
            live_audio_input: Support::Unsupported,
            file_result_stream: Support::Unsupported,
            max_upload_bytes: None,
            max_duration_micros: None,
            evidence: Evidence::FixtureVerified,
            revision: "probe".into(),
        }
    }
    fn estimate(&self, _: &Scope) -> CostEstimate {
        CostEstimate::Free
    }
    fn align(
        &self,
        audio: ai::AudioWindow<'_>,
        missing: &[RecognitionSegment],
        _: &Request<'_>,
        _: &dyn ExecutionHooks,
    ) -> Result<Vec<RecognitionSegment>, Error> {
        let call = self.calls.fetch_add(1, Ordering::Relaxed);
        if self.fail_first && call == 0 {
            return Err(Error::Inference);
        }
        Ok(missing
            .iter()
            .cloned()
            .map(|mut segment| {
                segment.range = Some(audio.range);
                segment.timing = vec![ai::TimedText {
                    text: segment.text.clone(),
                    unit: TimingUnit::Word,
                    range: audio.range,
                    provenance: ai::Provenance::Forced,
                    speaker: None,
                    confidence: None,
                }];
                segment
            })
            .collect())
    }
}

fn text_only_providers(
    transcription: TranscriptionProvider,
    asr_calls: Arc<AtomicUsize>,
    aligner: Arc<ProbeAligner>,
) -> Providers {
    let mut asr = FakeAsr::new(asr_calls);
    if transcription.is_remote() {
        asr.config = remote_config("fake-asr", "fake-asr-model");
    }
    Providers::from_parts(
        transcription,
        Arc::new(TextOnlyAsr(asr)),
        Some(aligner),
        None,
        None,
    )
    .unwrap()
}

#[test]
fn text_only_asr_runs_the_local_aligner() {
    let Some(input) = fixture() else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let asr_calls = Arc::new(AtomicUsize::new(0));
    let align_calls = Arc::new(AtomicUsize::new(0));
    let (mut runner, budget) = runner();
    let mut request = request(&input, "en", "en", directory.path(), false);
    request.overwrite = true;
    for transcription in [
        TranscriptionProvider::OpenAi {
            model: "fake-asr-model".into(),
        },
        TranscriptionProvider::Gipformer {
            model: "fake-asr-model".into(),
        },
    ] {
        let (mut cache, _) = engine::cache::ArtifactCache::open(
            &directory
                .path()
                .join(format!("cache-{}", transcription.id())),
            budget.clone(),
            16,
        )
        .unwrap();
        let providers = text_only_providers(
            transcription,
            asr_calls.clone(),
            Arc::new(ProbeAligner::new(align_calls.clone())),
        );
        let outcome = subtitles::run(
            &request,
            &providers,
            &mut runner,
            &mut cache,
            &AllowAll,
            &SilentProgress,
            &Cancellation::new(),
        )
        .unwrap();
        assert!(outcome.srt.is_some());
    }
    assert_eq!(asr_calls.load(Ordering::Relaxed), 2);
    assert_eq!(align_calls.load(Ordering::Relaxed), 2);
}

#[test]
fn alignment_failure_preserves_the_completed_asr_checkpoint() {
    let Some(input) = fixture() else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let asr_calls = Arc::new(AtomicUsize::new(0));
    let align_calls = Arc::new(AtomicUsize::new(0));
    let providers = text_only_providers(
        TranscriptionProvider::OpenAi {
            model: "fake-asr-model".into(),
        },
        asr_calls.clone(),
        Arc::new(ProbeAligner::failing_once(align_calls.clone())),
    );
    let (mut runner, budget) = runner();
    let (mut cache, _) =
        engine::cache::ArtifactCache::open(&directory.path().join("cache"), budget, 16).unwrap();
    let request = request(&input, "en", "en", directory.path(), false);
    let error = subtitles::run(
        &request,
        &providers,
        &mut runner,
        &mut cache,
        &AllowAll,
        &SilentProgress,
        &Cancellation::new(),
    )
    .unwrap_err();
    assert!(matches!(error, engine::EditError::Ai(ai::Error::Inference)));
    assert_eq!(asr_calls.load(Ordering::Relaxed), 1);
    let outcome = subtitles::run(
        &request,
        &providers,
        &mut runner,
        &mut cache,
        &AllowAll,
        &SilentProgress,
        &Cancellation::new(),
    )
    .unwrap();
    assert!(outcome.srt.is_some());
    assert_eq!(outcome.checkpoints, 1);
    assert_eq!(asr_calls.load(Ordering::Relaxed), 1);
    assert_eq!(align_calls.load(Ordering::Relaxed), 2);
}

struct BlockingAsr {
    config: ProviderConfig,
    calls: Arc<AtomicUsize>,
    entered: Arc<AtomicBool>,
}
impl Transcriber for BlockingAsr {
    fn config(&self) -> &ProviderConfig {
        &self.config
    }
    fn capabilities(&self, _: &Scope) -> Capabilities {
        FakeAsr::new(Arc::new(AtomicUsize::new(0))).capabilities(&Scope {
            provider: self.config.clone(),
            operation: Operation::Transcribe,
            payload_fingerprint: "probe".into(),
            mode: "transcribe".into(),
            texts: Vec::new(),
            source_language: lang("en"),
            target_language: None,
            source_samples: ai::SampleRange::new(0, 1).unwrap(),
            sample_rate: 1,
            max_bytes: 1,
        })
    }
    fn estimate(&self, _: &Scope) -> CostEstimate {
        CostEstimate::Free
    }
    fn transcribe(
        &self,
        _: ai::AudioWindow<'_>,
        _: &Request<'_>,
        hooks: &dyn ExecutionHooks,
    ) -> Result<Vec<RecognitionSegment>, Error> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        self.entered.store(true, Ordering::Release);
        while !hooks.cancelled() {
            thread::sleep(Duration::from_millis(1));
        }
        Err(Error::Cancelled)
    }
}

#[test]
fn operation_cancellation_stops_a_running_asr_job_without_output() {
    let Some(input) = fixture() else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let entered = Arc::new(AtomicBool::new(false));
    let transcriber = Arc::new(BlockingAsr {
        config: config("fake-asr", "fake-asr-model"),
        calls: calls.clone(),
        entered: entered.clone(),
    });
    let providers = Providers::from_parts(
        TranscriptionProvider::Whisper {
            model: "fake-asr-model".into(),
        },
        transcriber,
        None,
        None,
        None,
    )
    .unwrap();
    let (mut runner, budget) = runner();
    let (mut cache, _) =
        engine::cache::ArtifactCache::open(&directory.path().join("cache"), budget, 16).unwrap();
    let request = request(&input, "en", "en", directory.path(), false);
    let cancel = Cancellation::new();
    let observed = cancel.clone();
    let thread = thread::spawn(move || {
        while !entered.load(Ordering::Acquire) {
            thread::sleep(Duration::from_millis(1));
        }
        observed.cancel();
    });
    let error = subtitles::run(
        &request,
        &providers,
        &mut runner,
        &mut cache,
        &AllowAll,
        &SilentProgress,
        &cancel,
    )
    .unwrap_err();
    thread.join().unwrap();
    assert!(matches!(error, engine::EditError::Cancelled));
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    assert!(!request.srt.unwrap().exists());
}

struct CancelAtExport {
    cancel: Cancellation,
}
impl Progress for CancelAtExport {
    fn stage(&self, stage: engine::transcript::Stage) {
        if stage == engine::transcript::Stage::ExportSrt {
            self.cancel.cancel();
        }
    }
}

#[test]
fn cancellation_before_export_preserves_the_existing_output() {
    let Some(input) = fixture() else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("existing.srt");
    std::fs::write(&path, "keep this file").unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let providers = providers(calls, false, true);
    let (mut runner, budget) = runner();
    let (mut cache, _) =
        engine::cache::ArtifactCache::open(&directory.path().join("cache"), budget, 16).unwrap();
    let mut request = request(&input, "vi", "en", directory.path(), false);
    request.srt = Some(path.clone());
    let cancel = Cancellation::new();
    let progress = CancelAtExport {
        cancel: cancel.clone(),
    };
    let error = subtitles::run(
        &request,
        &providers,
        &mut runner,
        &mut cache,
        &AllowAll,
        &progress,
        &cancel,
    )
    .unwrap_err();
    assert!(matches!(error, engine::EditError::Cancelled));
    assert_eq!(std::fs::read_to_string(path).unwrap(), "keep this file");
}

struct BlockingTranslator {
    config: ProviderConfig,
    entered: Arc<AtomicBool>,
}
impl Translator for BlockingTranslator {
    fn config(&self) -> &ProviderConfig {
        &self.config
    }
    fn capabilities(&self, _: &Scope) -> Capabilities {
        FakeTranslator::new().capabilities(&Scope {
            provider: self.config.clone(),
            operation: Operation::Translate,
            payload_fingerprint: "probe".into(),
            mode: "context-window".into(),
            texts: Vec::new(),
            source_language: lang("vi"),
            target_language: Some(lang("en")),
            source_samples: ai::SampleRange::new(0, 1).unwrap(),
            sample_rate: 1,
            max_bytes: 1,
        })
    }
    fn estimate(&self, _: &Scope) -> CostEstimate {
        CostEstimate::Free
    }
    fn translate(
        &self,
        _: &TranslationWindow,
        _: &Request<'_>,
        hooks: &dyn ExecutionHooks,
    ) -> Result<Translation, Error> {
        self.entered.store(true, Ordering::Release);
        while !hooks.cancelled() {
            thread::sleep(Duration::from_millis(1));
        }
        Err(Error::Cancelled)
    }
}

#[test]
fn operation_cancellation_stops_a_running_translation_job() {
    let Some(input) = fixture() else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let entered = Arc::new(AtomicBool::new(false));
    let providers = Providers::from_parts(
        TranscriptionProvider::Whisper {
            model: "fake-asr-model".into(),
        },
        Arc::new(FakeAsr::new(calls)),
        None,
        Some(TranslationProvider::OpenAi {
            model: "fake-translator-model".into(),
        }),
        Some(Arc::new(BlockingTranslator {
            config: remote_config("fake-translator", "fake-translator-model"),
            entered: entered.clone(),
        })),
    )
    .unwrap();
    let (mut runner, budget) = runner();
    let (mut cache, _) =
        engine::cache::ArtifactCache::open(&directory.path().join("cache"), budget, 16).unwrap();
    let request = request(&input, "vi", "en", directory.path(), false);
    let cancel = Cancellation::new();
    let observed = cancel.clone();
    let thread = thread::spawn(move || {
        while !entered.load(Ordering::Acquire) {
            thread::sleep(Duration::from_millis(1));
        }
        observed.cancel();
    });
    let error = subtitles::run(
        &request,
        &providers,
        &mut runner,
        &mut cache,
        &AllowAll,
        &SilentProgress,
        &cancel,
    )
    .unwrap_err();
    thread.join().unwrap();
    assert!(matches!(error, engine::EditError::Cancelled));
    assert!(!request.srt.unwrap().exists());
}

struct CancelAtExtract {
    cancel: Cancellation,
}
impl Progress for CancelAtExtract {
    fn stage(&self, stage: engine::transcript::Stage) {
        if stage == engine::transcript::Stage::Extract {
            self.cancel.cancel();
        }
    }
}

#[test]
fn operation_cancellation_stops_extraction_before_any_asr() {
    let Some(input) = fixture() else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let providers = providers(calls.clone(), false, false);
    let (mut runner, budget) = runner();
    let (mut cache, _) =
        engine::cache::ArtifactCache::open(&directory.path().join("cache"), budget, 16).unwrap();
    let request = request(&input, "en", "en", directory.path(), false);
    let cancel = Cancellation::new();
    let progress = CancelAtExtract {
        cancel: cancel.clone(),
    };
    let error = subtitles::run(
        &request,
        &providers,
        &mut runner,
        &mut cache,
        &AllowAll,
        &progress,
        &cancel,
    )
    .unwrap_err();
    assert!(matches!(error, engine::EditError::Cancelled));
    assert_eq!(calls.load(Ordering::Relaxed), 0);
    assert!(!request.srt.unwrap().exists());
}

#[test]
fn progress_is_presentation_only_and_optional() {
    // SilentProgress is the documented no-op implementation for non-interactive use.
    SilentProgress.stage(engine::transcript::Stage::Extract);
}

fn registry_session(input: &Path) -> engine::Session {
    let mut s = crate::support::control::session();
    s.apply(engine::Command::AddAsset {
        asset: media::SourceAsset::import(media::AssetId(1), input.to_owned()).unwrap(),
    })
    .unwrap();
    s
}
fn caption_access(
    root: &Path,
    providers: Providers,
    consent: Arc<dyn engine::providers::ConsentPrompt + Send + Sync>,
) -> engine::subtitles::CaptionAccess {
    let (cache, _) =
        engine::cache::ArtifactCache::open(root, ByteBudget::new(512 << 20).unwrap(), 32).unwrap();
    engine::subtitles::CaptionAccess::new(
        Arc::new(providers),
        consent,
        Arc::new(std::sync::Mutex::new(cache)),
    )
}
fn caption_command(
    s: &mut engine::Session,
    r: &mut JobRunner,
    access: &engine::subtitles::CaptionAccess,
    id: &str,
    params: serde_json::Value,
    key: &str,
) -> engine::control::Response {
    let req = crate::support::control::request(s, id, params, key);
    let mut caller = crate::support::control::caller(&crate::support::control::Allow);
    caller.captions = Some(access);
    s.control_request(req, caller, r).unwrap()
}
fn caption_submit(
    s: &mut engine::Session,
    r: &mut JobRunner,
    access: &engine::subtitles::CaptionAccess,
    id: &str,
    params: serde_json::Value,
    key: &str,
) -> engine::jobs::JobId {
    let engine::control::ControlResult::JobAccepted { job, .. } =
        crate::support::control::ok(caption_command(s, r, access, id, params, key))
    else {
        panic!("expected job");
    };
    job
}
#[test]
fn registry_generation_applies_range_at_start_as_one_edit_and_refuses_foreign_stale_and_reused_results()
 {
    use crate::support::control::{Allow, caller, error, finish, ok, request};
    use engine::control::ErrorCode;
    use serde_json::json;
    use time::{SourceRange, TICKS_PER_SECOND, Tick};
    let Some(input) = fixture() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let access = caption_access(
        &dir.path().join("cache"),
        providers(Arc::new(AtomicUsize::new(0)), false, true),
        Arc::new(AllowAll),
    );
    let mut s = registry_session(&input);
    let before = s.project().clone();
    let rev = s.revision();
    let history = s.history_len();
    let (mut r, _) = runner();
    let range = SourceRange::new(Tick(TICKS_PER_SECOND), Tick(2 * TICKS_PER_SECOND)).unwrap();
    let job = caption_submit(
        &mut s,
        &mut r,
        &access,
        "captions.generate",
        json!({"asset":1,"source_language":"en","range":range}),
        "generate",
    );
    error(
        caption_command(
            &mut s,
            &mut r,
            &access,
            "captions.apply",
            json!({"job":job}),
            "pending",
        ),
        ErrorCode::StaleResult,
    );
    finish(&mut r, job);
    assert_eq!(
        r.snapshot(job).unwrap().state,
        engine::jobs::JobState::Completed
    );
    assert_eq!(s.project(), &before);
    assert_eq!(s.revision(), rev);
    assert_eq!(s.history_len(), history);
    let result = subtitles::decode_captions(r.output_bytes(job).unwrap()).unwrap();
    assert_eq!(result.range, range);
    assert_eq!(result.source_language, lang("en"));
    assert!(
        result
            .cues
            .iter()
            .all(|c| c.range.start() >= range.start() && c.range.end() <= range.end())
    );
    let mut foreign = engine::Session::new(before.as_ref().clone(), 10).unwrap();
    error(
        caption_command(
            &mut foreign,
            &mut r,
            &access,
            "captions.apply",
            json!({"job":job}),
            "foreign",
        ),
        ErrorCode::StaleResult,
    );
    error(
        caption_command(
            &mut s,
            &mut r,
            &access,
            "captions.apply",
            json!({"job":job,"start":-1}),
            "negative-start",
        ),
        ErrorCode::Refused,
    );
    let req = request(
        &s,
        "captions.apply",
        json!({"job":job,"start":3*TICKS_PER_SECOND}),
        "apply",
    );
    let response = s
        .control_request(req.clone(), caller(&Allow), &mut r)
        .unwrap();
    ok(response.clone());
    assert_ne!(s.revision(), rev);
    assert_eq!(s.history_len().0, history.0 + 1);
    let applied = s.project().clone();
    let item = applied
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|t| &t.items)
        .find(|i| matches!(i.content, project::ItemParams::Caption(_)))
        .unwrap();
    assert_eq!(item.start, Tick(3 * TICKS_PER_SECOND));
    assert_eq!(item.source_range, range);
    assert_eq!(item.source_asset, Some(media::AssetId(1)));
    let replay = s.control_request(req, caller(&Allow), &mut r).unwrap();
    assert_eq!(
        serde_json::to_value(response).unwrap(),
        serde_json::to_value(replay).unwrap()
    );
    error(
        caption_command(
            &mut s,
            &mut r,
            &access,
            "captions.apply",
            json!({"job":job}),
            "reused",
        ),
        ErrorCode::StaleResult,
    );
    s.undo().unwrap();
    assert_eq!(s.project(), &before);
    s.redo().unwrap();
    assert_eq!(s.project(), &applied);
    let job = caption_submit(
        &mut s,
        &mut r,
        &access,
        "captions.generate",
        json!({"asset":1,"source_language":"en"}),
        "stale-job",
    );
    finish(&mut r, job);
    s.apply(engine::Command::RenameProject {
        name: "later".into(),
    })
    .unwrap();
    error(
        caption_command(
            &mut s,
            &mut r,
            &access,
            "captions.apply",
            json!({"job":job}),
            "stale",
        ),
        ErrorCode::StaleResult,
    );
    let wrong = crate::support::control::submit(&mut r);
    finish(&mut r, wrong);
    error(
        caption_command(
            &mut s,
            &mut r,
            &access,
            "captions.apply",
            json!({"job":wrong}),
            "wrong-kind",
        ),
        ErrorCode::Refused,
    );
    for _ in 0..9 {
        let id = crate::support::control::submit(&mut r);
        finish(&mut r, id);
    }
    error(
        caption_command(
            &mut s,
            &mut r,
            &access,
            "captions.apply",
            json!({"job":job}),
            "evicted",
        ),
        ErrorCode::JobNotRetained,
    );
}

#[test]
fn registry_caption_admission_refuses_missing_access_setup_bad_ranges_grants_transactions_and_client_consent()
 {
    use crate::support::control::{Allow, Deny, caller, error, finish, request};
    use engine::control::ErrorCode;
    use serde_json::json;
    let Some(input) = fixture() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let access = caption_access(
        &dir.path().join("cache"),
        providers(calls.clone(), true, true),
        Arc::new(DenyUploads),
    );
    let mut s = registry_session(&input);
    let (mut r, _) = runner();
    let before = s.project().clone();
    let history = s.history_len();
    for (index, (id, params, expected)) in [
        ("captions.generate", json!({"asset":999,"source_language":"en"}), ErrorCode::Refused),
        ("captions.generate", json!({"asset":1,"range":{"start":-1,"end":1},"source_language":"en"}), ErrorCode::Refused),
        ("captions.generate", json!({"asset":1,"range":{"start":0,"end":3*time::TICKS_PER_SECOND},"source_language":"en"}), ErrorCode::Refused),
        ("captions.generate", json!({"asset":1,"source_language":"en","consent":true}), ErrorCode::InvalidParams),
        ("captions.translate", json!({"item":999,"target_language":"vi"}), ErrorCode::Refused),
        ("captions.apply", json!({"job":999}), ErrorCode::JobNotRetained),
    ].into_iter().enumerate() { error(caption_command(&mut s, &mut r, &access, id, params, &format!("refusal-{index}")), expected); }
    for policy in [&Allow as &dyn engine::control::Policy, &Deny] {
        let req = request(
            &s,
            "captions.generate",
            json!({"asset":1,"source_language":"en"}),
            "not-lent",
        );
        error(
            s.control_request(req, caller(policy), &mut r).unwrap(),
            ErrorCode::Permission,
        );
    }
    let unavailable = engine::subtitles::CaptionAccess::unavailable(
        engine::providers::Error::ModelMissing("whisper-large-v3-turbo").into(),
    );
    error(
        caption_command(
            &mut s,
            &mut r,
            &unavailable,
            "captions.generate",
            json!({"asset":1}),
            "setup",
        ),
        ErrorCode::Setup,
    );
    s.begin_gesture().unwrap();
    for (id, params) in [
        ("captions.generate", json!({"asset":1})),
        (
            "captions.translate",
            json!({"item":4,"target_language":"vi"}),
        ),
        ("captions.apply", json!({"job":0})),
    ] {
        error(
            caption_command(&mut s, &mut r, &access, id, params, id),
            ErrorCode::TransactionActive,
        );
    }
    s.cancel_gesture().unwrap();
    let rev = s.revision();
    let job = caption_submit(
        &mut s,
        &mut r,
        &access,
        "captions.generate",
        json!({"asset":1,"source_language":"en"}),
        "deny-upload",
    );
    finish(&mut r, job);
    assert_eq!(
        r.snapshot(job).unwrap().state,
        engine::jobs::JobState::Failed
    );
    assert_eq!(
        r.snapshot(job).unwrap().error.unwrap().code(),
        ErrorCode::Permission
    );
    assert_eq!(calls.load(Ordering::Relaxed), 0);
    assert!(r.output_bytes(job).is_none());
    assert_eq!(s.project(), &before);
    assert_eq!(s.revision(), rev);
    assert_eq!(s.history_len(), history);
}

#[test]
fn registry_generation_reuses_the_existing_subtitle_checkpoint_without_an_upload() {
    use crate::support::control::finish;
    use serde_json::json;
    let Some(input) = fixture() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let providers = Arc::new(providers(calls.clone(), true, false));
    let (mut r, budget) = runner();
    let (mut cache, _) =
        engine::cache::ArtifactCache::open(&dir.path().join("cache"), budget, 16).unwrap();
    let req = request(&input, "en", "en", dir.path(), false);
    subtitles::run(
        &req,
        &providers,
        &mut r,
        &mut cache,
        &AllowAll,
        &SilentProgress,
        &Cancellation::new(),
    )
    .unwrap();
    let access = engine::subtitles::CaptionAccess::new(
        providers,
        Arc::new(DenyUploads),
        Arc::new(std::sync::Mutex::new(cache)),
    );
    let mut s = registry_session(&input);
    let job = caption_submit(
        &mut s,
        &mut r,
        &access,
        "captions.generate",
        json!({"asset":1,"source_language":"en"}),
        "reuse",
    );
    finish(&mut r, job);
    let result = subtitles::decode_captions(r.output_bytes(job).unwrap()).unwrap();
    assert_eq!(result.checkpoints, 1);
    assert_eq!(calls.load(Ordering::Relaxed), 1);
}

struct WindowGate {
    config: ProviderConfig,
    calls: Arc<AtomicUsize>,
    behavior: Arc<AtomicUsize>,
    cache: Arc<std::sync::Mutex<engine::cache::ArtifactCache>>,
    entered: std::sync::mpsc::SyncSender<()>,
}
impl Transcriber for WindowGate {
    fn config(&self) -> &ProviderConfig {
        &self.config
    }
    fn capabilities(&self, scope: &Scope) -> Capabilities {
        let mut caps = FakeAsr::new(Arc::new(AtomicUsize::new(0))).capabilities(scope);
        caps.model = self.config.model().clone();
        caps
    }
    fn estimate(&self, _: &Scope) -> CostEstimate {
        CostEstimate::Free
    }
    fn transcribe(
        &self,
        audio: ai::AudioWindow<'_>,
        request: &Request<'_>,
        hooks: &dyn ExecutionHooks,
    ) -> Result<Vec<RecognitionSegment>, Error> {
        assert!(
            self.cache.try_lock().is_ok(),
            "cache mutex spans provider IO"
        );
        let n = self.calls.fetch_add(1, Ordering::AcqRel);
        if n == 1 {
            match self.behavior.load(Ordering::Acquire) {
                1 => return Err(Error::Inference),
                2 => {
                    self.entered.send(()).unwrap();
                    while !hooks.cancelled() {
                        thread::park_timeout(Duration::from_millis(1));
                    }
                    return Err(Error::Cancelled);
                }
                _ => {}
            }
        }
        FakeAsr::new(Arc::new(AtomicUsize::new(0))).transcribe(audio, request, hooks)
    }
}
#[test]
fn registry_failed_and_cancelled_windows_preserve_checkpoints_without_partial_results_or_cache_lock_during_io()
 {
    use crate::support::control::finish;
    use engine::jobs::JobState;
    use serde_json::json;
    let Some(input) = testkit::oracle::generate(
        &testkit::fixture::dir("engine"),
        "caption windows",
        "caption-31s.wav",
        &[
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=16000:duration=31",
            "-c:a",
            "pcm_s16le",
        ],
    ) else {
        return;
    };
    for behavior_value in [1, 2] {
        let dir = tempfile::tempdir().unwrap();
        let (mut r, budget) = runner();
        let (cache, _) =
            engine::cache::ArtifactCache::open(&dir.path().join("cache"), budget, 32).unwrap();
        let cache = Arc::new(std::sync::Mutex::new(cache));
        let calls = Arc::new(AtomicUsize::new(0));
        let behavior = Arc::new(AtomicUsize::new(behavior_value));
        let (send, receive) = std::sync::mpsc::sync_channel(1);
        let asr = Arc::new(WindowGate {
            config: remote_config("fake-asr", "fake-asr-model"),
            calls: calls.clone(),
            behavior: behavior.clone(),
            cache: cache.clone(),
            entered: send,
        });
        let providers = Providers::from_parts(
            TranscriptionProvider::OpenAi {
                model: "fake-asr-model".into(),
            },
            asr,
            Some(Arc::new(ProbeAligner::new(Arc::new(AtomicUsize::new(0))))),
            None,
            None,
        )
        .unwrap();
        let access =
            engine::subtitles::CaptionAccess::new(Arc::new(providers), Arc::new(AllowAll), cache);
        let mut s = registry_session(&input);
        let before = s.project().clone();
        let rev = s.revision();
        let history = s.history_len();
        let job = caption_submit(
            &mut s,
            &mut r,
            &access,
            "captions.generate",
            json!({"asset":1,"source_language":"en"}),
            "first",
        );
        r.tick().unwrap();
        if behavior_value == 2 {
            receive.recv_timeout(Duration::from_secs(10)).unwrap();
            r.cancel_job(job).unwrap();
        }
        finish(&mut r, job);
        assert_eq!(
            r.snapshot(job).unwrap().state,
            if behavior_value == 1 {
                JobState::Failed
            } else {
                JobState::Cancelled
            }
        );
        assert!(r.output_bytes(job).is_none());
        assert_eq!(s.project(), &before);
        assert_eq!(s.revision(), rev);
        assert_eq!(s.history_len(), history);
        behavior.store(0, Ordering::Release);
        let retry = caption_submit(
            &mut s,
            &mut r,
            &access,
            "captions.generate",
            json!({"asset":1,"source_language":"en"}),
            "retry",
        );
        finish(&mut r, retry);
        assert_eq!(
            r.snapshot(retry).unwrap().state,
            JobState::Completed,
            "{:?}",
            r.snapshot(retry).unwrap().error
        );
        let result = subtitles::decode_captions(r.output_bytes(retry).unwrap()).unwrap();
        assert_eq!(result.checkpoints, 1);
        assert_eq!(calls.load(Ordering::Relaxed), 3);
    }
}

fn many_cues(count: i64) -> engine::Session {
    let s = crate::support::control::seed();
    let mut data = s.project().data().clone();
    let item = data
        .scene_mut()
        .timeline
        .iter_mut()
        .flat_map(|t| &mut t.items)
        .find(|i| i.id == project::ItemId(4))
        .unwrap();
    let project::ItemParams::Caption(params) = &mut item.content else {
        panic!("caption");
    };
    let prototype = params.cues.cues[0].clone();
    params.cues.cues = (0..count)
        .map(|n| {
            let mut cue = prototype.clone();
            cue.id = format!("cue-{n}").into();
            cue.range =
                time::SourceRange::new(time::Tick(n * 1000), time::Tick((n + 1) * 1000)).unwrap();
            cue.word_ranges.clear();
            if n == 0 {
                cue.texts.insert(
                    lang("vi"),
                    captions::CueText {
                        text: "manual".into(),
                        edited: true,
                        translation_ref: None,
                    },
                );
            }
            cue
        })
        .collect();
    engine::Session::new(project::Project::new(data).unwrap(), 10).unwrap()
}
#[test]
fn registry_translation_snapshots_all_windows_preserves_edited_targets_and_applies_only_through_retranslate()
 {
    use crate::support::control::{error, finish, ok};
    use engine::control::ErrorCode;
    use serde_json::json;
    let dir = tempfile::tempdir().unwrap();
    let access = caption_access(
        &dir.path().join("cache"),
        providers(Arc::new(AtomicUsize::new(0)), false, true),
        Arc::new(AllowAll),
    );
    let mut s = many_cues(130);
    let before = s.project().clone();
    let rev = s.revision();
    let history = s.history_len();
    let (mut r, _) = runner();
    for (n, cues) in [
        json!([]),
        json!(["cue-1", "cue-1"]),
        json!(["missing"]),
        json!(["cue-0"]),
    ]
    .into_iter()
    .enumerate()
    {
        let response = caption_command(
            &mut s,
            &mut r,
            &access,
            "captions.translate",
            json!({"item":4,"cues":cues,"target_language":"vi"}),
            &format!("invalid-{n}"),
        );
        error(
            response,
            if n == 0 {
                ErrorCode::Limit
            } else {
                ErrorCode::Refused
            },
        );
    }
    let job = caption_submit(
        &mut s,
        &mut r,
        &access,
        "captions.translate",
        json!({"item":4,"target_language":"vi"}),
        "translate",
    );
    finish(&mut r, job);
    assert_eq!(
        r.snapshot(job).unwrap().state,
        engine::jobs::JobState::Completed,
        "{:?}",
        r.snapshot(job).unwrap().error
    );
    assert_eq!(s.project(), &before);
    assert_eq!(s.revision(), rev);
    assert_eq!(s.history_len(), history);
    let result = subtitles::decode_translations(r.output_bytes(job).unwrap()).unwrap();
    assert_eq!(result.texts.len(), 129);
    assert!(!result.texts.contains_key(&captions::NfcText::from("cue-0")));
    assert_eq!(
        result.translation_ref.as_str(),
        "fake-translator/fake-translator-model"
    );
    ok(caption_command(
        &mut s,
        &mut r,
        &access,
        "retranslate",
        json!({"item":4,"target_language":"vi","texts":result.texts,"translation_ref":result.translation_ref,"force":false}),
        "apply-translation",
    ));
    assert_eq!(s.history_len().0, history.0 + 1);
    let track = s
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|t| &t.items)
        .find(|i| i.id == project::ItemId(4))
        .unwrap();
    let project::ItemParams::Caption(params) = &track.content else {
        panic!("caption");
    };
    assert_eq!(
        params.cues.cues[0].texts[&lang("vi")].text.as_str(),
        "manual"
    );
    assert!(
        params.cues.cues[129].texts[&lang("vi")]
            .text
            .as_str()
            .ends_with("[vi]")
    );
    s.undo().unwrap();
    assert_eq!(s.project(), &before);
    let job = caption_submit(
        &mut s,
        &mut r,
        &access,
        "captions.translate",
        json!({"item":4,"cues":["cue-0"],"target_language":"vi","force":true}),
        "force",
    );
    finish(&mut r, job);
    let result = subtitles::decode_translations(r.output_bytes(job).unwrap()).unwrap();
    assert_eq!(result.texts.len(), 1);
    let deny = engine::subtitles::CaptionAccess::new(
        Arc::new(providers(Arc::new(AtomicUsize::new(0)), false, true)),
        Arc::new(DenyUploads),
        access.cache().unwrap().clone(),
    );
    let job = caption_submit(
        &mut s,
        &mut r,
        &deny,
        "captions.translate",
        json!({"item":4,"cues":["cue-1"],"target_language":"vi"}),
        "deny-translation",
    );
    finish(&mut r, job);
    assert_eq!(
        r.snapshot(job).unwrap().error.unwrap().code(),
        ErrorCode::Permission
    );
    assert!(r.output_bytes(job).is_none());
    assert_eq!(s.project(), &before);
}

#[test]
fn complete_retranslation_accepts_ten_thousand_cues_and_refuses_one_more_atomically() {
    for count in [10_000, 10_001] {
        let mut session = many_cues(10_000);
        let before = session.project().clone();
        let revision = session.revision();
        let texts = (0..count)
            .map(|n| (format!("cue-{n}").into(), "Translated".into()))
            .collect();
        let result = session.apply(engine::Command::Retranslate {
            item: project::ItemId(4),
            target_language: lang("vi"),
            texts,
            translation_ref: "fixture".into(),
            force: true,
        });
        if count == 10_000 {
            assert!(result.unwrap().changed);
            assert_eq!(session.history_len(), (1, 0));
        } else {
            assert!(matches!(
                result,
                Err(engine::EditError::Invalid("translation application"))
            ));
            assert_eq!(session.project(), &before);
            assert_eq!(session.revision(), revision);
            assert_eq!(session.history_len(), (0, 0));
        }
    }
}

struct LargeTranslator(FakeTranslator, usize);
impl Translator for LargeTranslator {
    fn config(&self) -> &ProviderConfig {
        self.0.config()
    }
    fn capabilities(&self, scope: &Scope) -> Capabilities {
        self.0.capabilities(scope)
    }
    fn estimate(&self, scope: &Scope) -> CostEstimate {
        self.0.estimate(scope)
    }
    fn translate(
        &self,
        window: &TranslationWindow,
        _: &Request<'_>,
        _: &dyn ExecutionHooks,
    ) -> Result<Translation, Error> {
        Ok(Translation {
            source_language: window.source_language.clone(),
            target_language: window.target_language.clone(),
            texts: window
                .segments
                .iter()
                .map(|s| (s.id.clone(), "x".repeat(self.1).into()))
                .collect(),
            number_warnings: Vec::new(),
        })
    }
}
#[test]
fn registry_oversized_translation_and_cancelled_provider_publish_no_result_or_live_edit() {
    use crate::support::control::finish;
    use serde_json::json;
    let dir = tempfile::tempdir().unwrap();
    let (mut r, _) = runner();
    let mut s = many_cues(10_000);
    let before = s.project().clone();
    let rev = s.revision();
    let p = Providers::from_parts(
        TranscriptionProvider::Whisper {
            model: "fake-asr-model".into(),
        },
        Arc::new(FakeAsr::new(Arc::new(AtomicUsize::new(0)))),
        None,
        Some(TranslationProvider::OpenAi {
            model: "fake-translator-model".into(),
        }),
        Some(Arc::new(LargeTranslator(FakeTranslator::new(), 1024))),
    )
    .unwrap();
    let access = caption_access(&dir.path().join("large"), p, Arc::new(AllowAll));
    let job = caption_submit(
        &mut s,
        &mut r,
        &access,
        "captions.translate",
        json!({"item":4,"target_language":"vi"}),
        "large",
    );
    finish(&mut r, job);
    let snap = r.snapshot(job).unwrap();
    assert_eq!(snap.state, engine::jobs::JobState::Failed);
    assert_eq!(
        snap.error.unwrap().code(),
        engine::control::ErrorCode::Limit
    );
    assert!(r.output_bytes(job).is_none());
    assert_eq!(s.project(), &before);
    assert_eq!(s.revision(), rev);
    let entered = Arc::new(AtomicBool::new(false));
    let p = Providers::from_parts(
        TranscriptionProvider::Whisper {
            model: "fake-asr-model".into(),
        },
        Arc::new(FakeAsr::new(Arc::new(AtomicUsize::new(0)))),
        None,
        Some(TranslationProvider::OpenAi {
            model: "fake-translator-model".into(),
        }),
        Some(Arc::new(BlockingTranslator {
            config: remote_config("fake-translator", "fake-translator-model"),
            entered: entered.clone(),
        })),
    )
    .unwrap();
    let access = caption_access(&dir.path().join("cancel"), p, Arc::new(AllowAll));
    let job = caption_submit(
        &mut s,
        &mut r,
        &access,
        "captions.translate",
        json!({"item":4,"cues":["cue-1"],"target_language":"vi"}),
        "cancel",
    );
    r.tick().unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !entered.load(Ordering::Acquire) {
        assert!(std::time::Instant::now() < deadline);
        thread::yield_now();
    }
    r.cancel_job(job).unwrap();
    finish(&mut r, job);
    assert_eq!(
        r.snapshot(job).unwrap().state,
        engine::jobs::JobState::Cancelled
    );
    assert!(r.output_bytes(job).is_none());
    assert_eq!(s.project(), &before);
    assert_eq!(s.revision(), rev);
}

#[test]
fn registry_translation_complete_result_round_trips_through_control_or_refuses_before_publication()
{
    use crate::support::control::{Allow, caller, finish, ok, request};
    use serde_json::json;
    // The middle row has less than 4 MiB of values, but keys/envelope push the request over.
    for (count, length, accepted) in [(4096, 1016, true), (4096, 1017, false), (5000, 1024, false)]
    {
        let dir = tempfile::tempdir().unwrap();
        let p = Providers::from_parts(
            TranscriptionProvider::Whisper {
                model: "fake-asr-model".into(),
            },
            Arc::new(FakeAsr::new(Arc::new(AtomicUsize::new(0)))),
            None,
            Some(TranslationProvider::OpenAi {
                model: "fake-translator-model".into(),
            }),
            Some(Arc::new(LargeTranslator(FakeTranslator::new(), length))),
        )
        .unwrap();
        let access = caption_access(&dir.path().join("cache"), p, Arc::new(AllowAll));
        let mut s = many_cues(count);
        let before = s.project().clone();
        let rev = s.revision();
        let history = s.history_len();
        let (mut r, _) = runner();
        let job = caption_submit(
            &mut s,
            &mut r,
            &access,
            "captions.translate",
            json!({"item":4,"target_language":"vi"}),
            "translate",
        );
        finish(&mut r, job);
        assert_eq!(s.project(), &before);
        assert_eq!(s.revision(), rev);
        assert_eq!(s.history_len(), history);
        let snap = r.snapshot(job).unwrap();
        if !accepted {
            assert_eq!(snap.state, engine::jobs::JobState::Failed);
            assert_eq!(
                snap.error.unwrap().code(),
                engine::control::ErrorCode::Limit
            );
            assert!(r.output_bytes(job).is_none());
            continue;
        }
        assert_eq!(
            snap.state,
            engine::jobs::JobState::Completed,
            "{:?}",
            snap.error
        );
        let result = subtitles::decode_translations(r.output_bytes(job).unwrap()).unwrap();
        assert_eq!(result.texts.len(), count as usize - 1);
        let mut req = request(
            &s,
            "retranslate",
            json!({"item":result.item,
            "target_language":result.target_language,"texts":result.texts,
            "translation_ref":result.translation_ref,"force":false}),
            &"k".repeat(128),
        );
        req.request_id = "r".repeat(128);
        let frame = engine::control::bounded_json(&req, engine::control::MAX_FRAME_BYTES).unwrap();
        ok(s.control_json(&frame, caller(&Allow), &mut r).unwrap());
        assert_eq!(s.history_len().0, history.0 + 1);
        s.undo().unwrap();
        assert_eq!(s.project(), &before);
    }
}

#[test]
fn registry_generation_refuses_changed_source_before_applying_a_completed_job() {
    use crate::support::control::{error, finish};
    use serde_json::json;
    let Some(fixture) = fixture() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("source.mp4");
    std::fs::copy(fixture, &input).unwrap();
    let mut s = registry_session(&input);
    let before = s.project().clone();
    let rev = s.revision();
    let history = s.history_len();
    let access = caption_access(
        &dir.path().join("cache"),
        providers(Arc::new(AtomicUsize::new(0)), false, false),
        Arc::new(AllowAll),
    );
    let (mut r, _) = runner();
    let job = caption_submit(
        &mut s,
        &mut r,
        &access,
        "captions.generate",
        json!({"asset":1,"source_language":"en"}),
        "generate",
    );
    finish(&mut r, job);
    let mut bytes = std::fs::read(&input).unwrap();
    bytes.extend_from_slice(b"changed content");
    std::fs::write(&input, bytes).unwrap();
    error(
        caption_command(
            &mut s,
            &mut r,
            &access,
            "captions.apply",
            json!({"job":job}),
            "changed",
        ),
        engine::control::ErrorCode::StaleResult,
    );
    assert_eq!(s.project(), &before);
    assert_eq!(s.revision(), rev);
    assert_eq!(s.history_len(), history);
}

struct LanguageGate {
    inner: FakeAsr,
    entered: std::sync::mpsc::SyncSender<()>,
    release: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
}
#[derive(Default)]
struct CaptionStageClock(std::sync::atomic::AtomicU64);
impl engine::jobs::Clock for CaptionStageClock {
    fn now(&self) -> Duration {
        Duration::from_millis(self.0.fetch_add(1000, Ordering::AcqRel))
    }
}
impl LanguageGate {
    fn wait(&self, hooks: &dyn ExecutionHooks) -> Result<(), Error> {
        self.entered.send(()).map_err(|_| Error::Inference)?;
        self.release
            .lock()
            .map_err(|_| Error::Inference)?
            .recv_timeout(Duration::from_secs(10))
            .map_err(|_| Error::Inference)?;
        if hooks.cancelled() {
            Err(Error::Cancelled)
        } else {
            Ok(())
        }
    }
}
impl Transcriber for LanguageGate {
    fn config(&self) -> &ProviderConfig {
        self.inner.config()
    }
    fn capabilities(&self, scope: &Scope) -> Capabilities {
        self.inner.capabilities(scope)
    }
    fn estimate(&self, scope: &Scope) -> CostEstimate {
        self.inner.estimate(scope)
    }
    fn detect_language(
        &self,
        _: ai::AudioWindow<'_>,
        hooks: &dyn ExecutionHooks,
    ) -> Result<LanguageTag, Error> {
        self.wait(hooks)?;
        Ok(lang("en"))
    }
    fn transcribe(
        &self,
        audio: ai::AudioWindow<'_>,
        request: &Request<'_>,
        hooks: &dyn ExecutionHooks,
    ) -> Result<Vec<RecognitionSegment>, Error> {
        self.wait(hooks)?;
        self.inner.transcribe(audio, request, hooks)
    }
}

#[test]
fn omitted_language_detection_reports_job_stages_and_uses_the_job_cancellation_token() {
    use crate::support::control::finish;
    use engine::jobs::{JobStage, JobState};
    use serde_json::json;
    let Some(input) = fixture() else {
        return;
    };
    for cancel_detection in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let (entered, receive) = std::sync::mpsc::sync_channel(1);
        let (release, gate) = std::sync::mpsc::sync_channel(1);
        let providers = Providers::from_parts(
            TranscriptionProvider::Whisper {
                model: "fake-asr-model".into(),
            },
            Arc::new(LanguageGate {
                inner: FakeAsr::new(calls.clone()),
                entered,
                release: std::sync::Mutex::new(gate),
            }),
            None,
            None,
            None,
        )
        .unwrap();
        let access = caption_access(&dir.path().join("cache"), providers, Arc::new(DenyUploads));
        let clock = Arc::new(CaptionStageClock::default());
        let mut runner = JobRunner::new(
            RunnerConfig {
                concurrency: [1, 1, 1],
                queue: 8,
                retained: 8,
                network_attempts: 1,
                backoff: Duration::from_millis(1),
                progress_interval: Duration::from_millis(10),
            },
            ByteBudget::new(512 << 20).unwrap(),
            clock.clone(),
        )
        .unwrap();
        let mut session = registry_session(&input);
        let before = session.project().clone();
        let revision = session.revision();
        let history = session.history_len();
        let job = caption_submit(
            &mut session,
            &mut runner,
            &access,
            "captions.generate",
            json!({"asset":1}),
            "detect",
        );
        runner.tick().unwrap();
        receive.recv_timeout(Duration::from_secs(10)).unwrap();
        runner.tick().unwrap();
        assert_eq!(runner.snapshot(job).unwrap().stage, JobStage::Extract);
        if cancel_detection {
            runner.cancel_job(job).unwrap();
        }
        release.send(()).unwrap();
        if !cancel_detection {
            receive.recv_timeout(Duration::from_secs(10)).unwrap();
            runner.tick().unwrap();
            assert_eq!(runner.snapshot(job).unwrap().stage, JobStage::Transcribe);
            release.send(()).unwrap();
        }
        finish(&mut runner, job);
        if cancel_detection {
            assert_eq!(runner.snapshot(job).unwrap().state, JobState::Cancelled);
            assert!(runner.output_bytes(job).is_none());
            assert_eq!(calls.load(Ordering::Acquire), 0);
        } else {
            assert_eq!(runner.snapshot(job).unwrap().state, JobState::Completed);
            let result = subtitles::decode_captions(runner.output_bytes(job).unwrap()).unwrap();
            assert_eq!(result.source_language, lang("en"));
            assert!(!result.cues.is_empty());
            assert_eq!(calls.load(Ordering::Acquire), 1);
        }
        assert_eq!(session.project(), &before);
        assert_eq!(session.revision(), revision);
        assert_eq!(session.history_len(), history);
    }
}

struct RetryTranslator {
    inner: FakeTranslator,
    calls: Arc<AtomicUsize>,
}
impl Translator for RetryTranslator {
    fn config(&self) -> &ProviderConfig {
        self.inner.config()
    }
    fn capabilities(&self, s: &Scope) -> Capabilities {
        self.inner.capabilities(s)
    }
    fn estimate(&self, s: &Scope) -> CostEstimate {
        self.inner.estimate(s)
    }
    fn translate(
        &self,
        _: &TranslationWindow,
        _: &Request<'_>,
        _: &dyn ExecutionHooks,
    ) -> Result<Translation, Error> {
        self.calls.fetch_add(1, Ordering::AcqRel);
        Err(Error::Schema)
    }
}
#[derive(Default)]
struct CaptionClock(std::sync::atomic::AtomicU64);
impl engine::jobs::Clock for CaptionClock {
    fn now(&self) -> Duration {
        Duration::from_millis(self.0.load(Ordering::Acquire))
    }
}
#[test]
fn registry_translation_retries_share_the_operation_attempt_ceiling() {
    use serde_json::json;
    let dir = tempfile::tempdir().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let p = Providers::from_parts(
        TranscriptionProvider::Whisper {
            model: "fake-asr-model".into(),
        },
        Arc::new(FakeAsr::new(Arc::new(AtomicUsize::new(0)))),
        None,
        Some(TranslationProvider::OpenAi {
            model: "fake-translator-model".into(),
        }),
        Some(Arc::new(RetryTranslator {
            inner: FakeTranslator::new(),
            calls: calls.clone(),
        })),
    )
    .unwrap();
    let access = caption_access(&dir.path().join("cache"), p, Arc::new(AllowAll));
    let clock = Arc::new(CaptionClock::default());
    let mut r = JobRunner::new(
        RunnerConfig {
            concurrency: [1, 1, 1],
            queue: 8,
            retained: 8,
            network_attempts: 10,
            backoff: Duration::from_millis(1),
            progress_interval: Duration::from_millis(10),
        },
        ByteBudget::new(512 << 20).unwrap(),
        clock.clone(),
    )
    .unwrap();
    let mut s = many_cues(2);
    let before = s.project().clone();
    let rev = s.revision();
    let job = caption_submit(
        &mut s,
        &mut r,
        &access,
        "captions.translate",
        json!({"item":4,"cues":["cue-1"],"target_language":"vi"}),
        "retry",
    );
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        clock.0.fetch_add(1000, Ordering::AcqRel);
        r.tick().unwrap();
        if r.snapshot(job).unwrap().state == engine::jobs::JobState::Failed {
            break;
        }
        assert!(std::time::Instant::now() < deadline);
        thread::yield_now();
    }
    assert_eq!(calls.load(Ordering::Acquire), 3);
    assert_eq!(r.snapshot(job).unwrap().attempts, 4);
    assert!(r.output_bytes(job).is_none());
    assert_eq!(s.project(), &before);
    assert_eq!(s.revision(), rev);
}

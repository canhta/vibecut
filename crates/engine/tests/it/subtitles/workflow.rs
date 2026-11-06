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
        Arc, Mutex, OnceLock,
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

/// Wraps `FakeAsr` and records the sample window each ASR call receives.
struct RecordingAsr {
    inner: FakeAsr,
    windows: Arc<Mutex<Vec<(u64, u64)>>>,
}
impl Transcriber for RecordingAsr {
    fn config(&self) -> &ProviderConfig {
        self.inner.config()
    }
    fn capabilities(&self, scope: &Scope) -> Capabilities {
        self.inner.capabilities(scope)
    }
    fn estimate(&self, scope: &Scope) -> CostEstimate {
        self.inner.estimate(scope)
    }
    fn transcribe(
        &self,
        audio: ai::AudioWindow<'_>,
        request: &Request<'_>,
        hooks: &dyn ExecutionHooks,
    ) -> Result<Vec<RecognitionSegment>, Error> {
        self.windows
            .lock()
            .unwrap()
            .push((audio.range.start(), audio.range.end()));
        self.inner.transcribe(audio, request, hooks)
    }
}

/// The ASR checkpoint key as the code before the transcript move derived it. Kept here so a
/// change to the real derivation shows up as a cache miss for checkpoints already on disk.
fn pre_move_asr_key(
    content_hash: &[u8; 32],
    window: (u64, u64),
    language: &str,
    config: &ProviderConfig,
) -> engine::cache::CacheKey {
    let hex = |bytes: &[u8]| -> String { bytes.iter().map(|b| format!("{b:02x}")).collect() };
    // The fingerprint is the hex of the content hash, and the key hashes the hex of that text.
    let fingerprint_hex = hex(hex(content_hash).as_bytes());
    let (start, end) = window;
    engine::cache::CacheKey::new(engine::cache::KeyInputs {
        kind: engine::cache::ArtifactKind::AsrChunk,
        source: *blake3::hash(format!("{fingerprint_hex}:{start}:{end}").as_bytes()).as_bytes(),
        provider: config.id().as_str(),
        model: config.model().as_str(),
        version: config.settings_fingerprint().as_str(),
        settings: format!("language={language};samples={start}..{end}").as_bytes(),
        transcript: [0; 32],
        cues: [0; 32],
        clip_range: [0; 32],
        translation: [0; 32],
    })
    .unwrap()
}

#[test]
fn asr_checkpoint_written_under_the_pre_move_key_is_reused() {
    let Some(input) = fixture() else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let windows = Arc::new(Mutex::new(Vec::new()));
    let calls = Arc::new(AtomicUsize::new(0));
    let asr = RecordingAsr {
        inner: FakeAsr::new(calls.clone()),
        windows: windows.clone(),
    };
    let config = asr.config().clone();
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
    let content_hash = media::SourceAsset::import(media::AssetId(1), input.clone())
        .unwrap()
        .content_hash()
        .0;
    let (mut runner, budget) = runner();
    let mut request = request(&input, "vi", "vi", directory.path(), false);
    request.srt = Some(directory.path().join("first.srt"));

    // The first run stores its window; the entry must sit under the pre-move key.
    let (mut first_cache, _) =
        engine::cache::ArtifactCache::open(&directory.path().join("first"), budget.clone(), 16)
            .unwrap();
    subtitles::run(
        &request,
        &providers,
        &mut runner,
        &mut first_cache,
        &AllowAll,
        &SilentProgress,
        &Cancellation::new(),
    )
    .unwrap();
    let window = {
        let seen = windows.lock().unwrap();
        assert_eq!(seen.len(), 1, "one speech window in the 2 s fixture");
        seen[0]
    };
    let key = pre_move_asr_key(&content_hash, window, "vi", &config);
    let stored = first_cache
        .get(key)
        .unwrap()
        .expect("checkpoint stored under the pre-move key")
        .bytes()
        .to_vec();

    // A cache holding only that entry satisfies a fresh run without calling the provider.
    let (mut seeded, _) =
        engine::cache::ArtifactCache::open(&directory.path().join("seeded"), budget, 16).unwrap();
    seeded.put(key, &stored).unwrap();
    request.srt = Some(directory.path().join("second.srt"));
    let outcome = subtitles::run(
        &request,
        &providers,
        &mut runner,
        &mut seeded,
        &AllowAll,
        &SilentProgress,
        &Cancellation::new(),
    )
    .unwrap();
    assert_eq!(outcome.checkpoints, 1);
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
        &engine::transcript::Cancellation::new(),
    )
    .unwrap_err();
    assert!(matches!(error, engine::EditError::ConsentRequired));
    assert_eq!(calls.load(Ordering::Relaxed), 0);
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
    Providers::from_parts(
        transcription,
        Arc::new(TextOnlyAsr(FakeAsr::new(asr_calls))),
        Some(aligner),
        None,
        None,
    )
    .unwrap()
}

#[test]
fn review_text_only_asr_reaches_local_aligner() {
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
    assert!(matches!(error, engine::EditError::JobFailure(_)));
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
            config: config("fake-translator", "fake-translator-model"),
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

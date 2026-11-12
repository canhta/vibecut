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
    subtitles::{self, LanguageTag},
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
        "transcribe".into(),
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

/// One subtitle run as the CLI composes it: generate, apply, translate, retranslate, export.
struct Run {
    input: PathBuf,
    source: LanguageTag,
    target: LanguageTag,
    bilingual: bool,
    srt: Option<PathBuf>,
    burn: Option<PathBuf>,
    overwrite: bool,
}

struct Outcome {
    cues: usize,
    checkpoints: usize,
    cost: engine::control::CostView,
    translated: bool,
}

type Cause = Arc<dyn engine::Failure + 'static>;

fn run(input: &Path, source: &str, target: &str, directory: &Path, burn: bool) -> Run {
    Run {
        input: input.to_path_buf(),
        source: lang(source),
        target: lang(target),
        bilingual: false,
        srt: Some(directory.join(format!("{source}-{target}.srt"))),
        burn: burn.then(|| directory.join(format!("{source}-{target}.mp4"))),
        overwrite: false,
    }
}

fn cause(error: &Cause) -> &engine::subtitles::Error {
    (error.as_ref() as &(dyn std::error::Error + 'static))
        .downcast_ref::<engine::subtitles::Error>()
        .expect("subtitle workflow cause")
}

fn consent_refused(error: &Cause) -> bool {
    matches!(
        cause(error),
        engine::subtitles::Error::Providers(engine::providers::Error::ConsentRequired)
            | engine::subtitles::Error::Transcript(engine::transcript::Error::Providers(
                engine::providers::Error::ConsentRequired
            ))
    )
}

/// Wait for a job; a failed job returns the cause it retained.
fn settle(runner: &mut JobRunner, job: engine::jobs::JobId) -> Result<(), Cause> {
    crate::support::control::finish(runner, job);
    let snapshot = runner.snapshot(job).unwrap();
    match snapshot.state {
        engine::jobs::JobState::Completed => Ok(()),
        engine::jobs::JobState::Failed => Err(snapshot.error.clone().expect("failure cause")),
        state => panic!("unexpected job state {state:?}"),
    }
}

fn workflow(
    run: &Run,
    access: &subtitles::CaptionAccess,
    runner: &mut JobRunner,
) -> Result<Outcome, Cause> {
    use serde_json::json;
    let asset = media::SourceAsset::import(media::AssetId(1), run.input.clone()).unwrap();
    let duration = asset.probe().duration.unwrap();
    let mut session =
        engine::transcript::source_session(&asset, duration, run.burn.is_some(), None).unwrap();
    let generated = caption_submit(
        &mut session,
        runner,
        access,
        "captions.generate",
        json!({"asset": 1, "source_language": run.source}),
        "generate",
    );
    settle(runner, generated)?;
    let captions = subtitles::decode_captions(runner.output_bytes(generated).unwrap()).unwrap();
    crate::support::control::ok(caption_command(
        &mut session,
        runner,
        access,
        "captions.apply",
        json!({"job": generated}),
        "apply",
    ));
    let (track, item) = session
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .find_map(|track| {
            track
                .items
                .iter()
                .find(|item| matches!(item.content, project::ItemParams::Caption(_)))
                .map(|item| (track.id, item.id))
        })
        .unwrap();
    let translated = run.source != run.target;
    if translated {
        let job = caption_submit(
            &mut session,
            runner,
            access,
            "captions.translate",
            json!({"item": item, "target_language": run.target}),
            "translate",
        );
        settle(runner, job)?;
        let result = subtitles::decode_translations(runner.output_bytes(job).unwrap()).unwrap();
        crate::support::control::ok(caption_command(
            &mut session,
            runner,
            access,
            "retranslate",
            json!({
                "item": result.item,
                "target_language": result.target_language,
                "texts": result.texts,
                "translation_ref": result.translation_ref,
                "force": false,
            }),
            "retranslate",
        ));
    }
    let languages = if translated && run.bilingual {
        vec![run.source.clone(), run.target.clone()]
    } else {
        vec![run.target.clone()]
    };
    if let Some(path) = &run.srt {
        let job = caption_submit(
            &mut session,
            runner,
            access,
            "export_subtitles",
            json!({"track": track, "format": "srt", "languages": languages,
                "path": path, "overwrite": run.overwrite}),
            "export",
        );
        settle(runner, job)?;
    }
    if let Some(path) = &run.burn {
        let job = caption_submit(
            &mut session,
            runner,
            access,
            "burn_subtitles",
            json!({
                "range": time::TimeRange::new(time::Tick::ZERO, duration).unwrap(),
                "languages": languages,
                "style": render::CaptionStyle::default(),
                "portrait": false,
                "path": path,
                "overwrite": run.overwrite,
            }),
            "burn",
        );
        settle(runner, job)?;
    }
    Ok(Outcome {
        cues: captions.cues.len(),
        checkpoints: captions.checkpoints,
        cost: captions.cost,
        translated,
    })
}

#[test]
fn six_directions_and_every_bilingual_pair() {
    let Some(input) = fixture() else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let access = caption_access(
        &directory.path().join("cache"),
        providers(calls.clone(), false, true),
        Arc::new(AllowAll),
    );
    let (mut runner, _) = runner();
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
            let mut run = run(&input, source, target, directory.path(), false);
            run.bilingual = bilingual;
            run.srt = Some(
                directory
                    .path()
                    .join(format!("{source}-{target}-{bilingual}.srt")),
            );
            let outcome = workflow(&run, &access, &mut runner).unwrap();
            assert!(outcome.cues > 0);
            let text = std::fs::read_to_string(run.srt.as_ref().unwrap()).unwrap();
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
    let access = caption_access(
        &directory.path().join("cache"),
        providers(Arc::new(AtomicUsize::new(0)), false, true),
        Arc::new(AllowAll),
    );
    let (mut runner, _) = runner();
    let mut run = run(&input, "vi", "en", directory.path(), false);
    run.bilingual = true;
    let outcome = workflow(&run, &access, &mut runner).unwrap();
    assert!(outcome.translated);
    let text = std::fs::read_to_string(run.srt.as_ref().unwrap()).unwrap();
    assert!(text.contains("xin chào"));
    assert!(text.contains("[en]"));
}

#[test]
fn srt_and_burn_are_produced_from_one_run() {
    let Some(input) = fixture() else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let access = caption_access(
        &directory.path().join("cache"),
        providers(Arc::new(AtomicUsize::new(0)), false, true),
        Arc::new(AllowAll),
    );
    let (mut runner, _) = runner();
    let run = run(&input, "vi", "en", directory.path(), true);
    workflow(&run, &access, &mut runner).unwrap();
    let srt = std::fs::read_to_string(run.srt.as_ref().unwrap()).unwrap();
    assert!(srt.contains("xin chào"));
    assert!(std::fs::metadata(run.burn.as_ref().unwrap()).unwrap().len() > 0);
}

#[test]
fn second_run_reuses_completed_asr_chunks() {
    let Some(input) = fixture() else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let access = caption_access(
        &directory.path().join("cache"),
        providers(calls.clone(), false, true),
        Arc::new(AllowAll),
    );
    let (mut runner, _) = runner();
    let mut run = run(&input, "vi", "en", directory.path(), false);
    run.srt = Some(directory.path().join("first.srt"));
    let first = workflow(&run, &access, &mut runner).unwrap();
    assert_eq!(first.checkpoints, 0);
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    run.srt = Some(directory.path().join("second.srt"));
    let second = workflow(&run, &access, &mut runner).unwrap();
    assert_eq!(second.checkpoints, 1);
    assert_eq!(calls.load(Ordering::Relaxed), 1, "ASR was not re-executed");
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
    let access = caption_access(
        &directory.path().join("cache"),
        providers(calls.clone(), true, true),
        Arc::new(DenyUploads),
    );
    let (mut runner, _) = runner();
    let run = run(&input, "vi", "en", directory.path(), false);
    let error = workflow(&run, &access, &mut runner).err().unwrap();
    assert!(consent_refused(&error));
    assert_eq!(calls.load(Ordering::Relaxed), 0);
    assert!(!run.srt.unwrap().exists());
}

#[test]
fn checkpoint_envelope_miss_renews_consent_but_bad_json_never_reruns_asr() {
    let Some(input) = fixture() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let providers = Arc::new(providers(calls.clone(), true, false));
    let (mut runner, budget) = runner();
    let root = dir.path().join("cache");
    let open = |budget: &ByteBudget| {
        let (cache, diagnostics) =
            engine::cache::ArtifactCache::open(&root, budget.clone(), 16).unwrap();
        (Arc::new(std::sync::Mutex::new(cache)), diagnostics)
    };
    let access =
        |cache: &Arc<std::sync::Mutex<engine::cache::ArtifactCache>>,
         consent: Arc<dyn engine::providers::ConsentPrompt + Send + Sync>| {
            subtitles::CaptionAccess::new(providers.clone(), consent, cache.clone())
        };
    let (cache, _) = open(&budget);
    let mut req = run(&input, "en", "en", dir.path(), false);
    let first = workflow(&req, &access(&cache, Arc::new(AllowAll)), &mut runner).unwrap();
    assert_eq!(first.cost, engine::control::CostView::Free);
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
    let refused = workflow(&req, &access(&cache, Arc::new(DenyUploads)), &mut runner)
        .err()
        .unwrap();
    assert!(consent_refused(&refused));
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    assert!(!req.srt.as_ref().unwrap().exists());
    let regenerated = workflow(&req, &access(&cache, Arc::new(AllowAll)), &mut runner).unwrap();
    assert_eq!(regenerated.checkpoints, 0);
    assert_eq!(regenerated.cost, engine::control::CostView::Free);
    assert_eq!(calls.load(Ordering::Relaxed), 2);
    drop(cache);
    let second_output = req.srt.clone().unwrap();
    let second_preserved = std::fs::read(&second_output).unwrap();
    let mut envelope = std::fs::read(&path).unwrap();
    envelope.truncate(72);
    envelope[40..72].copy_from_slice(blake3::hash(b"not JSON").as_bytes());
    envelope.extend_from_slice(b"not JSON");
    std::fs::write(&path, &envelope).unwrap();
    let (cache, diagnostics) = open(&budget);
    assert_eq!(diagnostics, 0);
    req.srt = Some(dir.path().join("must-not-publish.srt"));
    let malformed = workflow(&req, &access(&cache, Arc::new(AllowAll)), &mut runner)
        .err()
        .unwrap();
    assert!(matches!(
        cause(&malformed),
        engine::subtitles::Error::Transcript(engine::transcript::Error::Edit(
            engine::EditError::Invalid("cached recognition is malformed")
        ))
    ));
    assert_eq!(calls.load(Ordering::Relaxed), 2);
    assert_eq!(std::fs::read(&path).unwrap(), envelope);
    assert_eq!(std::fs::read(old_output).unwrap(), preserved);
    assert_eq!(std::fs::read(second_output).unwrap(), second_preserved);
    assert!(!req.srt.as_ref().unwrap().exists());
    assert_eq!(cache.lock().unwrap().clear().unwrap(), 0);
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
    assert!(
        matches!(engine::providers::choose_transcription("openai",Some("fixture-custom")),Ok(TranscriptionProvider::OpenAi {model}) if model=="fixture-custom")
    );
    assert!(engine::providers::choose_transcription("openai", Some("")).is_err());
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
        engine::providers::Error::CredentialMissing("OPENAI_API_KEY")
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
        "transcribe".into(),
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
    let (mut runner, _) = runner();
    let mut run = run(&input, "en", "en", directory.path(), false);
    run.overwrite = true;
    for transcription in [
        TranscriptionProvider::OpenAi {
            model: "fake-asr-model".into(),
        },
        TranscriptionProvider::Gipformer {
            model: "fake-asr-model".into(),
        },
    ] {
        let access = caption_access(
            &directory
                .path()
                .join(format!("cache-{}", transcription.id())),
            text_only_providers(
                transcription,
                asr_calls.clone(),
                Arc::new(ProbeAligner::new(align_calls.clone())),
            ),
            Arc::new(AllowAll),
        );
        workflow(&run, &access, &mut runner).unwrap();
        assert!(run.srt.as_ref().unwrap().exists());
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
    let (mut runner, _) = runner();
    let access = caption_access(
        &directory.path().join("cache"),
        providers,
        Arc::new(AllowAll),
    );
    let run = run(&input, "en", "en", directory.path(), false);
    let error = workflow(&run, &access, &mut runner).err().unwrap();
    assert!(matches!(
        cause(&error),
        engine::subtitles::Error::Transcript(engine::transcript::Error::Ai(ai::Error::Inference))
    ));
    assert_eq!(asr_calls.load(Ordering::Relaxed), 1);
    let outcome = workflow(&run, &access, &mut runner).unwrap();
    assert!(run.srt.as_ref().unwrap().exists());
    assert_eq!(outcome.checkpoints, 1);
    assert_eq!(asr_calls.load(Ordering::Relaxed), 1);
    assert_eq!(align_calls.load(Ordering::Relaxed), 2);
    // The acquisition-only task keeps its Transcript producer; the mixed pipeline above keeps Engine.
    let asr_calls = Arc::new(AtomicUsize::new(0));
    let align_calls = Arc::new(AtomicUsize::new(0));
    let providers = Arc::new(text_only_providers(
        TranscriptionProvider::OpenAi {
            model: "fake-asr-model".into(),
        },
        asr_calls.clone(),
        Arc::new(ProbeAligner::failing_once(align_calls.clone())),
    ));
    let (mut jobs, budget) = self::runner();
    let (cache, _) =
        engine::cache::ArtifactCache::open(&directory.path().join("natural-cache"), budget, 16)
            .unwrap();
    let cache = Arc::new(std::sync::Mutex::new(cache));
    let asset = media::SourceAsset::import(media::AssetId(1), input).unwrap();
    let duration = asset.probe().duration.unwrap();
    let mut session = crate::support::control::session();
    let mut cursor = session.snapshot_and_subscribe(&jobs).unwrap().cursor;
    for attempt in 0..2 {
        let providers = providers.clone();
        let cache = cache.clone();
        let asset = asset.clone();
        let (id, _) = jobs
            .submit(
                engine::jobs::JobSpec {
                    kind: engine::jobs::JobKind::Transcribe,
                    class: engine::jobs::ResourceClass::Cpu,
                    input: crate::support::control::key(),
                    bytes: engine::jobs::JobBytes {
                        compositor: engine::transcript::WORK_BYTES,
                        result: engine::transcript::RESULT_BYTES,
                        ..Default::default()
                    },
                },
                move |hooks| {
                    engine::transcript::transcribe(
                        &asset,
                        duration,
                        &lang("en"),
                        &providers,
                        None,
                        cache.as_ref(),
                        &AllowAll,
                        hooks,
                        &Cancellation::new(),
                        hooks,
                    )
                    .map(|output| output.checkpoints.to_le_bytes().to_vec())
                },
            )
            .unwrap();
        crate::support::control::finish(&mut jobs, id);
        let snapshot = jobs.snapshot(id).unwrap();
        if attempt == 0 {
            assert_eq!(
                (snapshot.state, snapshot.attempts),
                (engine::jobs::JobState::Failed, 1)
            );
            assert!(jobs.take_output(id).is_none());
            let raw = snapshot.error.as_ref().unwrap();
            let Some(engine::transcript::Error::Ai(source @ ai::Error::Inference)) = (raw.as_ref()
                as &(dyn std::error::Error + 'static))
                .downcast_ref::<engine::transcript::Error>()
            else {
                panic!("acquisition cause lost")
            };
            assert!(std::ptr::eq(
                source,
                raw.source().unwrap().downcast_ref::<ai::Error>().unwrap()
            ));
            assert_eq!(raw.to_string(), "AI execution failed");
            assert_eq!(raw.code(), engine::control::ErrorCode::Ai);
            assert!(!raw.transient());
            assert!(!raw.job_policy().terminal_cancelled);
            assert_eq!(raw.job_policy().retry_after_seconds, None);
            assert!(Arc::ptr_eq(
                raw,
                jobs.snapshots()
                    .iter()
                    .find(|row| row.id == id)
                    .unwrap()
                    .error
                    .as_ref()
                    .unwrap()
            ));
            assert!(Arc::ptr_eq(
                raw,
                jobs.drain_events()
                    .iter()
                    .find(|row| row.id == id)
                    .unwrap()
                    .error
                    .as_ref()
                    .unwrap()
            ));
            assert!(
                session
                    .snapshot_and_subscribe(&jobs)
                    .unwrap()
                    .jobs
                    .iter()
                    .any(|row| row.id == id && row.error == Some(engine::control::ErrorCode::Ai))
            );
            assert!(session.events(&mut cursor).unwrap().iter().any(|event| matches!(&event.payload, engine::control::Payload::JobChanged { snapshot } if snapshot.id == id && snapshot.error == Some(engine::control::ErrorCode::Ai))));
        } else {
            assert_eq!(snapshot.state, engine::jobs::JobState::Completed);
            assert_eq!(jobs.take_output(id).unwrap().bytes(), 1usize.to_le_bytes());
        }
    }
    assert_eq!(asr_calls.load(Ordering::Relaxed), 1);
    assert_eq!(align_calls.load(Ordering::Relaxed), 2);
}

#[test]
fn an_existing_output_survives_a_refused_export() {
    let Some(input) = fixture() else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("existing.srt");
    std::fs::write(&path, "keep this file").unwrap();
    let access = caption_access(
        &directory.path().join("cache"),
        providers(Arc::new(AtomicUsize::new(0)), false, true),
        Arc::new(AllowAll),
    );
    let (mut runner, _) = runner();
    let mut run = run(&input, "vi", "en", directory.path(), false);
    run.srt = Some(path.clone());
    assert!(workflow(&run, &access, &mut runner).is_err());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "keep this file");
    run.overwrite = true;
    workflow(&run, &access, &mut runner).unwrap();
    assert!(std::fs::read_to_string(path).unwrap().contains("xin chào"));
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
fn registry_maximum_models_require_scoped_unknown_price_for_generation_and_translation() {
    use crate::support::control::{Allow, caller, finish, ok};
    use ai::transport::ApiKey;
    use engine::providers::{CatalogOperation, ConsentPrompt, Cost, UploadRequest};
    use serde_json::{Value, json};
    use std::sync::Mutex;
    use transport::{HttpRequest, HttpResponse, HttpTransport};
    struct Transport {
        model: String,
        calls: Mutex<Vec<Operation>>,
    }
    impl HttpTransport for Transport {
        fn send(&self, request: HttpRequest) -> Result<HttpResponse, transport::Error> {
            let body = if request.url == ai::elevenlabs::ENDPOINT {
                assert!(
                    request
                        .body
                        .windows(self.model.len())
                        .any(|s| s == self.model.as_bytes())
                );
                self.calls.lock().unwrap().push(Operation::Transcribe);
                json!({"text":"Hello world.","words":[
                    {"text":"Hello ","start":0.1,"end":0.5,"type":"word"},
                    {"text":"world.","start":0.5,"end":1.5,"type":"word"}
                ]})
            } else {
                assert_eq!(request.url, ai::openai::TRANSLATION_ENDPOINT);
                let wire: Value = serde_json::from_slice(&request.body).unwrap();
                assert_eq!(wire["model"], self.model);
                self.calls.lock().unwrap().push(Operation::Translate);
                let window: TranslationWindow =
                    serde_json::from_str(wire["input"].as_str().unwrap()).unwrap();
                let translated = json!({"translations":window.segments.iter().map(|s|
                    json!({"id":s.id,"text":"Xin chào"})).collect::<Vec<_>>()});
                json!({"status":"completed","output":[{"type":"message","role":"assistant","status":"completed","content":[{"type":"output_text","text":translated.to_string()}]}]})
            };
            Ok(HttpResponse {
                status: 200,
                headers: vec![],
                body: serde_json::to_vec(&body).unwrap(),
            })
        }
    }
    struct Approve {
        configs: Vec<ProviderConfig>,
        seen: Mutex<Vec<CatalogOperation>>,
    }
    impl ConsentPrompt for Approve {
        fn approve(&self, request: &UploadRequest) -> engine::providers::Result<()> {
            assert!(self.configs.contains(&request.config));
            assert_eq!(request.model, request.config.model().as_str());
            assert_eq!(request.model.len(), 256);
            assert_eq!(request.cost, Cost::Unknown);
            Ok(())
        }
        fn approve_unknown_price(&self, request: &UploadRequest) -> engine::providers::Result<()> {
            self.approve(request)?;
            self.seen.lock().unwrap().push(request.operation);
            Ok(())
        }
    }
    fn cost_refusal(runner: &mut JobRunner, id: engine::jobs::JobId) {
        finish(runner, id);
        let snapshot = runner.snapshot(id).unwrap();
        assert_eq!(snapshot.state, engine::jobs::JobState::Failed);
        let cause = snapshot.error.as_ref().unwrap();
        assert!(matches!(
            (cause.as_ref() as &(dyn std::error::Error + 'static))
                .downcast_ref::<engine::subtitles::Error>(),
            Some(engine::subtitles::Error::Providers(
                engine::providers::Error::Ai(Error::Cost)
            )) | Some(engine::subtitles::Error::Transcript(
                engine::transcript::Error::Providers(engine::providers::Error::Ai(Error::Cost))
            ))
        ));
    }
    let input = fixture().expect("synthetic media oracle must be available");
    let root = tempfile::tempdir().unwrap();
    let (mut runner, budget) = runner();
    let (cache, _) = engine::cache::ArtifactCache::open(root.path(), budget, 16).unwrap();
    let cache = Arc::new(Mutex::new(cache));
    let mut references = Vec::new();
    for suffix in ['a', 'b'] {
        let model = format!("{}{suffix}", "x".repeat(255));
        let asr_config = ai::elevenlabs::ElevenLabsTranscriber::provider_config(&model).unwrap();
        let translation_config = ai::openai::OpenAiTranslator::provider_config(&model).unwrap();
        let transport = Arc::new(Transport {
            model: model.clone(),
            calls: Mutex::new(Vec::new()),
        });
        let asr = Arc::new(
            ai::elevenlabs::ElevenLabsTranscriber::new(
                asr_config.clone(),
                ApiKey::new("fixture").unwrap(),
                transport.clone(),
            )
            .unwrap(),
        );
        let translator = Arc::new(
            ai::openai::OpenAiTranslator::new(
                translation_config.clone(),
                ApiKey::new("fixture").unwrap(),
                transport.clone(),
            )
            .unwrap(),
        );
        let providers = Arc::new(
            Providers::from_parts(
                TranscriptionProvider::ElevenLabs {
                    model: model.clone(),
                },
                ai::elevenlabs::MODE.into(),
                asr,
                None,
                Some(TranslationProvider::OpenAi {
                    model: model.clone(),
                }),
                Some(translator.clone()),
            )
            .unwrap(),
        );
        let approve = Arc::new(Approve {
            configs: vec![asr_config, translation_config],
            seen: Mutex::new(Vec::new()),
        });
        let access =
            subtitles::CaptionAccess::new(providers.clone(), approve.clone(), cache.clone());
        let ordinary = subtitles::CaptionAccess::new(providers, Arc::new(AllowAll), cache.clone());
        let mut session = registry_session(&input);
        let params = json!({"asset":1,"source_language":"en"});
        let denied = caption_submit(
            &mut session,
            &mut runner,
            &ordinary,
            "captions.generate",
            params.clone(),
            "deny-price",
        );
        cost_refusal(&mut runner, denied);
        assert!(transport.calls.lock().unwrap().is_empty());
        let generated = caption_submit(
            &mut session,
            &mut runner,
            &access,
            "captions.generate",
            params.clone(),
            "generate",
        );
        finish(&mut runner, generated);
        let result = subtitles::decode_captions(runner.output_bytes(generated).unwrap()).unwrap();
        assert_eq!(result.model.as_str(), model);
        assert_eq!(
            result.checkpoints, 0,
            "a different maximum model cannot reuse the checkpoint"
        );
        let cached = caption_submit(
            &mut session,
            &mut runner,
            &ordinary,
            "captions.generate",
            params,
            "cached",
        );
        finish(&mut runner, cached);
        assert_eq!(
            subtitles::decode_captions(runner.output_bytes(cached).unwrap())
                .unwrap()
                .checkpoints,
            1
        );
        assert_eq!(*transport.calls.lock().unwrap(), [Operation::Transcribe]);
        ok(caption_command(
            &mut session,
            &mut runner,
            &access,
            "captions.apply",
            json!({"job":generated}),
            "apply",
        ));
        let item = session
            .project()
            .data()
            .scene()
            .timeline
            .iter()
            .flat_map(|t| &t.items)
            .find(|i| matches!(i.content, project::ItemParams::Caption(_)))
            .unwrap()
            .id;
        let params = json!({"item":item,"target_language":"vi"});
        let denied = caption_submit(
            &mut session,
            &mut runner,
            &ordinary,
            "captions.translate",
            params.clone(),
            "deny-translation-price",
        );
        cost_refusal(&mut runner, denied);
        assert_eq!(*transport.calls.lock().unwrap(), [Operation::Transcribe]);
        let translated = caption_submit(
            &mut session,
            &mut runner,
            &access,
            "captions.translate",
            params,
            "translate",
        );
        finish(&mut runner, translated);
        let snapshot = runner.snapshot(translated).unwrap();
        assert_eq!(
            snapshot.state,
            engine::jobs::JobState::Completed,
            "{:?}",
            snapshot.error
        );
        let result =
            subtitles::decode_translations(runner.output_bytes(translated).unwrap()).unwrap();
        assert_eq!(result.model.as_str(), model);
        assert_eq!(result.translation_ref.as_str().len(), 76);
        references.push(result.translation_ref.clone());
        let ids = result.texts.keys().cloned().collect();
        assert_eq!(
            *transport.calls.lock().unwrap(),
            [Operation::Transcribe, Operation::Translate]
        );
        assert_eq!(
            *approve.seen.lock().unwrap(),
            [CatalogOperation::Transcribe, CatalogOperation::Translate]
        );
        let request = crate::support::control::request(
            &session,
            "retranslate",
            json!({"item":result.item,"target_language":result.target_language,
                "texts":result.texts,"translation_ref":result.translation_ref,"force":false}),
            "apply-translation",
        );
        let request = serde_json::from_value(
            engine::control::parse_frame(&serde_json::to_vec(&request).unwrap()).unwrap(),
        )
        .unwrap();
        ok(session
            .control_request(request, caller(&Allow), &mut runner)
            .unwrap());
        let adapter = engine::transcript::SubtitleTranslator::from_parts(
            translator,
            ai::openai::TRANSLATION_MODE.into(),
        );
        engine::transcript::translate_selected(
            &mut session,
            &adapter,
            &mut runner,
            approve.as_ref(),
            &SilentProgress,
            &engine::transcript::Cancellation::new(),
            &engine::transcript::TranslationSelection {
                item,
                ids,
                target: lang("vi"),
                glossary: vec![],
                force: false,
            },
            &mut Cost::Free,
        )
        .unwrap();
        let caption = session
            .project()
            .data()
            .scene()
            .timeline
            .iter()
            .flat_map(|t| &t.items)
            .find(|i| i.id == item)
            .unwrap();
        let project::ItemParams::Caption(caption) = &caption.content else {
            panic!("caption");
        };
        assert_eq!(
            caption.cues.cues[0].texts[&lang("vi")]
                .translation_ref
                .as_ref(),
            references.last()
        );
        assert_eq!(
            *transport.calls.lock().unwrap(),
            [
                Operation::Transcribe,
                Operation::Translate,
                Operation::Translate
            ]
        );
    }
    assert_ne!(references[0], references[1]);
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
    // Optional setup retains one typed source across access clones and real
    // caption refusals, without admitting a job or disabling ordinary edits.
    let engine::RuntimeError::Failed(original) = unavailable.cache().err().unwrap() else {
        panic!("missing setup envelope")
    };
    let cloned = unavailable.clone();
    let engine::RuntimeError::Failed(reused) = cloned.cache().err().unwrap() else {
        panic!("missing cloned setup envelope")
    };
    assert!(Arc::ptr_eq(&original, &reused));
    assert!(matches!(
        (original.as_ref() as &(dyn std::error::Error + 'static))
            .downcast_ref::<subtitles::Error>(),
        Some(subtitles::Error::Providers(
            engine::providers::Error::ModelMissing("whisper-large-v3-turbo")
        ))
    ));
    assert_eq!(
        original.to_string(),
        "local model whisper-large-v3-turbo is not installed"
    );
    assert_eq!(original.code(), ErrorCode::Setup);
    assert!(original.source().is_none());
    assert!(!original.transient());
    assert!(!original.job_policy().terminal_cancelled);
    assert_eq!(original.job_policy().retry_after_seconds, None);
    let counts = r.counts();
    error(
        caption_command(
            &mut s,
            &mut r,
            &cloned,
            "captions.generate",
            json!({"asset":1}),
            "setup-clone",
        ),
        ErrorCode::Setup,
    );
    assert_eq!(r.counts(), counts);
    assert!(Arc::ptr_eq(s.project(), &before));
    assert_eq!(s.history_len(), history);
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
    let (cache, _) =
        engine::cache::ArtifactCache::open(&dir.path().join("cache"), budget, 16).unwrap();
    let cache = Arc::new(std::sync::Mutex::new(cache));
    // The CLI workflow wrote this checkpoint; a registry job reuses it without an upload.
    let written =
        engine::subtitles::CaptionAccess::new(providers.clone(), Arc::new(AllowAll), cache.clone());
    workflow(
        &run(&input, "en", "en", dir.path(), false),
        &written,
        &mut r,
    )
    .unwrap();
    let access = engine::subtitles::CaptionAccess::new(providers, Arc::new(DenyUploads), cache);
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
            "transcribe".into(),
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
        let snapshot = r.snapshot(job).unwrap();
        let raw = snapshot.error.as_ref().unwrap();
        if behavior_value == 1 {
            let Some(subtitles::Error::Transcript(engine::transcript::Error::Ai(
                source @ Error::Inference,
            ))) = (raw.as_ref() as &(dyn std::error::Error + 'static))
                .downcast_ref::<subtitles::Error>()
            else {
                panic!("caption producer changed")
            };
            assert!(std::ptr::eq(
                source,
                raw.source().unwrap().downcast_ref::<Error>().unwrap()
            ));
            assert_eq!(raw.to_string(), "AI execution failed");
            assert_eq!(raw.code(), engine::control::ErrorCode::Ai);
            assert_eq!(snapshot.attempts, 1);
        } else {
            assert!(matches!(
                (raw.as_ref() as &(dyn std::error::Error + 'static))
                    .downcast_ref::<engine::RuntimeError>(),
                Some(engine::RuntimeError::Cancelled)
            ));
        }
        assert!(!raw.transient());
        assert_eq!(raw.job_policy().retry_after_seconds, None);
        let listed = r.snapshots().into_iter().find(|row| row.id == job).unwrap();
        assert!(Arc::ptr_eq(raw, listed.error.as_ref().unwrap()));
        let event = r
            .drain_events()
            .into_iter()
            .find(|row| row.id == job)
            .unwrap();
        assert!(Arc::ptr_eq(raw, event.error.as_ref().unwrap()));
        let resync = s.snapshot_and_subscribe(&r).unwrap();
        assert!(resync.jobs.iter().any(|row| row.id == job
            && row.error == Some(raw.code())
            && row.state == snapshot.state));
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
    let too_many: Vec<_> = (0..129)
        .map(|n| json!({"term": format!("term-{n}"), "translation": null}))
        .collect();
    for (n, (cues, glossary, expected)) in [
        (json!([]), json!([]), ErrorCode::Limit),
        (json!(["cue-1", "cue-1"]), json!([]), ErrorCode::Refused),
        (json!(["missing"]), json!([]), ErrorCode::Refused),
        (json!(["cue-0"]), json!([]), ErrorCode::Refused),
        (json!(["cue-1"]), json!(too_many), ErrorCode::Ai),
    ]
    .into_iter()
    .enumerate()
    {
        let response = caption_command(
            &mut s,
            &mut r,
            &access,
            "captions.translate",
            json!({"item":4,"cues":cues,"target_language":"vi","glossary":glossary}),
            &format!("invalid-{n}"),
        );
        error(response, expected);
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
    assert!(result.translation_ref.as_str().starts_with("translation:"));
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
        json!({"item":4,"cues":["cue-0"],"target_language":"vi","force":true,
            "glossary":[{"term":"cue","translation":"tín hiệu"},{"term":"Vibe","translation":null}]}),
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
        "transcribe".into(),
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
        "transcribe".into(),
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
            "transcribe".into(),
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
            "transcribe".into(),
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
        "transcribe".into(),
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

/// Recognises every window as one word at the given confidence.
struct ConfidenceAsr {
    inner: FakeAsr,
    confidence: f32,
}
impl Transcriber for ConfidenceAsr {
    fn config(&self) -> &ProviderConfig {
        self.inner.config()
    }
    fn capabilities(&self, scope: &Scope) -> Capabilities {
        let mut capabilities = self.inner.capabilities(scope);
        capabilities.timing = BTreeSet::from([TimingUnit::Segment, TimingUnit::Word]);
        capabilities
    }
    fn estimate(&self, scope: &Scope) -> CostEstimate {
        self.inner.estimate(scope)
    }
    fn transcribe(
        &self,
        audio: ai::AudioWindow<'_>,
        _: &Request<'_>,
        _: &dyn ExecutionHooks,
    ) -> Result<Vec<RecognitionSegment>, Error> {
        Ok(vec![RecognitionSegment {
            id: format!("fake:{}", audio.range.start()).into(),
            text: "hello".into(),
            range: Some(audio.range),
            timing: vec![ai::TimedText {
                text: "hello".into(),
                unit: TimingUnit::Word,
                range: audio.range,
                provenance: captions::TimingProvenance::Native,
                speaker: None,
                confidence: captions::Confidence::try_from(self.confidence).ok(),
            }],
            raw_tokens: Vec::new(),
        }])
    }
}

/// These rows own the outcomes the desktop's auto-detect retry reacts to: the job fails with
/// the typed transcript error, so the window maps `NoSpeech` and `LowConfidence` from the
/// retained cause, with explicit or omitted language, and publishes nothing.
#[test]
fn registry_generation_fails_typed_for_silent_media_and_low_confidence_recognition() {
    use serde_json::json;
    let dir = tempfile::tempdir().unwrap();
    let Some(silent) = testkit::oracle::generate(
        &testkit::fixture::dir("engine"),
        "silent captions",
        "caption-silent-3s.wav",
        &[
            "-f",
            "lavfi",
            "-i",
            "anullsrc=r=16000:cl=mono",
            "-t",
            "3",
            "-c:a",
            "pcm_s16le",
        ],
    ) else {
        return;
    };
    let Some(tone) = fixture() else {
        return;
    };
    for (media, confidence, language, expected_no_speech) in [
        (&silent, 0.9, Some("en"), true),
        (&silent, 0.9, None, true),
        (&tone, 0.1, Some("en"), false),
    ] {
        let providers = Providers::from_parts(
            TranscriptionProvider::Whisper {
                model: "fake-asr-model".into(),
            },
            "transcribe".into(),
            Arc::new(ConfidenceAsr {
                inner: FakeAsr::new(Arc::new(AtomicUsize::new(0))),
                confidence,
            }),
            None,
            None,
            None,
        )
        .unwrap();
        let access = caption_access(&dir.path().join("cache"), providers, Arc::new(DenyUploads));
        let (mut r, _) = runner();
        let mut s = registry_session(media);
        let before = s.project().clone();
        let job = caption_submit(
            &mut s,
            &mut r,
            &access,
            "captions.generate",
            json!({"asset":1,"source_language":language}),
            "typed-failure",
        );
        let error = settle(&mut r, job).unwrap_err();
        let transcript = match cause(&error) {
            engine::subtitles::Error::Transcript(transcript) => transcript,
            other => panic!("{other:?}"),
        };
        assert_eq!(
            matches!(transcript, engine::transcript::Error::NoSpeech),
            expected_no_speech,
            "{language:?}: {transcript:?}"
        );
        assert!(
            expected_no_speech || matches!(transcript, engine::transcript::Error::LowConfidence),
            "{transcript:?}"
        );
        assert!(r.output_bytes(job).is_none());
        assert_eq!(s.project(), &before);
    }
}

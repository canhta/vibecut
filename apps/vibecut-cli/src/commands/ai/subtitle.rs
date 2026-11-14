//! Adapts native subtitle requests to the engine workflow.
use crate::args::CliError;
use crate::args::translation_required;
use crate::args::utf8;
use crate::output::{CliConsent, cost_json, cost_label};
use crate::output::{Output, json_path};
use crate::session::Context;
use crate::session::{Lent, cli_runner, cli_runtime, run_command, run_job_with, same_target};
use engine::Command;
use engine::cache::ArtifactCache;
use engine::control::{CostView, JobOutput, Policy};
use engine::jobs::JobRunner;
use engine::providers::{Cost, Providers};
use engine::subtitles::{CaptionAccess, LanguageTag};
use engine::transcript::source_session;
use media::{AssetId, SourceAsset, StreamKind};
use project::{ItemId, ItemParams, TrackId};
use serde_json::{Value, json};
use std::ffi::OsString;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use time::{Tick, TimeRange};

pub(crate) fn run(context: &Context, args: Vec<OsString>) -> Result<Output, CliError> {
    let mut args = args.into_iter();
    let mut input: Option<PathBuf> = None;
    let mut from = "auto".to_owned();
    let mut to: Option<String> = None;
    let mut bilingual = false;
    let mut srt: Option<PathBuf> = None;
    let mut burn: Option<PathBuf> = None;
    let mut portrait = None;
    let mut provider = "whisper.cpp".to_owned();
    let mut model: Option<String> = None;
    let mut translate_provider = "openai".to_owned();
    let mut translate_model: Option<String> = None;
    let mut yes_upload = false;
    let mut accept_unknown_price = false;
    let mut overwrite = false;
    while let Some(arg) = args.next() {
        let flag = utf8(&arg, "subtitle flag")?;
        match flag {
            "--from" => {
                from = utf8(
                    &args
                        .next()
                        .ok_or_else(|| CliError::Usage("--from requires a language".into()))?,
                    "source language",
                )?
                .to_owned();
            }
            "--to" => {
                to = Some(
                    utf8(
                        &args
                            .next()
                            .ok_or_else(|| CliError::Usage("--to requires a language".into()))?,
                        "target language",
                    )?
                    .to_owned(),
                );
            }
            "--bilingual" => bilingual = true,
            "--srt" => {
                srt = Some(PathBuf::from(args.next().ok_or_else(|| {
                    CliError::Usage("--srt requires an output path".into())
                })?));
            }
            "--burn" => {
                burn = Some(PathBuf::from(args.next().ok_or_else(|| {
                    CliError::Usage("--burn requires an output path".into())
                })?));
            }
            "--aspect" => {
                let raw = args
                    .next()
                    .ok_or_else(|| CliError::Usage("--aspect requires 9:16 or 16:9".into()))?;
                let value = utf8(&raw, "aspect")?;
                portrait = Some(match value {
                    "9:16" => true,
                    "16:9" => false,
                    _ => return Err(CliError::Usage("--aspect must be 9:16 or 16:9".into())),
                });
            }
            "--provider" => {
                provider = utf8(
                    &args
                        .next()
                        .ok_or_else(|| CliError::Usage("--provider requires an id".into()))?,
                    "provider",
                )?
                .to_owned();
            }
            "--model" => {
                model = Some(
                    utf8(
                        &args
                            .next()
                            .ok_or_else(|| CliError::Usage("--model requires an id".into()))?,
                        "model",
                    )?
                    .to_owned(),
                );
            }
            "--translate-provider" => {
                translate_provider = utf8(
                    &args.next().ok_or_else(|| {
                        CliError::Usage("--translate-provider requires an id".into())
                    })?,
                    "translation provider",
                )?
                .to_owned();
            }
            "--translate-model" => {
                translate_model = Some(
                    utf8(
                        &args.next().ok_or_else(|| {
                            CliError::Usage("--translate-model requires an id".into())
                        })?,
                        "translation model",
                    )?
                    .to_owned(),
                );
            }
            "--yes-upload" => yes_upload = true,
            "--accept-unknown-price" => accept_unknown_price = true,
            "--overwrite" => overwrite = true,
            "--help" | "-h" => {
                return Ok(Output::new(
                    json!({"usage": subtitle_usage()}),
                    subtitle_usage().to_owned(),
                    false,
                    0,
                ));
            }
            other if other.starts_with('-') => {
                return Err(CliError::Usage(format!("unknown subtitle flag: {other}")));
            }
            _ => {
                if input.is_some() {
                    return Err(CliError::Usage(
                        "subtitle takes one input media path".into(),
                    ));
                }
                input = Some(PathBuf::from(arg));
            }
        }
    }
    let input = input.ok_or_else(subtitle_usage_error)?;
    let to = to.ok_or_else(subtitle_usage_error)?;
    if srt.is_none() && burn.is_none() {
        return Err(CliError::Usage(
            "subtitle requires --srt and/or --burn".into(),
        ));
    }
    if portrait.is_some() && burn.is_none() {
        return Err(CliError::Usage("--aspect requires --burn".into()));
    }
    let source_language = match from.as_str() {
        "auto" => None,
        value => {
            if !["vi", "en", "zh"].contains(&value) {
                return Err(CliError::Usage("--from must be vi, en, zh or auto".into()));
            }
            Some(engine::providers::parse_language(value)?)
        }
    };
    let target_language =
        engine::providers::normalize_target_tag(&engine::providers::parse_language(&to)?)?;
    if !["vi", "en", "zh-Hans", "zh-Hant"].contains(&target_language.as_str()) {
        return Err(CliError::Usage(
            "--to must be vi, en, zh, zh-Hans or zh-Hant".into(),
        ));
    }
    if bilingual
        && source_language
            .as_ref()
            .is_some_and(|source| source == &target_language)
    {
        return Err(CliError::Usage(
            "--bilingual requires --from and --to to differ".into(),
        ));
    }
    let transcription = engine::providers::choose_transcription(&provider, model.as_deref())?;
    let credentials = engine::providers::Credentials::from_env();
    let runtime = cli_runtime()?;
    let budget = context.budget()?;
    let root = engine::cache::CacheRoot::discover()?.models();
    let (mut cache, _diagnostics) = ArtifactCache::open(&root, budget.clone(), 128)?;
    let cancellation = engine::transcript::Cancellation::new();
    let mut source_language = source_language;
    let translation = match &source_language {
        Some(source) if translation_required(source, &target_language, bilingual) => Some(
            engine::providers::choose_translation(&translate_provider, translate_model.as_deref())?,
        ),
        _ => None,
    };
    let mut providers = engine::providers::Providers::open(
        transcription,
        translation,
        &mut cache,
        &credentials,
        &runtime,
    )?;
    if source_language.is_none() {
        let detected = providers.detect_language_for(&input, &cancellation)?;
        if bilingual && detected == target_language {
            return Err(CliError::Usage(
                "--bilingual requires the detected source and --to to differ".into(),
            ));
        }
        if translation_required(&detected, &target_language, bilingual) {
            providers = providers.with_translation(
                engine::providers::choose_translation(
                    &translate_provider,
                    translate_model.as_deref(),
                )?,
                &credentials,
            )?;
        }
        source_language = Some(detected);
    }
    let plan = Plan {
        input,
        source_language: source_language.ok_or(CliError::Rejected("source language unresolved"))?,
        target_language,
        bilingual,
        srt,
        burn,
        portrait,
        overwrite,
    };
    let providers = Arc::new(providers);
    let access = CaptionAccess::new(
        providers.clone(),
        Arc::new(CliConsent::new(yes_upload, accept_unknown_price)),
        Arc::new(Mutex::new(cache)),
    );
    let report = execute(&plan, &access, cli_runner(budget)?)?;
    Ok(Output::new(
        report_json(&plan, &providers, &report)?,
        report_human(&plan, &providers, &report),
        false,
        0,
    ))
}

/// Everything the registry commands need, fixed before any job starts.
struct Plan {
    input: PathBuf,
    source_language: LanguageTag,
    target_language: LanguageTag,
    bilingual: bool,
    srt: Option<PathBuf>,
    burn: Option<PathBuf>,
    /// `Some(true)` selects 1080x1920; `Some(false)` 1920x1080; `None` keeps the canvas.
    portrait: Option<bool>,
    overwrite: bool,
}

/// What the finished jobs reported; the adapter owns no other result state.
struct Report {
    segments: usize,
    cues: usize,
    checkpoints: usize,
    transcription_cost: Cost,
    translation_cost: Cost,
    elapsed: Duration,
}

/// Authorises exactly the requested output paths, replacing only with `--overwrite`.
struct ExactOutputs<'a> {
    paths: Vec<&'a Path>,
    overwrite: bool,
}
impl Policy for ExactOutputs<'_> {
    fn authorize(&self, command: &Command, _: &project::Project) -> engine::control::Result<()> {
        match command.published_path() {
            Some((path, replace))
                if !self.paths.contains(&path.as_path()) || (replace && !self.overwrite) =>
            {
                Err(engine::control::Error::Control(
                    engine::control::ControlError::Permission,
                ))
            }
            _ => Ok(()),
        }
    }
}

fn refuse(error: engine::EditError) -> CliError {
    engine::subtitles::Error::Edit(error).into()
}

/// The workflow as registry commands over an in-memory project holding the input as
/// asset 1: generate, apply, translate, retranslate, then the requested exports.
fn execute(plan: &Plan, access: &CaptionAccess, mut runner: JobRunner) -> Result<Report, CliError> {
    let started = Instant::now();
    let asset = SourceAsset::import(AssetId(1), plan.input.clone())?;
    let duration = asset
        .probe()
        .duration
        .filter(|value| *value > Tick::ZERO)
        .ok_or_else(|| refuse(engine::EditError::Unsupported("media duration is unknown")))?;
    if plan.burn.is_some()
        && !asset
            .probe()
            .streams
            .iter()
            .any(|stream| stream.kind == StreamKind::Video)
    {
        return Err(refuse(engine::EditError::Unsupported(
            "burn-in requires a video stream",
        )));
    }
    if let (Some(srt), Some(burn)) = (&plan.srt, &plan.burn)
        && same_target(srt, burn)?
    {
        return Err(refuse(engine::EditError::Invalid(
            "subtitle outputs must differ",
        )));
    }
    for output in plan.srt.iter().chain(&plan.burn) {
        if same_target(&plan.input, output)? {
            return Err(refuse(engine::EditError::Invalid(
                "output must not replace the input media",
            )));
        }
    }
    let translate = plan.source_language != plan.target_language;
    let mut session = source_session(&asset, duration, plan.burn.is_some(), plan.portrait)?;
    let lent = Lent {
        captions: Some(access),
        ..Lent::default()
    };
    let generated = run_job_with(
        &mut session,
        &mut runner,
        lent,
        "captions.generate",
        json!({"asset": 1, "source_language": plan.source_language}),
    )?;
    let Some(JobOutput::Captions(generated_output)) = generated.output else {
        return Err(CliError::Rejected("transcription produced no captions"));
    };
    run_command(
        &mut session,
        &mut runner,
        lent,
        "captions.apply",
        json!({"job": generated.id}),
        true,
    )?;
    let (track, item) = caption_ids(&session)?;
    let mut translation_cost = Cost::Free;
    if translate {
        let translated = run_job_with(
            &mut session,
            &mut runner,
            lent,
            "captions.translate",
            json!({"item": item, "target_language": plan.target_language}),
        )?;
        let Some(JobOutput::Translations(result)) = translated.output else {
            return Err(CliError::Rejected("translation produced no texts"));
        };
        translation_cost = cost_of(&result.cost);
        run_command(
            &mut session,
            &mut runner,
            lent,
            "retranslate",
            json!({
                "item": result.item,
                "target_language": result.target_language,
                "texts": result.texts,
                "translation_ref": result.translation_ref,
                "force": false,
            }),
            true,
        )?;
    }
    let languages = if translate && plan.bilingual {
        vec![plan.source_language.clone(), plan.target_language.clone()]
    } else {
        vec![plan.target_language.clone()]
    };
    let policy = ExactOutputs {
        paths: plan
            .srt
            .iter()
            .chain(&plan.burn)
            .map(PathBuf::as_path)
            .collect(),
        overwrite: plan.overwrite,
    };
    let lent = Lent {
        policy: Some(&policy),
        ..Lent::default()
    };
    if let Some(path) = &plan.srt {
        run_job_with(
            &mut session,
            &mut runner,
            lent,
            "export_subtitles",
            json!({
                "track": track,
                "format": "srt",
                "languages": languages,
                "path": path,
                "overwrite": plan.overwrite,
            }),
        )?;
    }
    if let Some(path) = &plan.burn {
        run_job_with(
            &mut session,
            &mut runner,
            lent,
            "burn_subtitles",
            json!({
                "range": TimeRange::new(Tick::ZERO, duration)?,
                "languages": languages,
                "style": render::CaptionStyle::default(),
                "portrait": plan.portrait.unwrap_or(false),
                "path": path,
                "overwrite": plan.overwrite,
            }),
        )?;
    }
    runner.shutdown();
    Ok(Report {
        segments: generated_output.segments,
        cues: generated_output.cues.len(),
        checkpoints: generated_output.checkpoints,
        transcription_cost: cost_of(&generated_output.cost),
        translation_cost,
        elapsed: started.elapsed(),
    })
}

pub(super) fn cost_of(view: &CostView) -> Cost {
    match view {
        CostView::Free => Cost::Free,
        CostView::UpperBound { currency, micros } => Cost::UpperBound {
            currency: currency.clone(),
            micros: *micros,
        },
        CostView::Unknown => Cost::Unknown,
    }
}

/// The caption track and item the applied transcription created.
pub(super) fn caption_ids(session: &engine::Session) -> Result<(TrackId, ItemId), CliError> {
    session
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .find_map(|track| {
            track
                .items
                .iter()
                .find(|item| matches!(item.content, ItemParams::Caption(_)))
                .map(|item| (track.id, item.id))
        })
        .ok_or(CliError::Rejected(
            "applied transcription has no caption item",
        ))
}

fn subtitle_usage() -> &'static str {
    "usage: vibecut-cli subtitle <in> [--from vi|en|zh|auto] --to vi|en|zh [--bilingual]\n\
                     [--srt <out>] [--burn <out.mp4>] [--aspect 9:16]\n\
                     [--provider <id>] [--model <id>] [--translate-provider <id>]\n\
                     [--translate-model <id>] [--yes-upload] [--accept-unknown-price] [--overwrite]"
}
fn subtitle_usage_error() -> CliError {
    CliError::Usage(subtitle_usage().into())
}

fn report_json(plan: &Plan, providers: &Providers, report: &Report) -> Result<Value, CliError> {
    let path = |value: &Option<PathBuf>| -> Result<Value, CliError> {
        match value {
            Some(value) => Ok(Value::String(json_path(value)?)),
            None => Ok(Value::Null),
        }
    };
    Ok(json!({
        "status": "completed",
        "input": json_path(&plan.input)?,
        "source_language": plan.source_language.as_str(),
        "target_language": plan.target_language.as_str(),
        "provider": {
            "id": providers.transcription().id(),
            "model": providers.transcription().model(),
        },
        "translation_provider": providers.translation().map(|value| json!({
            "id": value.id(),
            "model": value.model(),
        })),
        "segments": report.segments,
        "cues": report.cues,
        "checkpoints": report.checkpoints,
        "srt": path(&plan.srt)?,
        "burn": path(&plan.burn)?,
        "transcription_cost": cost_json(&report.transcription_cost),
        "translation_cost": cost_json(&report.translation_cost),
        "elapsed_seconds": report.elapsed.as_secs_f64(),
    }))
}
fn report_human(plan: &Plan, providers: &Providers, report: &Report) -> String {
    let transcription = providers.transcription();
    let mut human = format!(
        "source: {}\ntarget: {}\nprovider: {} ({})\n",
        plan.source_language.as_str(),
        plan.target_language.as_str(),
        transcription.id(),
        transcription.model()
    );
    match providers.translation() {
        Some(value) => {
            let _ = writeln!(human, "translation: {} ({})", value.id(), value.model());
        }
        None => human.push_str("translation: none\n"),
    }
    let _ = writeln!(
        human,
        "segments: {}\ncues: {}\ncheckpoints reused: {}",
        report.segments, report.cues, report.checkpoints
    );
    if let Some(path) = &plan.srt {
        let _ = writeln!(human, "srt: {}", path.display());
    }
    if let Some(path) = &plan.burn {
        let _ = writeln!(human, "burn: {}", path.display());
    }
    let _ = writeln!(
        human,
        "cost: transcribe {}; translate {}\nelapsed: {:.3} s",
        cost_label(&report.transcription_cost),
        cost_label(&report.translation_cost),
        report.elapsed.as_secs_f64()
    );
    human.trim_end().to_owned()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]
    use super::*;
    use engine::providers::{AllowAll, DenyUploads, TranscriptionProvider, TranslationProvider};
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn config(id: &str, model: &str) -> ai::ProviderConfig {
        ai::ProviderConfig::new(
            id.into(),
            model.into(),
            "cli-v1".into(),
            ai::Location::Remote {
                backend: "fixture".into(),
                base_url: "https://fixture.invalid".into(),
                auth_ref: "fixture".into(),
            },
        )
        .unwrap()
    }
    fn capabilities(
        config: &ai::ProviderConfig,
        operation: ai::Operation,
        scope: &ai::Scope,
    ) -> ai::Capabilities {
        ai::Capabilities {
            model: config.model().clone(),
            // The engine requests these exact modes; a mismatch refuses Unsupported.
            mode: match operation {
                ai::Operation::Transcribe => "transcribe".into(),
                _ => ai::providers::openai::TRANSLATION_MODE.into(),
            },
            operation,
            languages: vec![scope.source_language.clone()],
            target_languages: scope.target_language.iter().cloned().collect(),
            timing: [ai::TimingUnit::Segment].into_iter().collect(),
            diarization: ai::Support::Unsupported,
            live_audio_input: ai::Support::Unsupported,
            file_result_stream: ai::Support::Unsupported,
            max_upload_bytes: None,
            max_duration_micros: None,
            evidence: ai::Evidence::FixtureVerified,
            revision: "cli-offline".into(),
        }
    }
    struct Asr(ai::ProviderConfig, Arc<AtomicUsize>);
    impl ai::Transcriber for Asr {
        fn config(&self) -> &ai::ProviderConfig {
            &self.0
        }
        fn capabilities(&self, scope: &ai::Scope) -> ai::Capabilities {
            capabilities(&self.0, ai::Operation::Transcribe, scope)
        }
        fn estimate(&self, _: &ai::Scope) -> ai::CostEstimate {
            ai::CostEstimate::Free
        }
        fn transcribe(
            &self,
            audio: ai::AudioWindow<'_>,
            _: &ai::Request<'_>,
            _: &dyn ai::ExecutionHooks,
        ) -> Result<Vec<ai::RecognitionSegment>, ai::Error> {
            self.1.fetch_add(1, Ordering::AcqRel);
            Ok(vec![ai::RecognitionSegment {
                id: "cue".into(),
                text: "Hello there".into(),
                range: Some(audio.range),
                timing: Vec::new(),
                raw_tokens: Vec::new(),
            }])
        }
    }
    struct Translator(ai::ProviderConfig);
    impl ai::Translator for Translator {
        fn config(&self) -> &ai::ProviderConfig {
            &self.0
        }
        fn capabilities(&self, scope: &ai::Scope) -> ai::Capabilities {
            capabilities(&self.0, ai::Operation::Translate, scope)
        }
        fn estimate(&self, _: &ai::Scope) -> ai::CostEstimate {
            ai::CostEstimate::Free
        }
        fn translate(
            &self,
            window: &ai::TranslationWindow,
            _: &ai::Request<'_>,
            _: &dyn ai::ExecutionHooks,
        ) -> Result<ai::Translation, ai::Error> {
            Ok(ai::Translation {
                source_language: window.source_language.clone(),
                target_language: window.target_language.clone(),
                texts: window
                    .segments
                    .iter()
                    .map(|segment| (segment.id.clone(), "Xin chao".into()))
                    .collect(),
                number_warnings: Vec::new(),
            })
        }
    }

    fn access(
        root: &Path,
        calls: &Arc<AtomicUsize>,
        consent: Arc<dyn engine::providers::ConsentPrompt + Send + Sync>,
    ) -> (CaptionAccess, Arc<Providers>) {
        let providers = Arc::new(
            Providers::from_parts(
                TranscriptionProvider::ElevenLabs {
                    model: "offline-model".into(),
                },
                "transcribe".into(),
                Arc::new(Asr(config("offline-asr", "offline-model"), calls.clone())),
                None,
                Some(TranslationProvider::OpenAi {
                    model: "offline-translation-model".into(),
                }),
                Some(Arc::new(Translator(config(
                    "offline-translator",
                    "offline-translation-model",
                )))),
            )
            .unwrap(),
        );
        let (cache, _) = ArtifactCache::open(
            &root.join("cache"),
            engine::ByteBudget::new(512 << 20).unwrap(),
            16,
        )
        .unwrap();
        (
            CaptionAccess::new(providers.clone(), consent, Arc::new(Mutex::new(cache))),
            providers,
        )
    }

    #[test]
    fn subtitle_workflow_exports_translated_srt_reuses_checkpoints_and_honours_consent_and_overwrite()
     {
        if !testkit::oracle::require("subtitle workflow") {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let input = root.path().join("input.mp4");
        let tone = "sine=frequency=997:sample_rate=44100:duration=3";
        let video = "testsrc2=size=128x96:rate=30:duration=3";
        let os = std::ffi::OsStr::new;
        testkit::oracle::ffmpeg(&[
            os("-f"),
            os("lavfi"),
            os("-i"),
            os(video),
            os("-f"),
            os("lavfi"),
            os("-i"),
            os(tone),
            os("-c:v"),
            os("libx264"),
            os("-pix_fmt"),
            os("yuv420p"),
            os("-c:a"),
            os("aac"),
            os("-shortest"),
            input.as_os_str(),
        ]);
        let calls = Arc::new(AtomicUsize::new(0));
        let plan = |name: &str, bilingual: bool| Plan {
            input: input.clone(),
            source_language: engine::providers::parse_language("en").unwrap(),
            target_language: engine::providers::parse_language("vi").unwrap(),
            bilingual,
            srt: Some(root.path().join(name)),
            burn: None,
            portrait: None,
            overwrite: false,
        };
        let runner = || cli_runner(Context.budget().unwrap()).unwrap();

        let (denied, _) = access(root.path(), &calls, Arc::new(DenyUploads));
        assert!(execute(&plan("refused.srt", false), &denied, runner()).is_err());
        assert_eq!(calls.load(Ordering::Relaxed), 0);
        assert!(!root.path().join("refused.srt").exists());
        drop(denied);

        let (allowed, providers) = access(root.path(), &calls, Arc::new(AllowAll));
        let first = plan("first.srt", true);
        let report = execute(&first, &allowed, runner()).unwrap();
        assert_eq!((report.checkpoints, calls.load(Ordering::Relaxed)), (0, 1));
        let srt = std::fs::read_to_string(first.srt.as_ref().unwrap()).unwrap();
        assert!(
            srt.contains("Hello there") && srt.contains("Xin chao"),
            "{srt}"
        );
        let value = report_json(&first, &providers, &report).unwrap();
        assert_eq!(value["status"], "completed");
        assert_eq!(value["source_language"], "en");
        assert_eq!(value["translation_provider"]["id"], "openai");
        assert_eq!(value["cues"], report.cues);
        assert!(report_human(&first, &providers, &report).contains("translation: openai"));

        let second = plan("second.srt", false);
        let report = execute(&second, &allowed, runner()).unwrap();
        assert_eq!((report.checkpoints, calls.load(Ordering::Relaxed)), (1, 1));
        assert!(
            !std::fs::read_to_string(second.srt.as_ref().unwrap())
                .unwrap()
                .contains("Hello there")
        );

        let mut again = plan("second.srt", false);
        assert!(execute(&again, &allowed, runner()).is_err());
        again.overwrite = true;
        execute(&again, &allowed, runner()).unwrap();

        let mut same = plan("clash.srt", false);
        same.burn = same.srt.clone();
        assert!(execute(&same, &allowed, runner()).is_err());
        let mut over_input = plan("unused.srt", false);
        over_input.srt = Some(input.clone());
        let before = std::fs::metadata(&input).unwrap().len();
        assert!(execute(&over_input, &allowed, runner()).is_err());
        assert_eq!(std::fs::metadata(&input).unwrap().len(), before);
    }

    #[test]
    fn output_policy_authorises_only_the_requested_paths_and_overwrite_only_when_granted() {
        let srt = PathBuf::from("out/a.srt");
        let empty = project::Project::empty(project::ProjectId(0), String::new().into()).unwrap();
        let export = |path: &str, overwrite: bool| Command::ExportSubtitles {
            track: TrackId(1),
            format: engine::Format::Srt,
            languages: Vec::new(),
            path: PathBuf::from(path),
            overwrite,
        };
        for (granted, path, overwrite, allowed) in [
            (false, "out/a.srt", false, true),
            (false, "out/a.srt", true, false),
            (true, "out/a.srt", true, true),
            (true, "out/other.srt", false, false),
            (true, "out/other.srt", true, false),
        ] {
            let policy = ExactOutputs {
                paths: vec![srt.as_path()],
                overwrite: granted,
            };
            assert_eq!(
                policy.authorize(&export(path, overwrite), &empty).is_ok(),
                allowed,
                "granted={granted} path={path} overwrite={overwrite}"
            );
        }
    }
}

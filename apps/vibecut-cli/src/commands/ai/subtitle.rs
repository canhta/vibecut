//! Adapts native subtitle requests to the engine workflow.
use crate::args::CliError;
use crate::args::translation_required;
use crate::args::utf8;
use crate::output::CliProgress;
use crate::output::{CliConsent, cost_json, cost_label};
use crate::output::{Output, json_path};
use crate::session::Context;
use crate::session::{cli_runner, cli_runtime};
use engine::cache::ArtifactCache;
use engine::subtitles;
use engine::subtitles::{SubtitleOutcome, SubtitleRequest};
use serde_json::{Value, json};
use std::ffi::OsString;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

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
    let credentials = engine::providers::Credentials::new(
        std::env::var("OPENAI_API_KEY").ok(),
        std::env::var("ELEVENLABS_API_KEY").ok(),
        std::env::var("ANTHROPIC_API_KEY").ok(),
    )?;
    let runtime = cli_runtime()?;
    let budget = context.budget()?;
    let root = engine::providers::default_cache_root()?;
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
    let mut runner = cli_runner(budget)?;
    let request = SubtitleRequest {
        input: input.clone(),
        source_language,
        target_language,
        bilingual,
        srt,
        burn,
        portrait,
        overwrite,
    };
    let progress = CliProgress::new();
    let consent = CliConsent::new(yes_upload);
    let outcome = subtitles::run(
        &request,
        &providers,
        &mut runner,
        &mut cache,
        &consent,
        &progress,
        &cancellation,
    )?;
    runner.shutdown();
    let value = outcome_json(&outcome, &input)?;
    Ok(Output::new(
        value,
        outcome_human_subtitle(&outcome),
        false,
        0,
    ))
}

fn subtitle_usage() -> &'static str {
    "usage: vibecut-cli subtitle <in> [--from vi|en|zh|auto] --to vi|en|zh [--bilingual]\n\
                     [--srt <out>] [--burn <out.mp4>] [--aspect 9:16]\n\
                     [--provider <id>] [--model <id>] [--translate-provider <id>]\n\
                     [--translate-model <id>] [--yes-upload] [--overwrite]"
}
fn subtitle_usage_error() -> CliError {
    CliError::Usage(subtitle_usage().into())
}

fn outcome_json(outcome: &SubtitleOutcome, input: &Path) -> Result<Value, CliError> {
    let path = |value: &Option<PathBuf>| -> Result<Value, CliError> {
        match value {
            Some(value) => Ok(Value::String(json_path(value)?)),
            None => Ok(Value::Null),
        }
    };
    Ok(json!({
        "status": "completed",
        "input": json_path(input)?,
        "source_language": outcome.source_language.as_str(),
        "target_language": outcome.target_language.as_str(),
        "provider": {
            "id": outcome.transcription.id(),
            "model": outcome.transcription.model(),
        },
        "translation_provider": outcome.translation.as_ref().map(|value| json!({
            "id": value.id(),
            "model": value.model(),
        })),
        "segments": outcome.segments,
        "cues": outcome.cues,
        "checkpoints": outcome.checkpoints,
        "srt": path(&outcome.srt)?,
        "burn": path(&outcome.burn)?,
        "transcription_cost": cost_json(&outcome.transcription_cost),
        "translation_cost": cost_json(&outcome.translation_cost),
        "elapsed_seconds": outcome.elapsed.as_secs_f64(),
    }))
}
fn outcome_human_subtitle(outcome: &SubtitleOutcome) -> String {
    let mut human = format!(
        "source: {}\ntarget: {}\nprovider: {} ({})\n",
        outcome.source_language.as_str(),
        outcome.target_language.as_str(),
        outcome.transcription.id(),
        outcome.transcription.model()
    );
    match &outcome.translation {
        Some(value) => {
            let _ = writeln!(human, "translation: {} ({})", value.id(), value.model());
        }
        None => human.push_str("translation: none\n"),
    }
    let _ = writeln!(
        human,
        "segments: {}\ncues: {}\ncheckpoints reused: {}",
        outcome.segments, outcome.cues, outcome.checkpoints
    );
    if let Some(path) = &outcome.srt {
        let _ = writeln!(human, "srt: {}", path.display());
    }
    if let Some(path) = &outcome.burn {
        let _ = writeln!(human, "burn: {}", path.display());
    }
    let _ = writeln!(
        human,
        "cost: transcribe {}; translate {}\nelapsed: {:.3} s",
        cost_label(&outcome.transcription_cost),
        cost_label(&outcome.translation_cost),
        outcome.elapsed.as_secs_f64()
    );
    human.trim_end().to_owned()
}

//! Adapts native clip requests to the shorts workflow.
use super::subtitle::{caption_ids, cost_of};
use crate::args::CliError;
use crate::args::{number_arg, parse_selection, seconds_arg, seconds_ticks, utf8};
use crate::output::{CliConsent, cost_json, cost_label, wait_for_job};
use crate::output::{Output, json_path, tick_json};
use crate::session::Context;
use crate::session::{Lent, cli_runner_sized, cli_runtime, run_command, run_job_with};
use engine::cache::ArtifactCache;
use engine::control::{
    ClipCandidateView, ClipEdge, ClipExportShort, ClipRefusal, ClipsResult, ControlResult,
    JobOutput,
};
use engine::jobs::JobRunner;
use engine::providers::Cost;
use engine::subtitles::{CaptionAccess, LanguageTag};
use engine::transcript::{build_session, source_session};
use media::{AssetId, SourceAsset};
use serde_json::{Value, json};
use shorts::{ClipJobs, clip_suggest};
use std::ffi::OsString;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use time::Tick;

pub(crate) fn run(context: &Context, args: Vec<OsString>) -> Result<Output, CliError> {
    let mut args = args.into_iter();
    let mut input: Option<PathBuf> = None;
    let mut from = "auto".to_owned();
    let mut count = 5usize;
    let mut min = 20.0f64;
    let mut max = 60.0f64;
    let mut min_segment = 2.0f64;
    let mut max_segments = 4usize;
    let mut overlap = 0.5f64;
    let mut keywords: Vec<String> = Vec::new();
    let mut provider = "heuristic".to_owned();
    let mut model: Option<String> = None;
    let mut asr_provider = "whisper.cpp".to_owned();
    let mut asr_model: Option<String> = None;
    let mut translate_provider = "openai".to_owned();
    let mut translate_model: Option<String> = None;
    let mut select: Option<Vec<usize>> = None;
    let mut export: Option<PathBuf> = None;
    let mut subs: Option<String> = None;
    let mut bilingual = false;
    let mut aspect: Option<String> = None;
    let mut yes_upload = false;
    let mut accept_unknown_price = false;
    let mut overwrite = false;
    let mut allow_degraded = false;
    while let Some(arg) = args.next() {
        let flag = utf8(&arg, "clips flag")?;
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
            "--count" => count = number_arg(&mut args, "--count")?,
            "--min" => min = seconds_arg(&mut args, "--min")?,
            "--max" => max = seconds_arg(&mut args, "--max")?,
            "--min-segment" => min_segment = seconds_arg(&mut args, "--min-segment")?,
            "--max-segments" => max_segments = number_arg(&mut args, "--max-segments")?,
            "--overlap" => {
                let value = args
                    .next()
                    .ok_or_else(|| CliError::Usage("--overlap requires a ratio".into()))?;
                overlap = utf8(&value, "overlap")?
                    .parse()
                    .map_err(|_| CliError::Usage("--overlap must be a number in 0..=1".into()))?;
            }
            "--keywords" => {
                let value = args
                    .next()
                    .ok_or_else(|| CliError::Usage("--keywords requires a list".into()))?;
                keywords = utf8(&value, "keywords")?
                    .split(',')
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_owned)
                    .collect();
            }
            "--provider" => {
                provider = utf8(
                    &args
                        .next()
                        .ok_or_else(|| CliError::Usage("--provider requires an id".into()))?,
                    "suggestion provider",
                )?
                .to_owned();
            }
            "--model" => {
                model = Some(
                    utf8(
                        &args
                            .next()
                            .ok_or_else(|| CliError::Usage("--model requires an id".into()))?,
                        "suggestion model",
                    )?
                    .to_owned(),
                );
            }
            "--asr-provider" => {
                asr_provider = utf8(
                    &args
                        .next()
                        .ok_or_else(|| CliError::Usage("--asr-provider requires an id".into()))?,
                    "ASR provider",
                )?
                .to_owned();
            }
            "--asr-model" => {
                asr_model = Some(
                    utf8(
                        &args
                            .next()
                            .ok_or_else(|| CliError::Usage("--asr-model requires an id".into()))?,
                        "ASR model",
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
            "--select" => {
                let value = args
                    .next()
                    .ok_or_else(|| CliError::Usage("--select requires indices".into()))?;
                select = Some(parse_selection(utf8(&value, "selection")?)?);
            }
            "--export" => {
                export = Some(PathBuf::from(args.next().ok_or_else(|| {
                    CliError::Usage("--export requires an output directory".into())
                })?));
            }
            "--subs" => {
                subs = Some(
                    utf8(
                        &args
                            .next()
                            .ok_or_else(|| CliError::Usage("--subs requires a language".into()))?,
                        "subtitle language",
                    )?
                    .to_owned(),
                );
            }
            "--bilingual" => bilingual = true,
            "--aspect" => {
                aspect = Some(
                    utf8(
                        &args
                            .next()
                            .ok_or_else(|| CliError::Usage("--aspect requires 9:16".into()))?,
                        "aspect",
                    )?
                    .to_owned(),
                );
            }
            "--yes-upload" => yes_upload = true,
            "--accept-unknown-price" => accept_unknown_price = true,
            "--overwrite" => overwrite = true,
            "--allow-degraded" => allow_degraded = true,
            "--help" | "-h" => {
                return Ok(Output::new(
                    json!({"usage": clips_usage()}),
                    clips_usage().to_owned(),
                    false,
                    0,
                ));
            }
            other if other.starts_with('-') => {
                return Err(CliError::Usage(format!("unknown clips flag: {other}")));
            }
            _ => {
                if input.is_some() {
                    return Err(CliError::Usage("clips takes one input media path".into()));
                }
                input = Some(PathBuf::from(arg));
            }
        }
    }
    let input = input.ok_or_else(clips_usage_error)?;
    if !(1..=32).contains(&count) || !(1..=32).contains(&max_segments) {
        return Err(CliError::Usage(
            "--count and --max-segments must be 1..=32".into(),
        ));
    }
    if !min.is_finite() || !max.is_finite() || !min_segment.is_finite() || min <= 0.0 {
        return Err(CliError::Usage(
            "--min, --max and --min-segment must be positive".into(),
        ));
    }
    if max < min || min_segment > max {
        return Err(CliError::Usage(
            "--max must be at least --min and --min-segment".into(),
        ));
    }
    if !overlap.is_finite() || !(0.0..=1.0).contains(&overlap) {
        return Err(CliError::Usage(
            "--overlap must be a number in 0..=1".into(),
        ));
    }
    if keywords.len() > 64 || keywords.iter().any(|value| value.len() > 256) {
        return Err(CliError::Usage(
            "at most 64 keywords of at most 256 bytes each".into(),
        ));
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
    let target_language = match &subs {
        None => None,
        Some(value) => {
            if !["vi", "en", "zh"].contains(&value.as_str()) {
                return Err(CliError::Usage("--subs must be vi, en or zh".into()));
            }
            Some(engine::providers::normalize_target_tag(
                &engine::providers::parse_language(value)?,
            )?)
        }
    };
    if export.is_none() && (subs.is_some() || bilingual || aspect.is_some() || select.is_some()) {
        return Err(CliError::Usage(
            "--subs, --bilingual, --aspect and --select require --export".into(),
        ));
    }
    if export.is_some() && subs.is_none() {
        return Err(CliError::Usage("--export requires --subs".into()));
    }
    if let Some(aspect) = &aspect
        && aspect != "9:16"
    {
        if aspect == "16:9" {
            return Err(CliError::Unsupported("clip export is portrait 9:16 only"));
        }
        return Err(CliError::Usage("--aspect must be 9:16".into()));
    }
    // Cheap pre-checks first: a refusal here costs no model load, no ASR and no upload.
    let suggestion = clip_suggest::choose_suggestion(&provider, model.as_deref())?;
    let transcription =
        engine::providers::choose_transcription(&asr_provider, asr_model.as_deref())?;
    let credentials = engine::providers::Credentials::from_env();
    clip_suggest::check_suggestion_credentials(&suggestion, &credentials)?;
    let runtime = cli_runtime()?;
    let budget = context.budget()?;
    // The process opens the models cache once; the lent access shares it with every job.
    let root = engine::cache::CacheRoot::discover()?.models();
    let (mut cache, _diagnostics) = ArtifactCache::open(&root, budget.clone(), 128)?;
    let cancellation = engine::transcript::Cancellation::new();
    let mut providers = engine::providers::Providers::open(
        transcription,
        None,
        &mut cache,
        &credentials,
        &runtime,
    )?;
    let source_language = match source_language {
        Some(language) => language,
        None => providers.detect_language_for(&input, &cancellation)?,
    };
    if let Some(target) = &target_language {
        if bilingual && &source_language == target {
            return Err(CliError::Usage(
                "--bilingual requires the source and --subs to differ".into(),
            ));
        }
        if &source_language != target || bilingual {
            providers = providers.with_translation(
                engine::providers::choose_translation(
                    &translate_provider,
                    translate_model.as_deref(),
                )?,
                &credentials,
            )?;
        }
    }
    let access = CaptionAccess::new(
        Arc::new(providers),
        Arc::new(CliConsent::new(yes_upload, accept_unknown_price)),
        Arc::new(Mutex::new(cache)),
    )
    .with_clips(Arc::new(ClipJobs::new(credentials)));
    let plan = Plan {
        input,
        source_language,
        target_language,
        count,
        min: seconds_ticks(min)?,
        max: seconds_ticks(max)?,
        min_segment: seconds_ticks(min_segment)?,
        max_segments,
        overlap,
        keywords,
        provider,
        model,
        select,
        export,
        bilingual,
        overwrite,
        allow_degraded,
    };
    let report = execute(&plan, &access, cli_runner_sized(budget, 32)?)?;
    Ok(Output::new(
        clips_json(&plan, &report)?,
        clips_human(&report),
        false,
        0,
    ))
}

/// Everything the registry commands need, fixed before any job starts.
struct Plan {
    input: PathBuf,
    source_language: LanguageTag,
    target_language: Option<LanguageTag>,
    count: usize,
    min: Tick,
    max: Tick,
    min_segment: Tick,
    max_segments: usize,
    overlap: f64,
    keywords: Vec<String>,
    provider: String,
    model: Option<String>,
    select: Option<Vec<usize>>,
    export: Option<PathBuf>,
    bilingual: bool,
    overwrite: bool,
    allow_degraded: bool,
}

/// What the finished jobs reported; the adapter owns no other result state.
struct Report {
    result: ClipsResult,
    shorts: Vec<ClipExportShort>,
    languages: Vec<LanguageTag>,
    translation_cost: Cost,
}

/// The workflow as registry commands: `clips.suggest`, then for an export the caption item
/// the cues become, optional `captions.translate` with `retranslate`, and `clips.export`.
fn execute(plan: &Plan, access: &CaptionAccess, mut runner: JobRunner) -> Result<Report, CliError> {
    let asset = SourceAsset::import(AssetId(1), plan.input.clone())?;
    let duration = asset
        .probe()
        .duration
        .filter(|value| *value > Tick::ZERO)
        .ok_or(CliError::Rejected("media duration is unknown"))?;
    let mut session = source_session(&asset, duration, false, None)?;
    let lent = Lent {
        captions: Some(access),
        ..Lent::default()
    };
    let suggested = run_job_with(
        &mut session,
        &mut runner,
        lent,
        "clips.suggest",
        json!({
            "asset": 1,
            "source_language": plan.source_language,
            "provider": plan.provider,
            "model": plan.model,
            "count": plan.count,
            "min_duration": plan.min,
            "max_duration": plan.max,
            "max_segments": plan.max_segments,
            "min_segment_duration": plan.min_segment,
            "overlap_tolerance": plan.overlap,
            "keywords": plan.keywords,
        }),
    )?;
    let Some(JobOutput::Clips(result)) = suggested.output else {
        return Err(CliError::Rejected("clip suggestion produced no candidates"));
    };
    let result = *result;
    let mut selected = match &plan.select {
        Some(values) => values.clone(),
        None => (0..result.clips.len()).collect(),
    };
    for index in &selected {
        if *index >= result.clips.len() {
            return Err(CliError::Usage(format!(
                "--select index {} is outside 1..={}",
                index + 1,
                result.clips.len()
            )));
        }
    }
    selected.sort_unstable();
    let languages = match &plan.target_language {
        Some(target) if plan.bilingual => vec![result.source_language.clone(), target.clone()],
        Some(target) => vec![target.clone()],
        None => Vec::new(),
    };
    let mut report = Report {
        result,
        shorts: Vec::new(),
        languages,
        translation_cost: Cost::Free,
    };
    let Some(directory) = &plan.export else {
        runner.shutdown();
        return Ok(report);
    };
    if selected.is_empty() {
        return Err(CliError::Rejected("no clip candidates to export"));
    }
    // The cues become the caption item the export burns; translation is the ordinary
    // caption command pair over that item, so consent and cost are shared with `subtitle`.
    let mut captions = build_session(&asset, &report.result.cues, duration, true, None)?;
    let (_, item) = caption_ids(&captions)?;
    if let Some(target) = &plan.target_language
        && &report.result.source_language != target
    {
        let translated = run_job_with(
            &mut captions,
            &mut runner,
            lent,
            "captions.translate",
            json!({"item": item, "target_language": target}),
        )?;
        let Some(JobOutput::Translations(translation)) = translated.output else {
            return Err(CliError::Rejected("translation produced no texts"));
        };
        report.translation_cost = cost_of(&translation.cost);
        run_command(
            &mut captions,
            &mut runner,
            lent,
            "retranslate",
            json!({
                "item": translation.item,
                "target_language": translation.target_language,
                "texts": translation.texts,
                "translation_ref": translation.translation_ref,
                "force": false,
            }),
            true,
        )?;
    }
    let accepted = run_command(
        &mut captions,
        &mut runner,
        lent,
        "clips.export",
        json!({
            "job": suggested.id,
            "select": selected,
            "item": item,
            "languages": report.languages,
            "directory": directory,
            "overwrite": plan.overwrite,
            "allow_degraded": plan.allow_degraded,
        }),
        true,
    )?;
    let ControlResult::ClipExports { shorts, .. } = accepted else {
        return Err(CliError::Rejected("command did not start export jobs"));
    };
    report.shorts = shorts;
    wait_for_shorts(&mut runner, &report.shorts)?;
    runner.shutdown();
    Ok(report)
}

/// Wait for every short's own job. The first failure cancels and drains the rest, so no
/// admitted sibling is left running; shorts that already published stay complete.
fn wait_for_shorts(runner: &mut JobRunner, shorts: &[ClipExportShort]) -> Result<(), CliError> {
    for (index, short) in shorts.iter().enumerate() {
        if let Err(error) = wait_for_job(runner, short.job) {
            for later in shorts.iter().skip(index + 1) {
                let _ = runner.cancel_job(later.job);
            }
            for later in shorts.iter().skip(index + 1) {
                let _ = wait_for_job(runner, later.job);
            }
            return Err(error);
        }
    }
    Ok(())
}

fn clips_usage() -> &'static str {
    "usage: vibecut-cli clips <in> [--from vi|en|zh|auto] [--count N]\n\
                    [--min 20s] [--max 60s] [--max-segments N] [--min-segment 2s]\n\
                    [--provider heuristic|openai|anthropic] [--model <id>]\n\
                    [--asr-provider <id>] [--asr-model <id>] [--keywords a,b]\n\
                    [--export <dir> [--select 1,3] --subs vi|en|zh [--bilingual]\n\
                     [--aspect 9:16] [--translate-provider <id>] [--overwrite]\n\
                     [--allow-degraded]] [--yes-upload] [--accept-unknown-price]"
}
fn clips_usage_error() -> CliError {
    CliError::Usage(clips_usage().into())
}
fn clips_json(plan: &Plan, report: &Report) -> Result<Value, CliError> {
    let result = &report.result;
    let candidates: Vec<Value> = result
        .clips
        .iter()
        .enumerate()
        .map(|(index, candidate)| candidate_json(index, candidate))
        .collect();
    let refusals = |list: &[ClipRefusal]| -> Vec<Value> {
        list.iter()
            .map(|clip| json!({"title": clip.title.as_str(), "reason": clip.reason}))
            .collect()
    };
    let exports: Vec<Value> = report
        .shorts
        .iter()
        .map(|clip| {
            Ok(json!({
                "candidate": clip.candidate + 1,
                "video": json_path(&clip.video)?,
                "subtitles": clip.subtitles.iter().map(|subtitle| {
                    Ok(json!({
                        "format": format_name(subtitle.format),
                        "path": json_path(&subtitle.path)?,
                    }))
                }).collect::<Result<Vec<_>, CliError>>()?,
            }))
        })
        .collect::<Result<Vec<_>, CliError>>()?;
    Ok(json!({
        "status": "completed",
        "input": json_path(&plan.input)?,
        "source_language": result.source_language.as_str(),
        "transcription": {
            "provider": result.transcription_provider,
            "model": result.transcription_model,
        },
        "suggestion": {
            "provider": result.provider,
            "model": result.model,
        },
        "settings": {
            "count": plan.count,
            "min_duration": tick_json(plan.min),
            "max_duration": tick_json(plan.max),
            "max_segments": plan.max_segments,
            "min_segment_duration": tick_json(plan.min_segment),
            "overlap_tolerance": plan.overlap,
            "keywords": plan.keywords,
        },
        "candidates": candidates,
        "rejected": refusals(&result.rejected),
        "dropped": refusals(&result.dropped),
        "transcript_segments": result.transcript_segments,
        "cues": result.cues.len(),
        "checkpoints": result.checkpoints,
        "cost": {
            "transcription": cost_json(&cost_of(&result.transcription_cost)),
            "suggestion": cost_json(&cost_of(&result.suggestion_cost)),
            "translation": cost_json(&report.translation_cost),
        },
        "subtitle_languages": report.languages.iter().map(LanguageTag::as_str).collect::<Vec<_>>(),
        "exports": exports,
        "elapsed_seconds": result.elapsed_seconds,
    }))
}

fn candidate_json(index: usize, candidate: &ClipCandidateView) -> Value {
    json!({
        "index": index + 1,
        "title": candidate.title.as_str(),
        "hook_text": candidate.hook_text.as_str(),
        "reason": candidate.reason.as_str(),
        "score": candidate.score,
        "source": candidate.source,
        "duration": tick_json(candidate.duration),
        "segments": candidate.segments.iter().map(|segment| json!({
            "start": tick_json(segment.range.start()),
            "end": tick_json(segment.range.end()),
            "role": segment.role,
            "source_order": segment.source_order,
        })).collect::<Vec<_>>(),
        "adjustments": candidate.adjustments.iter().map(|pair| json!({
            "start": adjustment_json(&pair.start),
            "end": adjustment_json(&pair.end),
        })).collect::<Vec<_>>(),
        "pieces": candidate.pieces.iter().map(|piece| json!({
            "start_segment_id": piece.start_segment_id.as_str(),
            "end_segment_id": piece.end_segment_id.as_str(),
            "role": piece.role,
        })).collect::<Vec<_>>(),
    })
}
fn adjustment_json(value: &ClipEdge) -> Value {
    json!({
        "original": tick_json(value.original),
        "adjusted": tick_json(value.adjusted),
        "delta": tick_json(value.delta),
        "kind": value.kind,
    })
}
fn format_name(format: engine::Format) -> &'static str {
    match format {
        engine::Format::Srt => "srt",
        engine::Format::WebVtt => "web_vtt",
        engine::Format::Txt => "txt",
    }
}

fn clips_human(report: &Report) -> String {
    let result = &report.result;
    let mut human = format!(
        "source: {}\nprovider: {} ({})\ntranscription: {} ({})\n",
        result.source_language.as_str(),
        result.provider,
        result.model,
        result.transcription_provider,
        result.transcription_model
    );
    let _ = writeln!(
        human,
        "candidates: {} ({} rejected, {} dropped)\ncheckpoints reused: {}",
        result.clips.len(),
        result.rejected.len(),
        result.dropped.len(),
        result.checkpoints
    );
    for (index, candidate) in result.clips.iter().enumerate() {
        let _ = writeln!(
            human,
            "{}. {:.3} s, {} segment(s), score {:.3}: {}",
            index + 1,
            candidate.duration.seconds(),
            candidate.segments.len(),
            candidate.score,
            candidate.title.as_str()
        );
        for segment in &candidate.segments {
            let _ = writeln!(
                human,
                "   {:.3}-{:.3} s {}",
                segment.range.start().seconds(),
                segment.range.end().seconds(),
                segment.role
            );
        }
    }
    for clip in &result.rejected {
        let _ = writeln!(human, "rejected ({}): {}", clip.reason, clip.title.as_str());
    }
    for clip in &report.shorts {
        let _ = writeln!(
            human,
            "exported {}: {}",
            clip.candidate + 1,
            clip.video.display()
        );
        for subtitle in &clip.subtitles {
            let _ = writeln!(
                human,
                "  {}: {}",
                format_name(subtitle.format),
                subtitle.path.display()
            );
        }
    }
    let _ = writeln!(
        human,
        "cost: transcribe {}; suggest {}; translate {}",
        cost_label(&cost_of(&result.transcription_cost)),
        cost_label(&cost_of(&result.suggestion_cost)),
        cost_label(&report.translation_cost)
    );
    human.trim_end().to_owned()
}

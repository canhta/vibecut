//! Adapts native clip requests to the shorts workflow.
use crate::args::CliError;
use crate::args::{number_arg, parse_selection, seconds_arg, seconds_ticks, utf8};
use crate::output::CliProgress;
use crate::output::{CliConsent, cost_json, cost_label};
use crate::output::{Output, json_path, tick_json};
use crate::session::Context;
use crate::session::{cli_runner, cli_runtime};
use engine::cache::ArtifactCache;
use engine::providers::Cost;
use engine::subtitles::LanguageTag;
use engine::{ClipDropReason, ClipRole, ClipSettings, ClipSource, ClipSourceSegment, NfcText};
use serde_json::{Value, json};
use shorts::{
    ClipCandidate, ClipExportRequest, ClipRequest, ClipSuggestionOutcome, SnapSettings,
    clip_suggest,
};
use std::ffi::OsString;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

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
    let suggestion = clip_suggest::choose_suggestion(&provider, model.as_deref())?;
    let transcription =
        engine::providers::choose_transcription(&asr_provider, asr_model.as_deref())?;
    let credentials = engine::providers::Credentials::from_env()?;
    clip_suggest::check_suggestion_credentials(&suggestion, &credentials)?;
    let runtime = cli_runtime()?;
    let budget = context.budget()?;
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
    let settings = ClipSettings {
        count,
        min_duration: seconds_ticks(min)?,
        max_duration: seconds_ticks(max)?,
        max_segments,
        min_segment_duration: seconds_ticks(min_segment)?,
        overlap_tolerance: overlap,
        language: source_language.clone(),
        keywords: keywords
            .iter()
            .map(|value| NfcText::from(value.as_str()))
            .collect(),
    };
    let request = ClipRequest {
        input: input.clone(),
        source_language: Some(source_language.clone()),
        settings: settings.clone(),
        snap: SnapSettings {
            frame_rate: None,
            tolerance: seconds_ticks(1.0)?,
            silence_rms: 0.01,
        },
        provider: suggestion,
    };
    let mut runner = cli_runner(budget)?;
    let progress = CliProgress::new();
    let consent = CliConsent::new(yes_upload);
    let outcome = clip_suggest::suggest(
        &request,
        &credentials,
        &providers,
        &mut runner,
        &mut cache,
        &consent,
        &progress,
        &cancellation,
    )?;
    let mut selected = match select {
        Some(values) => values,
        None => (0..outcome.clips.len()).collect(),
    };
    for index in &selected {
        if *index >= outcome.clips.len() {
            return Err(CliError::Usage(format!(
                "--select index {} is outside 1..={}",
                index + 1,
                outcome.clips.len()
            )));
        }
    }
    selected.sort_unstable();
    let languages = match &target_language {
        Some(target) if bilingual => vec![source_language.clone(), target.clone()],
        Some(target) => vec![target.clone()],
        None => Vec::new(),
    };
    let mut exported = None;
    if let Some(directory) = export {
        if selected.is_empty() {
            return Err(CliError::Rejected("no clip candidates to export"));
        }
        let result = clip_suggest::export(
            &outcome,
            ClipExportRequest {
                directory,
                select: selected,
                languages: languages.clone(),
                formats: Vec::new(),
                overwrite,
                allow_degraded,
                max_output_bytes: 64 << 30,
            },
            &providers,
            &mut runner,
            &consent,
            &progress,
            &cancellation,
        )?;
        exported = Some(result);
    }
    runner.shutdown();
    let value = clips_json(&outcome, &input, &settings, exported.as_ref(), &languages)?;
    Ok(Output::new(
        value,
        clips_human(&outcome, exported.as_ref()),
        false,
        0,
    ))
}

fn clips_usage() -> &'static str {
    "usage: vibecut-cli clips <in> [--from vi|en|zh|auto] [--count N]\n\
                    [--min 20s] [--max 60s] [--max-segments N] [--min-segment 2s]\n\
                    [--provider heuristic|openai|anthropic] [--model <id>]\n\
                    [--asr-provider <id>] [--asr-model <id>] [--keywords a,b]\n\
                    [--export <dir> [--select 1,3] --subs vi|en|zh [--bilingual]\n\
                     [--aspect 9:16] [--translate-provider <id>] [--overwrite]\n\
                     [--allow-degraded]] [--yes-upload]"
}
fn clips_usage_error() -> CliError {
    CliError::Usage(clips_usage().into())
}
fn clips_json(
    outcome: &ClipSuggestionOutcome,
    input: &Path,
    settings: &ClipSettings,
    exported: Option<&shorts::ClipExportOutcome>,
    languages: &[LanguageTag],
) -> Result<Value, CliError> {
    let candidates: Vec<Value> = outcome
        .clips
        .iter()
        .enumerate()
        .map(|(index, candidate)| candidate_json(index, candidate))
        .collect();
    let rejected: Vec<Value> = outcome
        .rejected
        .iter()
        .map(|clip| {
            json!({
                "title": clip.title.as_str(),
                "reason": boundary_failure_name(clip.reason),
            })
        })
        .collect();
    let dropped: Vec<Value> = outcome
        .dropped
        .iter()
        .map(|clip| {
            json!({
                "title": clip.proposal.title.as_str(),
                "reason": drop_reason_name(clip.reason),
            })
        })
        .collect();
    let exports: Vec<Value> = match exported {
        Some(result) => result
            .exports
            .iter()
            .map(|clip| {
                Ok(json!({
                    "candidate": clip.candidate + 1,
                    "video": json_path(&clip.paths.video)?,
                    "subtitles": clip.paths.subtitles.iter().map(|subtitle| {
                        Ok(json!({
                            "format": format_name(subtitle.format),
                            "path": json_path(&subtitle.path)?,
                        }))
                    }).collect::<Result<Vec<_>, CliError>>()?,
                }))
            })
            .collect::<Result<Vec<_>, CliError>>()?,
        None => Vec::new(),
    };
    Ok(json!({
        "status": "completed",
        "input": json_path(input)?,
        "source_language": outcome.source_language.as_str(),
        "transcription": {
            "provider": outcome.transcription_provider,
            "model": outcome.transcription_model,
        },
        "suggestion": {
            "provider": outcome.provider,
            "model": outcome.model,
        },
        "settings": {
            "count": settings.count,
            "min_duration": tick_json(settings.min_duration),
            "max_duration": tick_json(settings.max_duration),
            "max_segments": settings.max_segments,
            "min_segment_duration": tick_json(settings.min_segment_duration),
            "overlap_tolerance": settings.overlap_tolerance,
            "keywords": settings.keywords.iter().map(|value| value.as_str()).collect::<Vec<_>>(),
        },
        "candidates": candidates,
        "rejected": rejected,
        "dropped": dropped,
        "transcript_segments": outcome.transcript_segments,
        "cues": outcome.cues,
        "checkpoints": outcome.checkpoints,
        "cost": {
            "transcription": cost_json(&outcome.transcription_cost),
            "suggestion": cost_json(&outcome.suggestion_cost),
            "translation": exported.map_or(json!({"kind": "free"}), |value| cost_json(&value.translation_cost)),
        },
        "subtitle_languages": languages.iter().map(LanguageTag::as_str).collect::<Vec<_>>(),
        "exports": exports,
        "elapsed_seconds": outcome.elapsed.as_secs_f64(),
    }))
}

fn candidate_json(index: usize, candidate: &ClipCandidate) -> Value {
    json!({
        "index": index + 1,
        "title": candidate.proposal.title.as_str(),
        "hook_text": candidate.proposal.hook_text.as_str(),
        "reason": candidate.proposal.reason.as_str(),
        "score": candidate.proposal.score,
        "source": clip_source_name(candidate.source),
        "duration": tick_json(candidate.duration),
        "segments": candidate.segments.iter().map(segment_json).collect::<Vec<_>>(),
        "adjustments": candidate.adjustments.iter().map(|pair| json!({
            "start": adjustment_json(&pair.start),
            "end": adjustment_json(&pair.end),
        })).collect::<Vec<_>>(),
        "pieces": candidate.proposal.segments.iter().map(|piece| json!({
            "start_segment_id": piece.start_segment_id.as_str(),
            "end_segment_id": piece.end_segment_id.as_str(),
            "role": clip_role_name(piece.role),
        })).collect::<Vec<_>>(),
    })
}
fn segment_json(segment: &ClipSourceSegment) -> Value {
    json!({
        "start": tick_json(segment.range.start()),
        "end": tick_json(segment.range.end()),
        "role": clip_role_name(segment.role),
        "source_order": segment.source_order,
    })
}
fn adjustment_json(value: &shorts::clip_boundaries::BoundaryAdjustment) -> Value {
    json!({
        "original": tick_json(value.original),
        "adjusted": tick_json(value.adjusted),
        "delta": tick_json(value.delta),
        "kind": boundary_kind_name(value.kind),
    })
}
fn clip_source_name(source: ClipSource) -> &'static str {
    match source {
        ClipSource::Llm => "llm",
        ClipSource::Heuristic => "heuristic",
    }
}
fn clip_role_name(role: ClipRole) -> &'static str {
    match role {
        ClipRole::Hook => "hook",
        ClipRole::Context => "context",
        ClipRole::Payoff => "payoff",
    }
}
fn boundary_kind_name(kind: shorts::clip_boundaries::BoundaryKind) -> &'static str {
    match kind {
        shorts::clip_boundaries::BoundaryKind::SentenceSilence => "sentence_silence",
        shorts::clip_boundaries::BoundaryKind::Sentence => "sentence",
        shorts::clip_boundaries::BoundaryKind::Silence => "silence",
        shorts::clip_boundaries::BoundaryKind::WordOrSegment => "word_or_segment",
        shorts::clip_boundaries::BoundaryKind::WordFade => "word_fade",
    }
}
fn boundary_failure_name(reason: shorts::clip_boundaries::BoundaryFailure) -> &'static str {
    match reason {
        shorts::clip_boundaries::BoundaryFailure::NoSafeFrame => "no_safe_frame",
        shorts::clip_boundaries::BoundaryFailure::SegmentDuration => "segment_duration",
        shorts::clip_boundaries::BoundaryFailure::TotalDuration => "total_duration",
        shorts::clip_boundaries::BoundaryFailure::SegmentOverlap => "segment_overlap",
        shorts::clip_boundaries::BoundaryFailure::Candidate => "candidate",
    }
}
fn drop_reason_name(reason: ClipDropReason) -> &'static str {
    match reason {
        ClipDropReason::Duration => "duration",
        ClipDropReason::SegmentDuration => "segment_duration",
        ClipDropReason::SegmentCount => "segment_count",
        ClipDropReason::UnknownId => "unknown_id",
        ClipDropReason::ReversedRange => "reversed_range",
        ClipDropReason::HookOrder => "hook_order",
        ClipDropReason::SegmentOverlap => "segment_overlap",
        ClipDropReason::Duplicate => "duplicate",
        ClipDropReason::Overlap => "overlap",
        ClipDropReason::CountLimit => "count_limit",
    }
}
fn format_name(format: engine::Format) -> &'static str {
    match format {
        engine::Format::Srt => "srt",
        engine::Format::WebVtt => "web_vtt",
        engine::Format::Txt => "txt",
    }
}

fn clips_human(
    outcome: &ClipSuggestionOutcome,
    exported: Option<&shorts::ClipExportOutcome>,
) -> String {
    let mut human = format!(
        "source: {}\nprovider: {} ({})\ntranscription: {} ({})\n",
        outcome.source_language.as_str(),
        outcome.provider,
        outcome.model,
        outcome.transcription_provider,
        outcome.transcription_model
    );
    let _ = writeln!(
        human,
        "candidates: {} ({} rejected, {} dropped)\ncheckpoints reused: {}",
        outcome.clips.len(),
        outcome.rejected.len(),
        outcome.dropped.len(),
        outcome.checkpoints
    );
    for (index, candidate) in outcome.clips.iter().enumerate() {
        let _ = writeln!(
            human,
            "{}. {:.3} s, {} segment(s), score {:.3}: {}",
            index + 1,
            candidate.duration.seconds(),
            candidate.segments.len(),
            candidate.proposal.score,
            candidate.proposal.title.as_str()
        );
        for segment in &candidate.segments {
            let _ = writeln!(
                human,
                "   {:.3}-{:.3} s {}",
                segment.range.start().seconds(),
                segment.range.end().seconds(),
                clip_role_name(segment.role)
            );
        }
    }
    for clip in &outcome.rejected {
        let _ = writeln!(
            human,
            "rejected ({}): {}",
            boundary_failure_name(clip.reason),
            clip.title.as_str()
        );
    }
    if let Some(result) = exported {
        for clip in &result.exports {
            let _ = writeln!(
                human,
                "exported {}: {}",
                clip.candidate + 1,
                clip.paths.video.display()
            );
            for subtitle in &clip.paths.subtitles {
                let _ = writeln!(
                    human,
                    "  {}: {}",
                    format_name(subtitle.format),
                    subtitle.path.display()
                );
            }
        }
    }
    let _ = writeln!(
        human,
        "cost: transcribe {}; suggest {}; translate {}",
        cost_label(&outcome.transcription_cost),
        cost_label(&outcome.suggestion_cost),
        cost_label(&exported.map_or(Cost::Free, |value| value.translation_cost.clone()))
    );
    human.trim_end().to_owned()
}

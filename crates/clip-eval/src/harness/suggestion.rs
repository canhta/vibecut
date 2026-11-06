//! Calls one provider and snaps the clips it proposes to source boundaries.

use super::{NoHooks, RangeReport, SuggestionReport};
use crate::budget::Budget;
use crate::label::ResolvedSettings;
use crate::{Error, Result};
use ai::clips::{ClipInput, ClipSource, ClipSuggester, MAX_WIRE, MODE, suggest_clips};
use ai::{Consent, CostEstimate, Money, Operation, Request, SampleRange, Scope};
use captions::{LanguageProfile, Transcript};
use shorts::clip_boundaries::{ClipBoundarySettings, snap_clip_boundaries};
use std::path::Path;
use std::sync::Arc;
use time::{SourceRange, Tick};

pub(super) struct ProviderRun {
    pub(super) shorts: Vec<Vec<ai::clips::ClipSourceSegment>>,
    pub(super) report: Vec<SuggestionReport>,
    pub(super) snap_failures: usize,
}

fn scope_for(provider: &dyn ClipSuggester, window: &ClipInput, source_sha: &str) -> Result<Scope> {
    Ok(Scope {
        provider: provider.config().clone(),
        operation: Operation::SuggestClips,
        payload_fingerprint: format!("clip-eval:{source_sha}").into(),
        mode: MODE.into(),
        texts: vec![window.consent_text()?],
        source_language: window.settings.language.clone(),
        target_language: None,
        source_samples: SampleRange::new(0, 1_000)?,
        sample_rate: 1,
        max_bytes: MAX_WIRE,
    })
}

pub(super) fn estimate_provider(
    provider: &dyn ClipSuggester,
    input: &ClipInput,
    source_sha: &str,
) -> Result<u64> {
    let windows = input.windows()?;
    let mut total = 0u64;
    for window in &windows {
        let scope = scope_for(provider, window, source_sha)?;
        total = total.saturating_add(estimate_micros(&provider.estimate(&scope))?);
    }
    Ok(total)
}

fn estimate_micros(estimate: &CostEstimate) -> Result<u64> {
    match estimate {
        CostEstimate::Free => Ok(0),
        CostEstimate::UpperBound(money) => Ok(money.micros()),
        CostEstimate::Unknown => Err(Error::UnknownCost),
    }
}

#[derive(Clone, Copy)]
pub(super) struct RunContext<'a> {
    pub(super) provider: &'a dyn ClipSuggester,
    pub(super) input: &'a ClipInput,
    pub(super) transcript: &'a Transcript,
    pub(super) profile: &'a LanguageProfile,
    pub(super) settings: &'a ResolvedSettings,
    pub(super) source_sha: &'a str,
    pub(super) source: &'a Path,
    pub(super) energy: bool,
}

pub(super) fn run_provider(
    context: &RunContext<'_>,
    notes: &mut Vec<String>,
    estimate: u64,
    budget: &mut Budget,
) -> Result<ProviderRun> {
    let RunContext {
        provider,
        input,
        transcript,
        profile,
        settings,
        source_sha,
        source,
        energy: energy_enabled,
    } = *context;
    budget.reserve(estimate)?;
    let windows = input.windows()?;
    let mut scopes = Vec::with_capacity(windows.len());
    for window in &windows {
        scopes.push(scope_for(provider, window, source_sha)?);
    }
    let mut consents = Vec::with_capacity(scopes.len());
    for scope in &scopes {
        if provider.config().is_remote() {
            let estimate = provider.estimate(scope);
            let micros = estimate_micros(&estimate)?;
            consents.push(Some(Consent::grant(
                scope.clone(),
                Money::new("USD", micros)?,
            )?));
        } else {
            consents.push(None);
        }
    }
    let requests: Vec<_> = scopes
        .iter()
        .zip(&consents)
        .map(|(scope, consent)| Request {
            scope,
            consent: consent.as_ref(),
        })
        .collect();
    let result = suggest_clips(provider, input, &requests, &NoHooks)?;
    let mut report = Vec::with_capacity(result.clips.len());
    for clip in &result.clips {
        report.push(SuggestionReport {
            score: clip.proposal.score,
            source: match clip.source {
                ClipSource::Llm => "llm".into(),
                ClipSource::Heuristic => "heuristic".into(),
            },
            title: clip.proposal.title.as_str().to_owned(),
            segments: clip
                .segments
                .iter()
                .map(|segment| RangeReport {
                    start: segment.range.start().0,
                    end: segment.range.end().0,
                })
                .collect(),
        });
    }

    let energy = if energy_enabled {
        match energy_windows(source, transcript) {
            Ok(energy) => energy,
            Err(error) => {
                notes.push(format!("energy unavailable: {error}"));
                Vec::new()
            }
        }
    } else {
        Vec::new()
    };
    let source_range = source_range(source, transcript, energy_enabled)?;
    let boundary = ClipBoundarySettings {
        source_range,
        frame_rate: settings.frame_rate,
        tolerance: settings.snap_tolerance,
        silence_rms: settings.silence_rms,
        min_duration: settings.min_duration,
        max_duration: settings.max_duration,
        min_segment_duration: settings.min_segment_duration,
        max_segments: settings.max_segments,
    };
    let hooks = captions::LayoutHooks::default();
    let mut shorts = Vec::with_capacity(result.clips.len());
    let mut snap_failures = 0usize;
    for clip in &result.clips {
        match snap_clip_boundaries(
            std::slice::from_ref(&clip.segments),
            transcript,
            profile,
            &hooks,
            &energy,
            &boundary,
        ) {
            Ok(mut snapped) => match snapped.pop() {
                Some(short) => shorts.push(short.segments),
                None => {
                    snap_failures = snap_failures.saturating_add(1);
                    shorts.push(clip.segments.clone());
                }
            },
            Err(error) => {
                snap_failures = snap_failures.saturating_add(1);
                notes.push(format!("snap refused: {error}"));
                shorts.push(clip.segments.clone());
            }
        }
    }
    Ok(ProviderRun {
        shorts,
        report,
        snap_failures,
    })
}

fn source_range(source: &Path, transcript: &Transcript, probe: bool) -> Result<SourceRange> {
    let end = transcript
        .segments
        .iter()
        .map(|segment| segment.range.end())
        .max()
        .unwrap_or(Tick::ZERO);
    let probed = if probe {
        media::probe(source)
            .ok()
            .and_then(|probe| probe.duration)
            .filter(|duration| *duration > end)
            .unwrap_or(end)
    } else {
        end
    };
    if probed <= Tick::ZERO {
        return Err(Error::Options("source duration is empty".into()));
    }
    Ok(SourceRange::new(Tick::ZERO, probed)?)
}

fn energy_windows(
    source: &Path,
    transcript: &Transcript,
) -> Result<Vec<media::SpeechEnergyWindow>> {
    let asset = media::SourceAsset::import(media::AssetId(1), source.to_path_buf())?;
    let range = source_range(source, transcript, true)?;
    Ok(media::analyze_speech_energy(
        &asset,
        range,
        Arc::new(std::sync::atomic::AtomicBool::new(false)),
    )?)
}

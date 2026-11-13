//! Runs one clip suggestion as the body of a registry job.
use super::ranking::{clip_input, snap_candidate, snap_settings};
use super::view::{candidate_view, cost_view, dropped_views, rejected_views};
use super::{
    ClipCandidate, RejectedClip, SnapSettings, SuggestionProvider, build_suggester,
    check_suggestion_credentials, choose_suggestion, fingerprint, upload_request,
};
use ai::text::clips::ClipSettings;
use ai::{CostEstimate, Money, Scope};
use captions::NfcText;
use engine::cache::CacheKey;
use engine::control::{
    ClipEnergy, ClipsResult, ClipsSuggestRequest, Error as EngineError, MAX_CLIPS_RESULT_BYTES,
    encode_clips,
};
use engine::jobs::{JobBytes, JobId, JobKind, JobRunner, JobSpec, ResourceClass};
use engine::providers::{Cost, approve_upload};
use engine::subtitles::CaptionAccess;
use engine::transcript::{Cancellation, Transcription, WORK_BYTES, transcribe};
use engine::{EditError, RuntimeError, Session};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Instant;
use time::{Rounding, SourceRange, Tick};

/// Most overlapping provider windows one suggestion may approve and run.
const MAX_WINDOWS: usize = 64;
const DEFAULT_COUNT: usize = 5;
const DEFAULT_MAX_SEGMENTS: usize = 4;
const DEFAULT_OVERLAP: f64 = 0.5;

/// What the first attempt approved. A retry reuses exactly these scopes and tokens, so the
/// human approval, the per-invocation cost ceiling and the uploaded payload never change.
struct Approved {
    scopes: Vec<Scope>,
    tokens: Vec<Option<ai::Consent>>,
    per_attempt: Cost,
    input_digest: blake3::Hash,
}

fn seconds(value: f64) -> Result<Tick, EngineError> {
    Ok(Tick::from_seconds_f64(value, Rounding::Nearest)?)
}

fn estimate_of(cost: &Cost) -> Result<CostEstimate, EngineError> {
    Ok(match cost {
        Cost::Free => CostEstimate::Free,
        Cost::UpperBound { currency, micros } => {
            CostEstimate::UpperBound(Money::new(currency, *micros)?)
        }
        Cost::Unknown => CostEstimate::Unknown,
    })
}

/// Start the suggestion job: the provider choice and its credential are checked here, so a
/// refusal costs no ASR and no upload; everything else runs inside the job.
pub(crate) fn submit_suggest(
    credentials: &engine::providers::Credentials,
    runner: &mut JobRunner,
    access: &CaptionAccess,
    session: &Session,
    request: ClipsSuggestRequest,
    input: CacheKey,
) -> Result<JobId, EngineError> {
    let provider: SuggestionProvider = choose_suggestion(
        request.provider.as_deref().unwrap_or("heuristic"),
        request.model.as_deref(),
    )?;
    check_suggestion_credentials(&provider, credentials)?;
    let asset = request.asset;
    let duration = asset
        .probe()
        .duration
        .filter(|value| *value > Tick::ZERO)
        .ok_or(EngineError::Edit(EditError::Unsupported(
            "media duration is unknown",
        )))?;
    let (providers, consent) = access.share()?;
    let cache = access.cache()?.clone();
    let suggester = std::sync::Arc::new(build_suggester(&provider, credentials)?);
    let language = request.source_language;
    let keywords = request.keywords;
    let min_duration = request.min_duration.map_or_else(|| seconds(20.0), Ok)?;
    let max_duration = request.max_duration.map_or_else(|| seconds(60.0), Ok)?;
    let min_segment_duration = request
        .min_segment_duration
        .map_or_else(|| seconds(2.0), Ok)?;
    let count = request.count.unwrap_or(DEFAULT_COUNT);
    let max_segments = request.max_segments.unwrap_or(DEFAULT_MAX_SEGMENTS);
    let overlap_tolerance = request.overlap_tolerance.unwrap_or(DEFAULT_OVERLAP);
    let snap = SnapSettings {
        frame_rate: None,
        tolerance: seconds(1.0)?,
        silence_rms: 0.01,
    };
    let class = if provider.is_remote() || providers.transcriber().config().is_remote() {
        ResourceClass::Network
    } else {
        ResourceClass::Cpu
    };
    let provider_id: String = provider.id().into();
    let provider_model: String = provider.model().into();
    let approved: Mutex<Option<Approved>> = Mutex::new(None);
    let attempts = AtomicU8::new(0);
    let started = Instant::now();
    let (id, _) = runner.submit_for_session(
        session,
        JobSpec {
            kind: JobKind::SuggestClips,
            class,
            input,
            bytes: JobBytes {
                compositor: WORK_BYTES,
                result: MAX_CLIPS_RESULT_BYTES as u64,
                ..Default::default()
            },
        },
        move |hooks| -> Result<Vec<u8>, EngineError> {
            hooks.check_cancelled()?;
            let ran = attempts.fetch_add(1, Ordering::AcqRel).saturating_add(1);
            let cancel = Cancellation::from_flag(hooks.media_cancel_flag());
            let source_language = match &language {
                Some(language) => language.clone(),
                None => providers.detect_language_with_hooks(&asset, duration, &cancel, hooks)?,
            };
            let Transcription {
                transcript,
                cues,
                cost: transcription_cost,
                checkpoints,
                ..
            } = transcribe(
                &asset,
                duration,
                &source_language,
                &providers,
                None,
                &cache,
                consent.as_ref(),
                hooks,
                &cancel,
                hooks,
            )?;
            hooks.check_cancelled()?;
            let settings = ClipSettings {
                count,
                min_duration,
                max_duration,
                max_segments,
                min_segment_duration,
                overlap_tolerance,
                language: source_language.clone(),
                keywords: keywords.clone(),
            };
            let clip_input = clip_input(&cues, &source_language, &settings)?;
            let windows = clip_input.windows()?;
            if windows.len() > MAX_WINDOWS {
                return Err(EngineError::Control(engine::control::ControlError::Limit));
            }
            let digest =
                blake3::hash(&serde_json::to_vec(&clip_input).map_err(|_| {
                    EngineError::Edit(EditError::Invalid("clip input serialization"))
                })?);
            let mut guard = approved.lock().map_err(|_| RuntimeError::Poisoned)?;
            if guard.is_none() {
                // Consent binds exact payloads: each window is approved once, before any
                // upload, and a retry reuses these approvals instead of asking again.
                let source_fingerprint = NfcText::from(fingerprint(&asset));
                let mut scopes = Vec::with_capacity(windows.len());
                let mut tokens = Vec::with_capacity(windows.len());
                let mut per_attempt = Cost::Free;
                for window in &windows {
                    let scope = Scope::clip_suggestion(
                        suggester.as_clip().config().clone(),
                        window,
                        source_fingerprint.clone(),
                    )?;
                    let estimate = suggester.as_clip().estimate(&scope);
                    per_attempt.add(&estimate);
                    let token = if scope.provider.is_remote() {
                        let disclosure = suggester
                            .disclosure(&scope)?
                            .ok_or(EngineError::Edit(EditError::Invalid("clip consent scope")))?;
                        Some(approve_upload(
                            consent.as_ref(),
                            &upload_request(&scope, &disclosure, &source_language)?,
                            &scope,
                            &estimate,
                        )?)
                    } else {
                        None
                    };
                    scopes.push(scope);
                    tokens.push(token);
                }
                *guard = Some(Approved {
                    scopes,
                    tokens,
                    per_attempt,
                    input_digest: digest,
                });
            }
            let approval = guard
                .as_ref()
                .ok_or(EngineError::Edit(EditError::Invalid("clip consent scope")))?;
            if approval.input_digest != digest {
                return Err(EngineError::Edit(EditError::Invalid(
                    "clip transcript changed between attempts",
                )));
            }
            let requests: Vec<_> = approval
                .scopes
                .iter()
                .zip(&approval.tokens)
                .map(|(scope, consent)| ai::Request {
                    scope,
                    consent: consent.as_ref(),
                })
                .collect();
            let suggested =
                ai::text::clips::suggest_clips(suggester.as_clip(), &clip_input, &requests, hooks)?;
            // Every attempted invocation is paid for, including transient failures.
            let mut suggestion_cost = Cost::Free;
            for _ in 0..ran {
                suggestion_cost.add(&estimate_of(&approval.per_attempt)?);
            }
            drop(guard);
            hooks.check_cancelled()?;
            let full = SourceRange::new(Tick::ZERO, duration)?;
            let energy = media::analyze_speech_energy(&asset, full, hooks.media_cancel_flag())?;
            let profile = captions::LanguageRegistry::standard()?
                .profile(&source_language)
                .clone();
            let boundaries = snap_settings(&asset, duration, &settings, &snap)?;
            let mut clips: Vec<ClipCandidate> = Vec::new();
            clips
                .try_reserve_exact(suggested.clips.len())
                .map_err(|_| EngineError::Control(engine::control::ControlError::Limit))?;
            let mut rejected = Vec::new();
            for suggestion in &suggested.clips {
                match snap_candidate(suggestion, &transcript, &profile, &energy, &boundaries) {
                    Ok(candidate) => clips.push(candidate),
                    Err(reason) => rejected.push(RejectedClip {
                        title: suggestion.proposal.title.clone(),
                        reason,
                    }),
                }
                // Each snapped short counts as it arrives; a refusal leaves the count where it was.
                hooks
                    .produced(u32::try_from(clips.len()).unwrap_or(u32::MAX))
                    .ok();
            }
            hooks.check_cancelled()?;
            let result = ClipsResult {
                asset: asset.id(),
                source_language,
                transcription_provider: providers.transcription().id().into(),
                transcription_model: providers.transcription().model().into(),
                provider: provider_id.clone(),
                model: provider_model.clone(),
                clips: clips.iter().map(candidate_view).collect(),
                rejected: rejected_views(&rejected),
                dropped: dropped_views(&suggested.dropped),
                cues,
                energy: energy
                    .iter()
                    .map(|window| ClipEnergy(window.range, window.rms as f32))
                    .collect(),
                transcript_segments: transcript.segments.len(),
                checkpoints,
                transcription_cost: cost_view(&transcription_cost),
                suggestion_cost: cost_view(&suggestion_cost),
                elapsed_seconds: started.elapsed().as_secs_f64(),
            };
            // Hours-long sources downsample their energy windows to fit the result limit.
            encode_clips(result)
        },
    )?;
    Ok(id)
}

//! Produces source-snapped clip candidates.
use super::{
    Arc, ArtifactCache, ArtifactKind, BoundaryFailure, CacheKey, Cancellation,
    ClipBoundarySettings, ClipCandidate, ClipInput, ClipRequest, ClipSegment, ClipSettings,
    ClipSuggester, ClipSuggestion, ClipSuggestionOutcome, ConsentPrompt, Cost, CostEstimate,
    Credentials, Cue, EditError, Error, FrameRate, Instant, JobBytes, JobKind, JobRunner, JobSpec,
    KeyInputs, LanguageTag, MAX_WIRE, MODE, Money, NfcText, Operation, Progress, PromptApproval,
    Providers, RejectedClip, ResourceClass, SampleRange, Scope, SnapSettings, SourceAsset,
    SourceRange, Stage, Tick, Transcription, build_session, build_suggester, engine_providers,
    snap_clip_boundaries, transcribe, upload_request, wait_for_job,
};
fn clip_input(
    cues: &[Cue],
    language: &LanguageTag,
    settings: &ClipSettings,
) -> Result<ClipInput, EditError> {
    let mut segments = Vec::new();
    segments
        .try_reserve_exact(cues.len())
        .map_err(|_| EditError::Limit)?;
    for cue in cues {
        let text = cue
            .texts
            .get(language)
            .ok_or(EditError::Invalid("clip transcript language"))?;
        segments.push(ClipSegment {
            id: cue.id.clone(),
            text: text.text.clone(),
            range: cue.range,
        });
    }
    let input = ClipInput {
        segments,
        settings: settings.clone(),
    };
    input.validate()?;
    Ok(input)
}

fn scope_for(
    provider: &dyn ClipSuggester,
    window: &ClipInput,
    fingerprint: &NfcText,
) -> Result<Scope, EditError> {
    Ok(Scope {
        provider: provider.config().clone(),
        operation: Operation::SuggestClips,
        payload_fingerprint: fingerprint.clone(),
        mode: MODE.into(),
        texts: vec![window.consent_text()?],
        source_language: window.settings.language.clone(),
        target_language: None,
        source_samples: SampleRange::new(0, 1_000)?,
        sample_rate: 1,
        max_bytes: MAX_WIRE,
    })
}

fn snap_settings(
    asset: &SourceAsset,
    duration: Tick,
    settings: &ClipSettings,
    snap: &SnapSettings,
) -> Result<ClipBoundarySettings, EditError> {
    let frame_rate = snap.frame_rate.or_else(|| {
        asset
            .probe()
            .streams
            .iter()
            .find(|stream| stream.kind == media::StreamKind::Video)
            .and_then(|stream| stream.frame_rate)
    });
    Ok(ClipBoundarySettings {
        source_range: SourceRange::new(Tick::ZERO, duration)?,
        frame_rate: frame_rate.unwrap_or(FrameRate::FPS_30),
        tolerance: snap.tolerance,
        silence_rms: snap.silence_rms,
        min_duration: settings.min_duration,
        max_duration: settings.max_duration,
        min_segment_duration: settings.min_segment_duration,
        max_segments: settings.max_segments,
    })
}

fn snap_candidate(
    suggestion: &ClipSuggestion,
    transcript: &captions::Transcript,
    profile: &captions::LanguageProfile,
    energy: &[media::SpeechEnergyWindow],
    settings: &ClipBoundarySettings,
) -> Result<ClipCandidate, BoundaryFailure> {
    let snapped = snap_clip_boundaries(
        std::slice::from_ref(&suggestion.segments),
        transcript,
        profile,
        energy,
        settings,
    )
    .map_err(|error| match error {
        Error::ClipBoundary { reason, .. } => reason,
        _ => BoundaryFailure::NoSafeFrame,
    })?;
    let Some(short) = snapped.into_iter().next() else {
        return Err(BoundaryFailure::Candidate);
    };
    Ok(ClipCandidate {
        proposal: suggestion.proposal.clone(),
        segments: short.segments,
        adjustments: short.adjustments,
        duration: short.duration,
        source: suggestion.source,
    })
}

pub(super) fn fingerprint(asset: &SourceAsset) -> String {
    format!(
        "clips:{}",
        blake3::Hash::from(asset.content_hash().0).to_hex()
    )
}

/// Transcribe, suggest, and snap one source file. No output file is written.
#[allow(clippy::too_many_arguments)]
pub fn suggest(
    request: &ClipRequest,
    credentials: &Credentials,
    providers: &Providers,
    runner: &mut JobRunner,
    cache: &mut ArtifactCache,
    consent: &dyn ConsentPrompt,
    progress: &dyn Progress,
    cancel: &Cancellation,
) -> Result<ClipSuggestionOutcome, Error> {
    let started = Instant::now();
    cancel.checkpoint()?;
    let asset =
        SourceAsset::import(media::AssetId(1), request.input.clone()).map_err(Error::kernel)?;
    let duration = asset
        .probe()
        .duration
        .filter(|value| *value > Tick::ZERO)
        .ok_or(EditError::Unsupported("media duration is unknown"))?;
    let source_language = match &request.source_language {
        Some(language) => language.clone(),
        None => providers.detect_language_for(&request.input, cancel)?,
    };
    let transcription = transcribe(
        &asset,
        duration,
        &source_language,
        providers,
        runner,
        cache,
        consent,
        progress,
        cancel,
    )?;
    let Transcription {
        transcript,
        cues,
        cost: transcription_cost,
        checkpoints,
        ..
    } = transcription;
    let input = clip_input(&cues, &source_language, &request.settings)?;
    if input.windows().map_err(Error::kernel)?.len() > 64 {
        return Err(EditError::Limit.into());
    }

    let provider = Arc::new(build_suggester(&request.provider, credentials)?);
    let source_fingerprint = NfcText::from(fingerprint(&asset));
    let windows = input.windows().map_err(Error::kernel)?;
    let mut scopes = Vec::with_capacity(windows.len());
    let mut requests = Vec::with_capacity(windows.len());
    for window in &windows {
        let scope = scope_for(provider.as_clip(), window, &source_fingerprint)?;
        if let Some(disclosure) = provider.disclosure(&scope)? {
            requests.push((
                scope.clone(),
                upload_request(&disclosure, &source_language)?,
            ));
        }
        scopes.push(scope);
    }
    let mut approval = PromptApproval {
        consent,
        requests,
        spent: Cost::Free,
        refusal: None,
    };
    // Approval binds exact payloads. Retries reuse only these approved scopes,
    // like transcription jobs; the runner owns their attempts and backoff.
    let tokens = scopes
        .iter()
        .map(|scope| {
            engine_providers::Approval::approve(
                &mut approval,
                scope,
                &provider.as_clip().estimate(scope),
                false,
            )
        })
        .collect::<Result<Vec<_>, _>>();
    if let Some(error) = approval.refusal.take() {
        return Err(error.into());
    }
    let tokens = tokens.map_err(Error::kernel)?;
    let settings_bytes = serde_json::to_vec(&(provider.as_clip().config(), &input))
        .map_err(|_| EditError::Invalid("clip input serialization"))?;
    let key = CacheKey::new(KeyInputs {
        kind: ArtifactKind::Transcript,
        source: asset.content_hash().0,
        provider: provider.as_clip().config().id().as_str(),
        model: provider.as_clip().config().model().as_str(),
        version: "clip-suggestion-job-v1",
        settings: &[],
        transcript: *blake3::hash(&settings_bytes).as_bytes(),
        cues: [0; 32],
        clip_range: [0; 32],
        translation: [0; 32],
    })?;
    progress.stage(Stage::SuggestClips);
    let class = if request.provider.is_remote() {
        ResourceClass::Network
    } else {
        ResourceClass::Cpu
    };
    let (id, token) = runner.submit(
        JobSpec {
            kind: JobKind::SuggestClips,
            class,
            input: key,
            bytes: JobBytes {
                compositor: 16 << 20,
                result: 8 << 20,
                ..Default::default()
            },
        },
        move |hooks| {
            let requests: Vec<_> = scopes
                .iter()
                .zip(&tokens)
                .map(|(scope, consent)| ai::Request {
                    scope,
                    consent: consent.as_ref(),
                })
                .collect();
            let result = ai::clips::suggest_clips(provider.as_clip(), &input, &requests, hooks)?;
            serde_json::to_vec(&result).map_err(|_| EditError::Invalid("clip result serialization"))
        },
    )?;
    let output = wait_for_job(runner, id, &token, progress, cancel)?;
    let suggested: ai::clips::ClipSuggestions = serde_json::from_slice(output.bytes())
        .map_err(|_| EditError::Invalid("clip result serialization"))?;
    // Account for every attempted invocation, including transient failures.
    let attempts = runner.snapshot(id).ok_or(EditError::StaleResult)?.attempts;
    let estimate = approval.spent.clone();
    for _ in 1..attempts {
        approval.spent.add(&match &estimate {
            Cost::Free => CostEstimate::Free,
            Cost::UpperBound { currency, micros } => {
                CostEstimate::UpperBound(Money::new(currency, *micros).map_err(Error::kernel)?)
            }
            Cost::Unknown => CostEstimate::Unknown,
        });
    }
    cancel.checkpoint()?;

    progress.stage(Stage::Snap);
    let energy = media::analyze_speech_energy(
        &asset,
        SourceRange::new(Tick::ZERO, duration).map_err(Error::kernel)?,
        cancel.media_flag(),
    )
    .map_err(Error::kernel)?;
    let profile = captions::LanguageRegistry::standard()
        .map_err(Error::kernel)?
        .profile(&source_language)
        .clone();
    let boundaries = snap_settings(&asset, duration, &request.settings, &request.snap)?;
    let mut clips = Vec::new();
    let mut rejected = Vec::new();
    clips
        .try_reserve_exact(suggested.clips.len())
        .map_err(|_| EditError::Limit)?;
    for suggestion in &suggested.clips {
        match snap_candidate(suggestion, &transcript, &profile, &energy, &boundaries) {
            Ok(candidate) => clips.push(candidate),
            Err(reason) => rejected.push(RejectedClip {
                title: suggestion.proposal.title.clone(),
                reason,
            }),
        }
    }
    let source_project = build_session(&asset, &cues, duration, true, None)?
        .project()
        .as_ref()
        .clone();
    cancel.checkpoint()?;
    Ok(ClipSuggestionOutcome {
        input: request.input.clone(),
        source_language,
        transcription_provider: providers.transcription().id().into(),
        transcription_model: providers.transcription().model().into(),
        provider: request.provider.id().into(),
        model: request.provider.model().into(),
        clips,
        dropped: suggested.dropped,
        rejected,
        transcript_segments: transcript.segments.len(),
        cues: cues.len(),
        checkpoints,
        transcription_cost,
        suggestion_cost: approval.spent,
        elapsed: started.elapsed(),
        asset,
        duration,
        source_cues: cues,
        source_project,
        transcript,
        energy,
        boundaries,
    })
}

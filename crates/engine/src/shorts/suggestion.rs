//! Clip suggestion workflow: transcript segments in, ranked shorts with snapped
//! source edges out. Export preparation reuses the 05 batch and the subtitle
//! translation path; the CLI only parses arguments and prints this result.
use crate::EditError;
use crate::cache::{ArtifactCache, ArtifactKind, CacheKey, KeyInputs};
use crate::clip_boundaries::{
    BoundaryFailure, ClipBoundarySettings, SegmentAdjustment, snap_clip_boundaries,
};
use crate::jobs::{JobBytes, JobKind, JobRunner, JobSpec, JobState, ResourceClass};
use crate::providers as engine_providers;
use crate::shorts::{ClipBatch, ClipBatchRequest, ClipPaths, ClipShort, MAX_BATCH_CLIPS};
use crate::subtitles::{
    self, Cancellation, CatalogOperation, ConsentPrompt, Cost, Progress, Providers, Stage,
    Transcription, UploadRequest,
};
use ai::clips::{
    ANTHROPIC_SETTINGS, AnthropicClipSuggester, ClipDisclosure, ClipInput, ClipProposal,
    ClipSegment, ClipSettings, ClipSource, ClipSourceSegment, ClipSuggester, ClipSuggestion,
    DroppedClip, HeuristicClipSuggester, MAX_WIRE, MODE, OPENAI_LUNA_MODEL, OPENAI_SETTINGS,
    OpenAiClipSuggester,
};
use ai::{Consent, CostEstimate, Location, Money, Operation, ProviderConfig, SampleRange, Scope};
use captions::{Cue, CueTrack, Format, LanguageTag, NfcText};
use media::SourceAsset;
use project::Project;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use time::{FrameRate, SourceRange, Tick};

/// One user-facing source of clip proposals. No implicit fallback exists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SuggestionProvider {
    Heuristic,
    OpenAi { model: String },
    Anthropic { model: String },
}
impl SuggestionProvider {
    pub fn id(&self) -> &'static str {
        match self {
            Self::Heuristic => "heuristic",
            Self::OpenAi { .. } => "openai",
            Self::Anthropic { .. } => "anthropic",
        }
    }
    pub fn model(&self) -> &str {
        match self {
            Self::Heuristic => "sentence-pause-keyword",
            Self::OpenAi { model } | Self::Anthropic { model } => model,
        }
    }
    pub fn is_remote(&self) -> bool {
        !matches!(self, Self::Heuristic)
    }
}

/// Offline picker data; constructing it never loads an adapter or calls a provider.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SuggestionOption {
    pub id: &'static str,
    pub label: &'static str,
    pub local: bool,
    pub models: Vec<&'static str>,
    pub policy_url: Option<&'static str>,
}
pub fn suggestion_options() -> Vec<SuggestionOption> {
    vec![
        SuggestionOption {
            id: "heuristic",
            label: "Local heuristic (no upload)",
            local: true,
            models: vec!["sentence-pause-keyword"],
            policy_url: None,
        },
        SuggestionOption {
            id: "openai",
            label: "OpenAI (remote)",
            local: false,
            models: vec![OPENAI_LUNA_MODEL, ai::openai::TRANSLATION_MODEL],
            policy_url: Some(OPENAI_POLICY_URL),
        },
        SuggestionOption {
            id: "anthropic",
            label: "Anthropic (remote)",
            local: false,
            models: vec![ai::anthropic::MODEL],
            policy_url: Some(ai::anthropic::POLICY_URL),
        },
    ]
}

/// Operation-specific provider data link shown before remote clip consent.
pub const OPENAI_POLICY_URL: &str = "https://developers.openai.com/api/docs/guides/your-data";

pub fn choose_suggestion(
    provider: &str,
    model: Option<&str>,
) -> Result<SuggestionProvider, EditError> {
    let option = suggestion_options()
        .into_iter()
        .find(|option| option.id == provider)
        .ok_or(EditError::Unsupported("unknown clip suggestion provider"))?;
    let model = model
        .or_else(|| option.models.first().copied())
        .ok_or(EditError::Unsupported("missing clip suggestion model"))?;
    if !option.models.contains(&model) {
        return Err(EditError::Unsupported(
            "unsupported model for clip suggestion provider",
        ));
    }
    match option.id {
        "heuristic" => Ok(SuggestionProvider::Heuristic),
        "openai" => Ok(SuggestionProvider::OpenAi {
            model: model.into(),
        }),
        "anthropic" => Ok(SuggestionProvider::Anthropic {
            model: model.into(),
        }),
        _ => Err(EditError::Unsupported("unknown clip suggestion provider")),
    }
}

/// Frame snapping inputs. `frame_rate` defaults to the source video rate.
#[derive(Clone, Copy, Debug)]
pub struct SnapSettings {
    pub frame_rate: Option<FrameRate>,
    pub tolerance: Tick,
    pub silence_rms: f64,
}

#[derive(Clone, Debug)]
pub struct ClipRequest {
    pub input: PathBuf,
    /// `None` asks the selected ASR provider for local language detection.
    pub source_language: Option<LanguageTag>,
    pub settings: ClipSettings,
    pub snap: SnapSettings,
    pub provider: SuggestionProvider,
}

/// One ranked short after independent per-edge snapping.
#[derive(Clone, Debug)]
pub struct ClipCandidate {
    pub proposal: ClipProposal,
    pub segments: Vec<ClipSourceSegment>,
    pub adjustments: Vec<SegmentAdjustment>,
    pub duration: Tick,
    pub source: ClipSource,
}

/// A suggestion that could not be snapped safely and was dropped from the result.
#[derive(Clone, Debug)]
pub struct RejectedClip {
    pub title: NfcText,
    pub reason: BoundaryFailure,
}

pub struct ClipSuggestionOutcome {
    pub input: PathBuf,
    pub source_language: LanguageTag,
    pub transcription_provider: String,
    pub transcription_model: String,
    pub provider: String,
    pub model: String,
    pub clips: Vec<ClipCandidate>,
    pub dropped: Vec<DroppedClip>,
    pub rejected: Vec<RejectedClip>,
    pub transcript_segments: usize,
    pub cues: usize,
    pub checkpoints: usize,
    pub transcription_cost: Cost,
    pub suggestion_cost: Cost,
    pub elapsed: Duration,
    /// Retained source context for desktop review; no ASR or energy re-decoding.
    pub asset: SourceAsset,
    duration: Tick,
    pub source_cues: Vec<Cue>,
    pub source_project: Project,
    pub transcript: captions::Transcript,
    pub energy: Vec<media::SpeechEnergyWindow>,
    pub boundaries: ClipBoundarySettings,
}

enum BuiltSuggester {
    Heuristic(HeuristicClipSuggester),
    OpenAi(OpenAiClipSuggester),
    Anthropic(AnthropicClipSuggester),
}
impl BuiltSuggester {
    fn as_clip(&self) -> &dyn ClipSuggester {
        match self {
            Self::Heuristic(value) => value,
            Self::OpenAi(value) => value,
            Self::Anthropic(value) => value,
        }
    }
    fn disclosure(
        &self,
        window: &ClipInput,
        scope: &Scope,
    ) -> Result<Option<ClipDisclosure>, EditError> {
        match self {
            Self::Heuristic(_) => Ok(None),
            Self::OpenAi(value) => Ok(Some(value.disclosure(window, scope)?)),
            Self::Anthropic(value) => Ok(Some(value.disclosure(window, scope)?)),
        }
    }
}

fn build_suggester(
    provider: &SuggestionProvider,
    credentials: &subtitles::Credentials,
) -> Result<BuiltSuggester, EditError> {
    match provider {
        SuggestionProvider::Heuristic => {
            Ok(BuiltSuggester::Heuristic(HeuristicClipSuggester::new()?))
        }
        SuggestionProvider::OpenAi { model } => {
            let config = ProviderConfig::new(
                "openai".into(),
                model.as_str().into(),
                OPENAI_SETTINGS.into(),
                Location::Remote {
                    backend: ai::openai::BACKEND.into(),
                    base_url: ai::openai::BASE_URL.into(),
                    auth_ref: "OPENAI_API_KEY".into(),
                },
            )?;
            Ok(BuiltSuggester::OpenAi(OpenAiClipSuggester::new(
                config,
                credentials.openai()?.clone(),
                Arc::new(ai::transport::UreqTransport::new()),
            )?))
        }
        SuggestionProvider::Anthropic { model } => {
            let config = ProviderConfig::new(
                "anthropic".into(),
                model.as_str().into(),
                ANTHROPIC_SETTINGS.into(),
                Location::Remote {
                    backend: ai::anthropic::BACKEND.into(),
                    base_url: ai::anthropic::BASE_URL.into(),
                    auth_ref: "ANTHROPIC_API_KEY".into(),
                },
            )?;
            Ok(BuiltSuggester::Anthropic(AnthropicClipSuggester::new(
                config,
                credentials.anthropic()?.clone(),
                Arc::new(ai::transport::UreqTransport::new()),
            )?))
        }
    }
}

fn upload_request(
    disclosure: &ClipDisclosure,
    language: &LanguageTag,
) -> Result<UploadRequest, EditError> {
    Ok(UploadRequest {
        operation: CatalogOperation::SuggestClips,
        provider: disclosure.provider.into(),
        model: disclosure.model.as_str().into(),
        languages: language.as_str().into(),
        audio_seconds: 0.0,
        upload_bytes: u64::try_from(disclosure.payload.as_str().len())
            .map_err(|_| EditError::Limit)?,
        cost: Cost::from(&disclosure.estimate),
        terms: vec![
            ("endpoint", disclosure.endpoint.into()),
            (
                "operation",
                "clip suggestions: transcript segments, no audio".into(),
            ),
            ("retention_training_policy", disclosure.policy_url.into()),
        ],
    })
}

struct PromptApproval<'a> {
    consent: &'a dyn ConsentPrompt,
    requests: Vec<(Scope, UploadRequest)>,
    spent: Cost,
    refusal: Option<EditError>,
}
impl engine_providers::Approval for PromptApproval<'_> {
    fn approve(
        &mut self,
        scope: &Scope,
        estimate: &CostEstimate,
        _fallback: bool,
    ) -> Result<Option<Consent>, ai::Error> {
        self.spent.add(estimate);
        if !scope.provider.is_remote() {
            return Ok(None);
        }
        let request = self
            .requests
            .iter()
            .find(|(planned, _)| planned == scope)
            .map(|(_, request)| request)
            .ok_or(ai::Error::Invalid("clip consent scope"))?;
        if let Err(error) = self.consent.approve(request) {
            self.refusal = Some(error);
            return Err(ai::Error::ConsentRequired);
        }
        let money = match estimate {
            CostEstimate::Free => Money::new("USD", 0)?,
            CostEstimate::UpperBound(money) => money.clone(),
            CostEstimate::Unknown => return Err(ai::Error::Cost),
        };
        Ok(Some(Consent::grant(scope.clone(), money)?))
    }
}

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
        &captions::LayoutHooks::default(),
        energy,
        settings,
    )
    .map_err(|error| match error {
        EditError::ClipBoundary { reason, .. } => reason,
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

fn fingerprint(asset: &SourceAsset) -> String {
    format!("clips:{}", hex(&asset.content_hash().0))
}

/// Transcribe, suggest, and snap one source file. No output file is written.
#[allow(clippy::too_many_arguments)]
pub fn suggest(
    request: &ClipRequest,
    credentials: &subtitles::Credentials,
    providers: &Providers,
    runner: &mut JobRunner,
    cache: &mut ArtifactCache,
    consent: &dyn ConsentPrompt,
    progress: &dyn Progress,
    cancel: &Cancellation,
) -> Result<ClipSuggestionOutcome, EditError> {
    let started = Instant::now();
    cancel.checkpoint()?;
    let asset = SourceAsset::import(media::AssetId(1), request.input.clone())?;
    let duration = asset
        .probe()
        .duration
        .filter(|value| *value > Tick::ZERO)
        .ok_or(EditError::Unsupported("media duration is unknown"))?;
    let source_language = match &request.source_language {
        Some(language) => language.clone(),
        None => providers.detect_language_for(&request.input, cancel)?,
    };
    let transcription = subtitles::transcribe(
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
    if input.windows()?.len() > 64 {
        return Err(EditError::Limit);
    }

    let provider = Arc::new(build_suggester(&request.provider, credentials)?);
    let source_fingerprint = NfcText::from(fingerprint(&asset));
    let windows = input.windows()?;
    let mut scopes = Vec::with_capacity(windows.len());
    let mut requests = Vec::with_capacity(windows.len());
    for window in &windows {
        let scope = scope_for(provider.as_clip(), window, &source_fingerprint)?;
        if let Some(disclosure) = provider.disclosure(window, &scope)? {
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
        return Err(error);
    }
    let tokens = tokens?;
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
    let output = subtitles::wait_for_job(runner, id, &token, progress, cancel)?;
    let suggested: ai::clips::ClipSuggestions = serde_json::from_slice(output.bytes())
        .map_err(|_| EditError::Invalid("clip result serialization"))?;
    // Account for every attempted invocation, including transient failures.
    let attempts = runner.snapshot(id).ok_or(EditError::StaleResult)?.attempts;
    let estimate = approval.spent.clone();
    for _ in 1..attempts {
        approval.spent.add(&match &estimate {
            Cost::Free => CostEstimate::Free,
            Cost::UpperBound { currency, micros } => {
                CostEstimate::UpperBound(Money::new(currency, *micros)?)
            }
            Cost::Unknown => CostEstimate::Unknown,
        });
    }
    cancel.checkpoint()?;

    progress.stage(Stage::Snap);
    let energy = media::analyze_speech_energy(
        &asset,
        SourceRange::new(Tick::ZERO, duration)?,
        cancel.media_flag(),
    )?;
    let profile = providers.language_profile(&source_language)?;
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
    let source_project = subtitles::build_session(&asset, &cues, duration, true, None)?
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

/// Refuse a remote suggestion choice without its credential before ASR work starts.
pub fn check_suggestion_credentials(
    provider: &SuggestionProvider,
    credentials: &subtitles::Credentials,
) -> Result<(), EditError> {
    match provider {
        SuggestionProvider::Heuristic => Ok(()),
        SuggestionProvider::OpenAi { .. } => credentials.openai().map(|_| ()),
        SuggestionProvider::Anthropic { .. } => credentials.anthropic().map(|_| ()),
    }
}

#[derive(Clone, Debug)]
pub struct ClipExportRequest {
    /// Existing output directory.
    pub directory: PathBuf,
    /// Zero-based candidate indices in candidate order; 1..=1000 distinct values.
    pub select: Vec<usize>,
    /// None/empty for no captions, or one/two distinct languages to burn.
    pub languages: Vec<LanguageTag>,
    pub formats: Vec<Format>,
    pub overwrite: bool,
    pub allow_degraded: bool,
    pub max_output_bytes: u64,
}

/// Prepared clips keep the requested candidate index for each batch slot.
pub struct ClipExportPrep {
    pub batch: ClipBatch,
    pub selected: Vec<usize>,
    pub translation_cost: Cost,
}

/// Translate (when needed) and prepare every selected short for export.
#[allow(clippy::too_many_arguments)]
pub fn prepare_export(
    outcome: &ClipSuggestionOutcome,
    request: ClipExportRequest,
    providers: &Providers,
    runner: &mut JobRunner,
    consent: &dyn ConsentPrompt,
    progress: &dyn Progress,
    cancel: &Cancellation,
) -> Result<ClipExportPrep, EditError> {
    if request.select.is_empty() || request.select.len() > MAX_BATCH_CLIPS {
        return Err(EditError::Invalid("clip selection size"));
    }
    let mut seen = std::collections::BTreeSet::new();
    for index in &request.select {
        if *index >= outcome.clips.len() || !seen.insert(*index) {
            return Err(EditError::Invalid("clip selection index"));
        }
    }
    let mut track = CueTrack {
        cues: outcome.source_cues.clone(),
    };
    let mut translation_cost = Cost::Free;
    for language in &request.languages {
        if language == &outcome.source_language {
            continue;
        }
        let translated = subtitles::translate_track(
            &outcome.asset,
            outcome.duration,
            &outcome.source_cues,
            providers,
            runner,
            consent,
            progress,
            cancel,
            &outcome.source_language,
            language,
            &NfcText::from(fingerprint(&outcome.asset)),
            &mut translation_cost,
        )?;
        for (target, source) in track.cues.iter_mut().zip(translated.cues) {
            if target.id != source.id {
                return Err(EditError::Invalid("translated clip cues"));
            }
            for (tag, text) in source.texts {
                if tag != outcome.source_language {
                    target.texts.insert(tag, text);
                }
            }
        }
    }
    let mut shorts = Vec::with_capacity(request.select.len());
    for index in &request.select {
        let candidate = outcome
            .clips
            .get(*index)
            .ok_or(EditError::Invalid("clip selection index"))?;
        shorts.push(ClipShort::new(
            candidate.proposal.title.clone(),
            candidate.segments.clone(),
        ));
    }
    let mut batch = ClipBatchRequest::new(outcome.asset.id(), shorts, request.directory);
    batch.cues = track;
    batch.languages = request.languages;
    batch.subtitle_formats = request.formats;
    batch.overwrite = request.overwrite;
    batch.allow_degraded = request.allow_degraded;
    batch.max_output_bytes = request.max_output_bytes;
    let batch = ClipBatch::prepare(&outcome.source_project, batch)?;
    Ok(ClipExportPrep {
        batch,
        selected: request.select,
        translation_cost,
    })
}

/// One published short in batch-slot order.
#[derive(Clone, Debug)]
pub struct ClipExportPaths {
    pub candidate: usize,
    pub paths: ClipPaths,
}

#[derive(Clone, Debug)]
pub struct ClipExportOutcome {
    pub exports: Vec<ClipExportPaths>,
    pub translation_cost: Cost,
}

/// Export with bounded parallelism, refilling slots as shorts complete. A failed
/// admission or job cancels and drains admitted siblings before returning.
/// Already published complete files remain.
#[allow(clippy::too_many_arguments)]
pub fn export(
    outcome: &ClipSuggestionOutcome,
    request: ClipExportRequest,
    providers: &Providers,
    runner: &mut JobRunner,
    consent: &dyn ConsentPrompt,
    progress: &dyn Progress,
    cancel: &Cancellation,
) -> Result<ClipExportOutcome, EditError> {
    let prep = prepare_export(
        outcome, request, providers, runner, consent, progress, cancel,
    )?;
    let mut batch = prep.batch;
    let capacity = runner.batch_capacity(ResourceClass::GpuEncoder);
    let mut active = Vec::with_capacity(capacity);
    let mut next = 0;
    let result = (|| {
        while next < prep.selected.len() || !active.is_empty() {
            cancel.checkpoint()?;
            while active.len() < capacity && next < prep.selected.len() {
                cancel.checkpoint()?;
                match batch.submit_one(runner, next) {
                    Ok(_) => {
                        active.push(next);
                        next += 1;
                    }
                    // Pump the caller's other work before retrying admission.
                    Err(EditError::QueueFull) => break,
                    Err(error) => return Err(error),
                }
            }
            cancel.checkpoint()?;
            runner.tick()?;
            for event in runner.drain_events() {
                if active
                    .iter()
                    .any(|slot| batch.job_id(*slot) == Some(event.id))
                {
                    progress.job(&event);
                }
            }
            let mut index = 0;
            while index < active.len() {
                let slot = *active.get(index).ok_or(EditError::StaleResult)?;
                let snapshot = batch.snapshot(runner, slot).ok_or(EditError::StaleResult)?;
                match snapshot.state {
                    JobState::Completed => {
                        let _ = runner
                            .take_output(snapshot.id)
                            .ok_or(EditError::StaleResult)?;
                        active.swap_remove(index);
                    }
                    JobState::Failed => {
                        return Err(match snapshot.error {
                            Some(error) => EditError::JobFailure(error),
                            None => EditError::Invalid("clip job failed without diagnostics"),
                        });
                    }
                    JobState::Cancelled => return Err(EditError::Cancelled),
                    _ => index += 1,
                }
            }
            if !active.is_empty() || next < prep.selected.len() {
                std::thread::sleep(Duration::from_millis(5));
            }
        }
        Ok(())
    })();
    if let Err(error) = result {
        batch.cancel_all();
        // Keep the batch alive until every admitted worker has reached a terminal
        // state. Never shut down unrelated work in the caller's runner.
        loop {
            let running = (0..next).any(|slot| {
                batch.snapshot(runner, slot).is_some_and(|snapshot| {
                    !matches!(
                        snapshot.state,
                        JobState::Completed | JobState::Failed | JobState::Cancelled
                    )
                })
            });
            if !running {
                break;
            }
            runner.tick()?;
            std::thread::sleep(Duration::from_millis(5));
        }
        for slot in 0..next {
            if let Some(id) = batch.job_id(slot) {
                let _ = runner.take_output(id);
            }
        }
        return Err(error);
    }
    let mut exports = Vec::with_capacity(prep.selected.len());
    for (slot, candidate) in prep.selected.iter().enumerate() {
        exports.push(ClipExportPaths {
            candidate: *candidate,
            paths: batch
                .paths(slot)
                .ok_or(EditError::Invalid("clip paths"))?
                .clone(),
        });
    }
    Ok(ClipExportOutcome {
        exports,
        translation_cost: prep.translation_cost,
    })
}

fn hex(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(text, "{byte:02x}");
    }
    text
}

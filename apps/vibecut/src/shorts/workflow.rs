//! Composes source-to-shorts generation for the app.
use super::{GenerationRequest, Generator, ReviewSource, TaskEvents};
use engine::Result;
use engine::{
    ByteBudget,
    cache::ArtifactCache,
    jobs::{JobRunner, RunnerConfig, SystemClock},
    subtitles::{self, Cancellation},
};
use shorts::clip_suggest::{self, ClipRequest, SnapSettings};
use std::{sync::Arc, time::Duration};
use time::{TICKS_PER_SECOND, Tick};

pub struct LocalVideoWorkflow;
impl Generator for LocalVideoWorkflow {
    fn run(
        &self,
        request: GenerationRequest,
        budget: ByteBudget,
        events: &TaskEvents,
        cancel: &Cancellation,
    ) -> Result<ReviewSource> {
        let language = subtitles::parse_language(&request.language)?;
        let transcription = subtitles::choose_transcription(
            &request.transcription_provider,
            Some(&request.transcription_model),
        )?;
        let provider = clip_suggest::choose_suggestion(
            &request.suggestion_provider,
            Some(&request.suggestion_model),
        )?;
        let credentials = subtitles::Credentials::new(
            std::env::var("OPENAI_API_KEY").ok(),
            std::env::var("ELEVENLABS_API_KEY").ok(),
            std::env::var("ANTHROPIC_API_KEY").ok(),
        )?;
        clip_suggest::check_suggestion_credentials(&provider, &credentials)?;
        let root = subtitles::default_cache_root()?;
        let (mut cache, _) = ArtifactCache::open(&root, budget.clone(), 128)?;
        let runtime = subtitles::Runtime {
            gipformer_runner: subtitles::local_asr_runner(),
            ..Default::default()
        };
        let providers =
            subtitles::Providers::open(transcription, None, &mut cache, &credentials, &runtime)?;
        let mut runner = JobRunner::new(
            RunnerConfig {
                concurrency: [2, 1, 1],
                queue: 8,
                retained: 16,
                network_attempts: 1,
                backoff: Duration::from_millis(10),
                progress_interval: Duration::from_millis(20),
            },
            budget,
            Arc::new(SystemClock::default()),
        )?;
        let settings = engine::ClipSettings {
            count: request.count,
            min_duration: Tick(i64::from(request.min_seconds) * TICKS_PER_SECOND),
            max_duration: Tick(i64::from(request.max_seconds) * TICKS_PER_SECOND),
            max_segments: request.max_segments,
            min_segment_duration: Tick(TICKS_PER_SECOND),
            overlap_tolerance: 0.2,
            language: language.clone(),
            keywords: Vec::new(),
        };
        let request = ClipRequest {
            input: request.path,
            source_language: Some(language),
            settings,
            snap: SnapSettings {
                frame_rate: None,
                tolerance: Tick(TICKS_PER_SECOND),
                silence_rms: 0.01,
            },
            provider,
        };
        let outcome = clip_suggest::suggest(
            &request,
            &credentials,
            &providers,
            &mut runner,
            &mut cache,
            events,
            events,
            cancel,
        )?;
        let candidates = outcome.clips;
        Ok(ReviewSource {
            project: outcome.source_project,
            asset: outcome.asset.id(),
            transcript: outcome.transcript,
            cues: captions::CueTrack {
                cues: outcome.source_cues,
            },
            energy: outcome.energy,
            boundaries: outcome.boundaries,
            candidates,
        })
    }
}

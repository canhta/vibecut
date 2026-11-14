//! Configures the CLI engine runtime.
use crate::args::CliError;
use engine::ByteBudget;
use engine::jobs::{JobRunner, RunnerConfig, SystemClock};
use std::sync::Arc;
use std::time::Duration;

pub(crate) fn cli_runtime() -> Result<engine::providers::Runtime, CliError> {
    let mut runtime = engine::providers::Runtime::default();
    if let Ok(value) = std::env::var("VIBECUT_ASR_THREADS") {
        let threads = value.parse::<u8>().map_err(|_| {
            CliError::Usage("VIBECUT_ASR_THREADS must be an integer from 1 to 32".into())
        })?;
        if !(1..=32).contains(&threads) {
            return Err(CliError::Usage(
                "VIBECUT_ASR_THREADS must be an integer from 1 to 32".into(),
            ));
        }
        runtime.threads = threads;
    }
    if std::env::var("VIBECUT_ASR_GPU").is_ok_and(|value| value == "0") {
        runtime.use_gpu = false;
    }
    runtime.gipformer_runner = engine::providers::local_asr_runner();
    Ok(runtime)
}

pub(crate) fn cli_runner(budget: ByteBudget) -> Result<JobRunner, CliError> {
    cli_runner_sized(budget, 8)
}

/// A runner whose queue and retained results hold `slots` jobs, for a command that
/// submits a whole selection at once, such as one export job per short.
pub(crate) fn cli_runner_sized(budget: ByteBudget, slots: usize) -> Result<JobRunner, CliError> {
    Ok(JobRunner::new(
        RunnerConfig {
            concurrency: [1, 1, 2],
            queue: slots,
            retained: slots,
            network_attempts: 3,
            backoff: Duration::from_millis(250),
            progress_interval: Duration::from_millis(50),
        },
        budget,
        Arc::new(SystemClock::default()),
    )?)
}

/// The upload prompt one session lends to every remote workflow.
pub(crate) type Consent = Arc<dyn engine::providers::ConsentPrompt + Send + Sync>;

/// Optional local speech resources. Their typed refusal is lent to control while
/// edits and inspection remain available; this never downloads models. `consent` is the
/// session's one upload prompt, asked by every remote workflow it lends (clips, narration).
/// Narration needs no caption resource, so it is lent even when those are unavailable, with
/// the speech adapters opened from the process credentials.
pub(crate) fn caption_access(
    budget: ByteBudget,
    consent: Consent,
) -> engine::subtitles::CaptionAccess {
    use engine::{
        cache::{ArtifactCache, CacheRoot},
        dubbing::CredentialSpeech,
        providers::{Credentials, Providers},
    };
    let open = |consent: Consent| -> engine::subtitles::Result<engine::subtitles::CaptionAccess> {
        let root = CacheRoot::discover()?.models();
        let (mut cache, _) = ArtifactCache::open(&root, budget, 128)?;
        let providers = Providers::open(
            engine::providers::choose_transcription("whisper.cpp", None)?,
            None,
            &mut cache,
            &Credentials::from_env(),
            &engine::providers::Runtime::default(),
        )?;
        Ok(engine::subtitles::CaptionAccess::new(
            Arc::new(providers),
            consent,
            Arc::new(std::sync::Mutex::new(cache)),
        )
        .with_clips(Arc::new(shorts::ClipJobs::new(Credentials::from_env()))))
    };
    let access = open(consent.clone()).unwrap_or_else(|error| {
        engine::subtitles::CaptionAccess::unavailable(error).with_consent(consent)
    });
    lend_narration(
        access,
        Arc::new(CredentialSpeech::new(Credentials::from_env())),
    )
}

/// Lend `dub.generate` its narrator over `opener`. It asks the consent prompt of `access`, so
/// narration has no second approval path.
pub(crate) fn lend_narration(
    access: engine::subtitles::CaptionAccess,
    opener: Arc<dyn engine::dubbing::SpeechOpener>,
) -> engine::subtitles::CaptionAccess {
    access.with_dubbing(Arc::new(engine::dubbing::Narrator::new(opener)))
}

/// The local face detector lent to `reframe.generate` and clip-export layouts. A root that
/// already holds the models cache (through `CaptionAccess::cache`) passes it as `shared`;
/// otherwise the process opens the models cache once here. Either way every load reads
/// through that one cache, and no weights are downloaded.
pub(crate) fn vision_access(
    budget: ByteBudget,
    shared: Option<Arc<std::sync::Mutex<engine::cache::ArtifactCache>>>,
) -> engine::control::VisionAccess {
    shorts::installed_faces(model_lookup(budget, shared))
}

/// The face detector of [`vision_access`] and the local person segmenter lent to
/// `footage.analyze {kind: person_matte}`, both reading through one models-cache lookup; no
/// weights are downloaded.
pub(crate) fn vision_and_segmentation(
    budget: ByteBudget,
    shared: Option<Arc<std::sync::Mutex<engine::cache::ArtifactCache>>>,
) -> (
    engine::control::VisionAccess,
    engine::control::SegmentationAccess,
) {
    let lookup = model_lookup(budget, shared);
    (
        shorts::installed_faces(lookup.clone()),
        shorts::installed_person_matte(lookup),
    )
}

/// The verified models-cache lookup over `shared`, or over the cache this process opens once.
fn model_lookup(
    budget: ByteBudget,
    shared: Option<Arc<std::sync::Mutex<engine::cache::ArtifactCache>>>,
) -> shorts::ModelLookup {
    use ai::foundation::models::Model;
    use engine::cache::ModelLease;
    match shared {
        Some(cache) => Arc::new(
            move |model: &Model,
                  hooks: &dyn ai::ExecutionHooks|
                  -> engine::control::Result<Option<ModelLease>> {
                Ok(cache
                    .lock()
                    .map_err(|_| engine::RuntimeError::Poisoned)?
                    .model(model, hooks)?)
            },
        ),
        None => {
            let opened = engine::models::ModelTransfers::open_native(budget, 128)
                .ok()
                .map(|(transfers, _)| transfers);
            Arc::new(
                move |model: &Model,
                      hooks: &dyn ai::ExecutionHooks|
                      -> engine::control::Result<Option<ModelLease>> {
                    match &opened {
                        Some(transfers) => Ok(transfers.installed(model, hooks)?),
                        None => Err(engine::RuntimeError::CacheBusy.into()),
                    }
                },
            )
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]
    use super::lend_narration;
    use crate::args::CliError;
    use crate::output::CliConsent;
    use crate::session::job::run_job_with;
    use crate::session::registry::{Lent, session};
    use ai::speech::{
        SpeechAudio, SpeechCapabilities, SpeechFormat, SpeechLanguage, SpeechOutput, SpeechPolicy,
        SpeechRequest, SpeechSynthesizer, SpeechUsage, SpeechVoice, SynthesisConsent,
        SynthesisEstimate, VoiceCategory, prepare_synthesis,
    };
    use ai::{Error, ExecutionHooks, Location, Money, ProviderConfig, Support};
    use engine::dubbing::SpeechOpener;
    use engine::jobs::JobState;
    use engine::subtitles::{CaptionAccess, LanguageTag};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    const RATE: u32 = 24_000;

    /// A scripted speech provider: one second of a quiet tone, no network.
    struct Voice {
        config: ProviderConfig,
        calls: AtomicUsize,
    }
    impl Voice {
        fn new() -> Self {
            Self {
                config: ProviderConfig::new(
                    "scripted".into(),
                    "model-a".into(),
                    "settings".into(),
                    Location::Remote {
                        backend: "fixture".into(),
                        base_url: "https://example.com/v1".into(),
                        auth_ref: "secret-reference".into(),
                    },
                )
                .unwrap(),
                calls: AtomicUsize::new(0),
            }
        }
        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
        fn caps(&self) -> SpeechCapabilities {
            SpeechCapabilities {
                provider: self.config.clone(),
                fingerprint: "scripted:model-a".into(),
                languages: vec![SpeechLanguage {
                    language: LanguageTag::parse("en").unwrap(),
                    script: "Latn".into(),
                }],
                voices: vec![SpeechVoice {
                    id: "alloy".into(),
                    category: VoiceCategory::BuiltIn,
                }],
                outputs: vec![output()],
                max_characters: Some(500),
                streaming: Support::Unsupported,
                max_concurrency: Some(1),
                requests_per_minute: None,
                cost_model: "bounded USD invocation".into(),
                policy: SpeechPolicy {
                    policy_url: "https://example.com/policy".into(),
                    retention: "kept for 30 days".into(),
                    training: "not used for training".into(),
                    voice_disclosure: "synthetic voice".into(),
                },
            }
        }
    }
    fn output() -> SpeechOutput {
        SpeechOutput {
            format: SpeechFormat::Pcm16,
            sample_rate: RATE,
        }
    }
    impl SpeechSynthesizer for Voice {
        fn config(&self) -> &ProviderConfig {
            &self.config
        }
        fn capabilities(&self, _: &SpeechRequest) -> SpeechCapabilities {
            self.caps()
        }
        fn estimate(&self, _: &SpeechRequest) -> SynthesisEstimate {
            SynthesisEstimate::UpperBound(Money::new("USD", 2_500).unwrap())
        }
        fn synthesize(
            &self,
            request: &SpeechRequest,
            consent: &SynthesisConsent,
            _: &dyn ExecutionHooks,
        ) -> Result<SpeechAudio, Error> {
            prepare_synthesis(self, request, consent)?;
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(SpeechAudio {
                samples: vec![0.25; RATE as usize],
                sample_rate: RATE,
                duration_micros: 1_000_000,
                fingerprint: self.caps().fingerprint,
                usage: SpeechUsage {
                    billable_characters: Some(request.characters()),
                    input_tokens: None,
                    output_tokens: None,
                    actual_cost: Some(Money::new("USD", 1_000).unwrap()),
                },
                transcript: Some(request.text().clone()),
            })
        }
    }

    struct Opener(Arc<Voice>);
    impl SpeechOpener for Opener {
        fn open(
            &self,
            provider: &str,
            model: &str,
            _: Option<u16>,
        ) -> Result<(Arc<dyn SpeechSynthesizer>, SpeechOutput), Error> {
            if (provider, model) != ("scripted", "model-a") {
                return Err(Error::Unsupported);
            }
            let voice: Arc<dyn SpeechSynthesizer> = self.0.clone();
            Ok((voice, output()))
        }
    }

    /// Run `dub.generate` through the registry with the narrator lent over the scripted
    /// provider and the CLI consent prompt, as the control session lends it.
    fn dub_generate(consent: CliConsent, voice: &Arc<Voice>) -> Result<JobState, CliError> {
        let access = lend_narration(
            CaptionAccess::consent_only(Arc::new(consent)),
            Arc::new(Opener(voice.clone())),
        );
        let mut project = session()?;
        let mut runner = super::cli_runner(engine::ByteBudget::new(512 << 20).unwrap())?;
        let view = run_job_with(
            &mut project,
            &mut runner,
            Lent {
                captions: Some(&access),
                ..Lent::default()
            },
            "dub.generate",
            serde_json::json!({
                "provider": "scripted",
                "model": "model-a",
                "voice": "alloy",
                "language": "en",
                "text": "Welcome back.",
                "cue_duration": time::TICKS_PER_SECOND,
            }),
        );
        runner.shutdown();
        Ok(view?.state)
    }

    #[test]
    fn dub_generate_runs_over_the_lent_narrator_when_the_cli_consent_approves() {
        let voice = Arc::new(Voice::new());
        let state = dub_generate(CliConsent::new(true, false), &voice).unwrap();
        assert_eq!(state, JobState::Completed);
        assert_eq!(voice.calls(), 1);
    }

    #[test]
    fn dub_generate_is_refused_before_any_request_without_upload_consent() {
        let voice = Arc::new(Voice::new());
        let Err(error) = dub_generate(CliConsent::new(false, false), &voice) else {
            panic!("narration without --yes-upload must be refused");
        };
        assert_eq!(error.code(), ("rejected", 5));
        assert_eq!(voice.calls(), 0);
    }
}

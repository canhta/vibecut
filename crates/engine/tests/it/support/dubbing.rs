//! A scripted speech provider and narration assets for the dubbing tests. No network.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "public-interface conformance assertions"
)]
use ai::speech::{
    SpeechAudio, SpeechCapabilities, SpeechFormat, SpeechLanguage, SpeechOutput, SpeechPolicy,
    SpeechRequest, SpeechSynthesizer, SpeechUsage, SpeechVoice, SynthesisConsent,
    SynthesisEstimate, VoiceCategory, prepare_synthesis,
};
use ai::{Error, ExecutionHooks, Location, Money, ProviderConfig, Support};
use captions::LanguageTag;
use engine::dubbing::SpeechOpener;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

pub(crate) const RATE: u32 = 24_000;
pub(crate) const POLICY: &str = "https://example.com/policy";

/// How the scripted provider answers one synthesis.
#[derive(Clone, Debug)]
pub(crate) enum Script {
    /// This many samples of a quiet tone.
    Samples(usize),
    Fail(fn() -> Error),
    /// A transcript that differs from the request.
    WrongTranscript,
}

pub(crate) struct Voice {
    config: ProviderConfig,
    estimate: SynthesisEstimate,
    script: Script,
    pub(crate) calls: AtomicUsize,
    languages: Vec<&'static str>,
    voices: Vec<&'static str>,
}
pub(crate) fn money(micros: u64) -> Money {
    Money::new("USD", micros).unwrap()
}
pub(crate) fn config(provider: &str, model: &str) -> ProviderConfig {
    ProviderConfig::new(
        provider.into(),
        model.into(),
        "settings".into(),
        Location::Remote {
            backend: "fixture".into(),
            base_url: "https://example.com/v1".into(),
            auth_ref: "secret-reference".into(),
        },
    )
    .unwrap()
}
impl Voice {
    pub(crate) fn new(provider: &str, model: &str, script: Script) -> Self {
        Self {
            config: config(provider, model),
            estimate: SynthesisEstimate::UpperBound(money(2_500)),
            script,
            calls: AtomicUsize::new(0),
            languages: vec!["vi", "en"],
            voices: vec!["alloy", "nova"],
        }
    }
    pub(crate) fn priced(mut self, estimate: SynthesisEstimate) -> Self {
        self.estimate = estimate;
        self
    }
    pub(crate) fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
    fn caps(&self) -> SpeechCapabilities {
        SpeechCapabilities {
            provider: self.config.clone(),
            fingerprint: format!(
                "{}:{}",
                self.config.id().as_str(),
                self.config.model().as_str()
            )
            .as_str()
            .into(),
            languages: self
                .languages
                .iter()
                .map(|tag| SpeechLanguage {
                    language: LanguageTag::parse(tag).unwrap(),
                    script: "Latn".into(),
                })
                .collect(),
            voices: self
                .voices
                .iter()
                .map(|id| SpeechVoice {
                    id: (*id).into(),
                    category: VoiceCategory::BuiltIn,
                })
                .collect(),
            outputs: vec![output()],
            max_characters: Some(500),
            streaming: Support::Unsupported,
            max_concurrency: Some(1),
            requests_per_minute: None,
            cost_model: "bounded USD invocation".into(),
            policy: SpeechPolicy {
                policy_url: POLICY.into(),
                retention: "kept for 30 days".into(),
                training: "not used for training".into(),
                voice_disclosure: "synthetic voice".into(),
            },
        }
    }
}
pub(crate) fn output() -> SpeechOutput {
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
        self.estimate.clone()
    }
    fn synthesize(
        &self,
        request: &SpeechRequest,
        consent: &SynthesisConsent,
        _: &dyn ExecutionHooks,
    ) -> Result<SpeechAudio, Error> {
        prepare_synthesis(self, request, consent)?;
        self.calls.fetch_add(1, Ordering::SeqCst);
        let count = match &self.script {
            Script::Fail(error) => return Err(error()),
            Script::Samples(count) => *count,
            Script::WrongTranscript => RATE as usize,
        };
        let transcript = if matches!(self.script, Script::WrongTranscript) {
            "different text".into()
        } else {
            request.text().clone()
        };
        Ok(SpeechAudio {
            samples: vec![0.25; count],
            sample_rate: RATE,
            duration_micros: (count as u64 * 1_000_000).div_ceil(u64::from(RATE)),
            fingerprint: self.caps().fingerprint,
            usage: SpeechUsage {
                billable_characters: Some(request.characters()),
                input_tokens: None,
                output_tokens: None,
                actual_cost: Some(money(1_000)),
            },
            transcript: Some(transcript),
        })
    }
}

/// Opens only the pairs it was given; anything else is `Unsupported`, never a substitute.
pub(crate) struct Opener {
    pub(crate) voices: Vec<(String, String, Arc<Voice>)>,
    pub(crate) attempts: Mutex<Vec<(String, String, Option<u16>)>>,
}
impl Opener {
    pub(crate) fn new(voices: Vec<(&str, &str, Arc<Voice>)>) -> Self {
        Self {
            voices: voices
                .into_iter()
                .map(|(provider, model, voice)| (provider.to_owned(), model.to_owned(), voice))
                .collect(),
            attempts: Mutex::new(Vec::new()),
        }
    }
}
impl SpeechOpener for Opener {
    fn open(
        &self,
        provider: &str,
        model: &str,
        speed: Option<u16>,
    ) -> Result<(Arc<dyn SpeechSynthesizer>, SpeechOutput), Error> {
        self.attempts
            .lock()
            .unwrap()
            .push((provider.to_owned(), model.to_owned(), speed));
        self.voices
            .iter()
            .find(|(candidate, name, _)| candidate == provider && name == model)
            .map(|(_, _, voice)| {
                let voice: Arc<dyn SpeechSynthesizer> = voice.clone();
                (voice, output())
            })
            .ok_or(Error::Unsupported)
    }
}

/// The narration asset a take becomes: mono 24 kHz audio of `duration` ticks.
pub(crate) fn asset(id: u64, duration: time::Tick) -> media::SourceAsset {
    testkit::source::asset(
        id,
        std::env::temp_dir().join(format!("vibecut-narration-{id}.wav")),
    )
    .format("wav")
    .duration(duration)
    .content_hash(media::ContentHash::from_bytes(
        format!("narration {id}").as_bytes(),
    ))
    .stream(
        testkit::source::audio()
            .codec("pcm_s16le")
            .time_base(media::TimeBase::new(1, 24000).unwrap())
            .duration(duration)
            .build(),
    )
    .build()
}

/// Provenance for a placed take, with the default ducking.
pub(crate) fn narration(cue: time::Tick) -> project::Narration {
    serde_json::from_value(serde_json::json!({
        "provider": "scripted",
        "model": "model-a",
        "voice": "alloy",
        "language": "en",
        "script": "Latn",
        "provider_speed_thousandths": null,
        "fingerprint": "scripted:model-a",
        "text_sha256": "a".repeat(64),
        "characters": 13,
        "consent": {
            "policy_url": POLICY,
            "retention": "kept for 30 days",
            "training": "not used for training",
            "voice_disclosure": "synthetic voice",
            "upper_bound": {"currency": "USD", "micros": 2500}
        },
        "cue_duration": cue.0,
        "ducking": {"enabled": true, "depth_tenths_db": 120, "attack_ms": 120, "release_ms": 300}
    }))
    .unwrap()
}

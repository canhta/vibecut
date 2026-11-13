//! Prepares, discloses, approves and runs one synthesis against the chosen provider.
use super::take::DubTake;
use crate::providers::{CatalogOperation, ConsentPrompt, Cost, Result, UploadRequest};
use ai::speech::{
    SpeechCapabilities, SpeechLanguage, SpeechOutput, SpeechRequest, SpeechSynthesizer,
    SynthesisConsent, SynthesisEstimate,
};
use captions::{LanguageTag, NfcText};
use project::{Narration, NarrationConsent, NarrationCost, NarrationDucking};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use time::Tick;

/// Largest retained take: about 100 seconds of 24 kHz mono PCM16 as JSON.
pub const MAX_TAKE_BYTES: u64 = 32 * 1024 * 1024;
/// The provider output ceiling for one narration request.
const MAX_TAKE_MICROS: u64 = 120_000_000;
const PREVIEW_CHARACTERS: usize = 120;

/// What `dub.generate` asks for. Provider, model and voice are the user's explicit choice;
/// nothing falls back to another.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub provider: String,
    pub model: String,
    pub voice: String,
    pub language: LanguageTag,
    /// The exact text that is sent to the provider.
    pub text: NfcText,
    /// The output time the cue allows; the fit tiers are measured against it.
    pub cue_duration: Tick,
    /// ElevenLabs `voice_settings.speed` in thousandths, shown in consent.
    pub provider_speed_thousandths: Option<u16>,
}

/// Builds the synthesizer for one explicit provider and model with the user's credential.
/// An unknown pair is `ai::Error::Unsupported`; the opener never picks a substitute.
pub trait SpeechOpener: Send + Sync {
    fn open(
        &self,
        provider: &str,
        model: &str,
        provider_speed_thousandths: Option<u16>,
    ) -> std::result::Result<(Arc<dyn SpeechSynthesizer>, SpeechOutput), ai::Error>;
}

/// Everything the consent sheet shows before any text leaves the machine.
#[derive(Clone, Debug, PartialEq)]
pub struct Disclosure {
    pub provider: String,
    pub model: String,
    pub voice: String,
    pub language: String,
    pub script: String,
    /// Extended grapheme clusters of the exact text.
    pub characters: u64,
    pub preview: String,
    pub text_sha256: String,
    pub provider_speed_thousandths: Option<u16>,
    pub policy_url: String,
    pub retention: String,
    pub training: String,
    pub voice_disclosure: String,
    pub cost: Cost,
    pub fingerprint: String,
}

/// A validated request with its capabilities, quote and disclosure, before approval.
pub struct Prepared {
    synth: Arc<dyn SpeechSynthesizer>,
    request: SpeechRequest,
    capabilities: SpeechCapabilities,
    estimate: SynthesisEstimate,
    cue_duration: Tick,
    provider_speed_thousandths: Option<u16>,
    upload: UploadRequest,
    disclosure: Disclosure,
}

/// The explicit narration workflow: one provider per run, one approval per call.
pub struct Narrator {
    opener: Arc<dyn SpeechOpener>,
}

fn script_of(language: &LanguageTag) -> std::result::Result<&'static str, ai::Error> {
    match language.as_str() {
        "vi" | "en" => Ok("Latn"),
        "zh" | "zh-Hans" => Ok("Hans"),
        "zh-Hant" => Ok("Hant"),
        _ => Err(ai::Error::UnsupportedLanguage),
    }
}
fn hex(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
fn cost_of(estimate: &SynthesisEstimate) -> Cost {
    match estimate {
        SynthesisEstimate::Free => Cost::Free,
        SynthesisEstimate::Exact(money) | SynthesisEstimate::UpperBound(money) => {
            Cost::UpperBound {
                currency: money.currency().into(),
                micros: money.micros(),
            }
        }
        SynthesisEstimate::Unknown => Cost::Unknown,
    }
}

impl Narrator {
    pub fn new(opener: Arc<dyn SpeechOpener>) -> Self {
        Self { opener }
    }

    /// Validate the request against the chosen provider's capabilities and quote it. No
    /// network IO. An unsupported language or voice is refused here, before consent.
    pub fn prepare(&self, request: &Request) -> Result<Prepared> {
        if request.cue_duration <= Tick::ZERO {
            return Err(ai::Error::Invalid("cue duration").into());
        }
        let script = script_of(&request.language)?;
        let (synth, output) = self.opener.open(
            &request.provider,
            &request.model,
            request.provider_speed_thousandths,
        )?;
        let speech = SpeechRequest::new(
            synth.config().clone(),
            request.text.clone(),
            NfcText::from(request.voice.as_str()),
            SpeechLanguage {
                language: request.language.clone(),
                script: NfcText::from(script),
            },
            output,
            MAX_TAKE_MICROS,
        )?;
        let capabilities = synth.capabilities(&speech);
        capabilities.check(&speech)?;
        let estimate = synth.estimate(&speech);
        let text = request.text.as_str();
        let mut preview: String = text.chars().take(PREVIEW_CHARACTERS).collect();
        if text.chars().count() > PREVIEW_CHARACTERS {
            preview.push('…');
        }
        let disclosure = Disclosure {
            provider: synth.config().id().as_str().into(),
            model: synth.config().model().as_str().into(),
            voice: request.voice.clone(),
            language: request.language.as_str().into(),
            script: script.into(),
            characters: speech.characters(),
            preview,
            text_sha256: hex(&speech.text_sha256()),
            provider_speed_thousandths: request.provider_speed_thousandths,
            policy_url: capabilities.policy.policy_url.as_str().into(),
            retention: capabilities.policy.retention.as_str().into(),
            training: capabilities.policy.training.as_str().into(),
            voice_disclosure: capabilities.policy.voice_disclosure.as_str().into(),
            cost: cost_of(&estimate),
            fingerprint: capabilities.fingerprint.as_str().into(),
        };
        let speed = request.provider_speed_thousandths.map_or_else(
            || "default".to_owned(),
            |speed| format!("{}x", f64::from(speed) / 1000.0),
        );
        let upload = UploadRequest {
            config: synth.config().clone(),
            operation: CatalogOperation::SynthesizeSpeech,
            provider: disclosure.provider.clone(),
            model: disclosure.model.clone(),
            languages: format!("{}-{}", disclosure.language, disclosure.script),
            audio_seconds: 0.0,
            upload_bytes: text.len() as u64,
            cost: disclosure.cost.clone(),
            terms: vec![
                ("policy", disclosure.policy_url.clone()),
                ("voice", disclosure.voice.clone()),
                ("characters", disclosure.characters.to_string()),
                ("text", disclosure.preview.clone()),
                ("text_sha256", disclosure.text_sha256.clone()),
                ("speed", speed),
                ("retention", disclosure.retention.clone()),
                ("training", disclosure.training.clone()),
                ("voice_disclosure", disclosure.voice_disclosure.clone()),
            ],
        };
        Ok(Prepared {
            synth,
            request: speech,
            capabilities,
            estimate,
            cue_duration: request.cue_duration,
            provider_speed_thousandths: request.provider_speed_thousandths,
            upload,
            disclosure,
        })
    }
}

impl Prepared {
    /// What the consent sheet shows.
    pub fn disclosure(&self) -> &Disclosure {
        &self.disclosure
    }
    /// The shared upload request the existing consent prompt is asked to approve.
    pub fn upload(&self) -> &UploadRequest {
        &self.upload
    }

    /// Ask the prompt, then bind its answer to this exact request: a changed quote, text,
    /// voice or settings needs a fresh approval. Unknown pricing asks its own question.
    pub fn approve(&self, prompt: &dyn ConsentPrompt) -> Result<SynthesisConsent> {
        match &self.estimate {
            SynthesisEstimate::Unknown => {
                prompt.approve_unknown_price(&self.upload)?;
                let acknowledgement = ai::UnknownPriceAcknowledgement::grant(
                    self.request.provider().clone(),
                    ai::Operation::Synthesize,
                );
                Ok(SynthesisConsent::grant_unknown_price(
                    &self.request,
                    &self.capabilities,
                    &acknowledgement,
                )?)
            }
            SynthesisEstimate::Free => {
                prompt.approve(&self.upload)?;
                Ok(SynthesisConsent::grant(
                    &self.request,
                    &self.capabilities,
                    self.estimate.clone(),
                    ai::Money::new("USD", 0)?,
                )?)
            }
            SynthesisEstimate::Exact(money) | SynthesisEstimate::UpperBound(money) => {
                prompt.approve(&self.upload)?;
                Ok(SynthesisConsent::grant(
                    &self.request,
                    &self.capabilities,
                    self.estimate.clone(),
                    money.clone(),
                )?)
            }
        }
    }

    /// Run the single attempt. Cancellation, a provider failure, a rejected transcript or a
    /// cost beyond the approved bound discards everything: no partial take exists.
    pub fn synthesize(
        &self,
        consent: &SynthesisConsent,
        hooks: &dyn ai::ExecutionHooks,
    ) -> std::result::Result<DubTake, ai::Error> {
        let audio = ai::speech::synthesize(self.synth.as_ref(), &self.request, consent, hooks)?;
        let pcm = audio
            .samples
            .iter()
            .map(|sample| (sample.clamp(-1.0, 1.0) * f32::from(i16::MAX)).round() as i16)
            .collect();
        let upper_bound = match &self.estimate {
            SynthesisEstimate::Free => Some(NarrationCost {
                currency: NfcText::from("USD"),
                micros: 0,
            }),
            SynthesisEstimate::Exact(money) | SynthesisEstimate::UpperBound(money) => {
                Some(NarrationCost {
                    currency: NfcText::from(money.currency()),
                    micros: money.micros(),
                })
            }
            SynthesisEstimate::Unknown => None,
        };
        let policy = &self.capabilities.policy;
        Ok(DubTake {
            sample_rate: audio.sample_rate,
            pcm,
            narration: Narration {
                provider: NfcText::from(self.disclosure.provider.as_str()),
                model: NfcText::from(self.disclosure.model.as_str()),
                voice: NfcText::from(self.disclosure.voice.as_str()),
                language: NfcText::from(self.disclosure.language.as_str()),
                script: NfcText::from(self.disclosure.script.as_str()),
                provider_speed_thousandths: self.provider_speed_thousandths,
                fingerprint: audio.fingerprint,
                text_sha256: NfcText::from(self.disclosure.text_sha256.as_str()),
                characters: self.disclosure.characters,
                consent: NarrationConsent {
                    policy_url: policy.policy_url.clone(),
                    retention: policy.retention.clone(),
                    training: policy.training.clone(),
                    voice_disclosure: policy.voice_disclosure.clone(),
                    upper_bound,
                },
                cue_duration: self.cue_duration,
                ducking: NarrationDucking::DEFAULT,
            },
        })
    }
}

/// Cancellation for a take run outside a job: the dialog sets it, the provider seam polls it.
#[derive(Clone, Debug, Default)]
pub struct CancelFlag(Arc<AtomicBool>);
impl CancelFlag {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
}
impl ai::ExecutionHooks for CancelFlag {
    fn cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
    fn stage(&self, _: ai::Stage) {}
}

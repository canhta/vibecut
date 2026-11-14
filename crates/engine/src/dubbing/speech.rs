//! Opens the speech adapter of the provider and model the user chose, with their own key.
use super::SpeechOpener;
use crate::providers::Credentials;
use ai::providers::elevenlabs::speech::{
    ElevenLabsSpeech, ElevenLabsSpeechSettings, LoggingMode, SAMPLE_RATE as ELEVENLABS_RATE,
    SpeechSpeed,
};
use ai::providers::openai::{OpenAiSpeechSynthesizer, SPEECH_SAMPLE_RATE, SPEECH_VOICES};
use ai::speech::{SpeechFormat, SpeechOutput, SpeechSynthesizer, SpeechVoice, VoiceCategory};
use captions::NfcText;
use std::sync::Arc;
use transport::{HttpTransport, UreqTransport};

/// The ElevenLabs premade (stock) voices offered by the picker, id and label. Cloned and
/// professional-clone voices are never listed. Each id is `premade` in a free account's
/// `/v1/voices` and synthesized there on 2026-11-14; older stock ids (Rachel, Josh) became
/// library voices that a free plan is refused (HTTP 402 `paid_plan_required`).
pub const ELEVENLABS_VOICES: [(&str, &str); 5] = [
    ("EXAVITQu4vr4xnSDxMaL", "Sarah"),
    ("pNInz6obpgDQGcFmaJgB", "Adam"),
    ("JBFqnCBsd6RMkjVDRZzb", "George"),
    ("Xb7hH8MSUJpSbSDYk0k2", "Alice"),
    ("nPczCjzI2devNBz1zQrb", "Brian"),
];

/// One provider the narration panel can choose, offline: no adapter is built and no key is read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpeechOption {
    pub id: &'static str,
    pub label: &'static str,
    /// Shipped model ids, the default first. The user may type another id.
    pub models: Vec<String>,
    /// Voice ids with a display label (the id itself for the built-in OpenAI voices).
    pub voices: Vec<(&'static str, &'static str)>,
    /// Whether the provider offers a `voice_settings.speed`.
    pub speed: bool,
}

/// The speech providers in the order the panel lists them.
pub fn speech_options() -> Vec<SpeechOption> {
    vec![
        SpeechOption {
            id: "openai",
            label: "OpenAI",
            models: vec![ai::defaults::model("openai", ai::Operation::Synthesize).into()],
            voices: SPEECH_VOICES.iter().map(|voice| (*voice, *voice)).collect(),
            speed: false,
        },
        SpeechOption {
            id: "elevenlabs",
            label: "ElevenLabs",
            models: vec![
                ai::defaults::model("elevenlabs", ai::Operation::Synthesize).into(),
                "eleven_multilingual_v2".into(),
            ],
            voices: ELEVENLABS_VOICES.to_vec(),
            speed: true,
        },
    ]
}

/// Builds the narration adapters from the user's credentials. The transport is injected so a
/// test never reaches a network; the shipped one is the bounded HTTPS client.
pub struct CredentialSpeech {
    credentials: Credentials,
    transport: Arc<dyn HttpTransport>,
}

impl CredentialSpeech {
    pub fn new(credentials: Credentials) -> Self {
        Self::with_transport(credentials, Arc::new(UreqTransport::new()))
    }
    pub fn with_transport(credentials: Credentials, transport: Arc<dyn HttpTransport>) -> Self {
        Self {
            credentials,
            transport,
        }
    }
}

/// A missing or malformed key is an authentication failure; AI refusals keep their own cause.
fn key_error(error: crate::providers::Error) -> ai::Error {
    match error {
        crate::providers::Error::Ai(error) => error,
        _ => ai::Error::Auth,
    }
}

impl SpeechOpener for CredentialSpeech {
    fn open(
        &self,
        provider: &str,
        model: &str,
        provider_speed_thousandths: Option<u16>,
    ) -> Result<(Arc<dyn SpeechSynthesizer>, SpeechOutput), ai::Error> {
        match provider {
            "openai" => {
                // OpenAI offers no speed setting; asking for one is refused, not dropped.
                if provider_speed_thousandths.is_some() {
                    return Err(ai::Error::Unsupported);
                }
                let synthesizer = OpenAiSpeechSynthesizer::new(
                    OpenAiSpeechSynthesizer::provider_config(model)?,
                    self.credentials.openai().map_err(key_error)?,
                    self.transport.clone(),
                )?;
                Ok((
                    Arc::new(synthesizer),
                    SpeechOutput {
                        format: SpeechFormat::WavPcm16,
                        sample_rate: SPEECH_SAMPLE_RATE,
                    },
                ))
            }
            "elevenlabs" => {
                let settings = ElevenLabsSpeechSettings {
                    speed: provider_speed_thousandths
                        .map(|speed| SpeechSpeed::new(f64::from(speed) / 1000.0))
                        .transpose()?,
                    logging: LoggingMode::Default,
                };
                let voices = ELEVENLABS_VOICES
                    .iter()
                    .map(|(id, _)| SpeechVoice {
                        id: NfcText::from(*id),
                        category: VoiceCategory::Stock,
                    })
                    .collect();
                let synthesizer = ElevenLabsSpeech::new(
                    ElevenLabsSpeech::provider_config(model, &settings)?,
                    self.credentials.elevenlabs().map_err(key_error)?,
                    self.transport.clone(),
                    settings,
                    voices,
                )?;
                Ok((
                    Arc::new(synthesizer),
                    SpeechOutput {
                        format: SpeechFormat::Pcm16,
                        sample_rate: ELEVENLABS_RATE,
                    },
                ))
            }
            _ => Err(ai::Error::Unsupported),
        }
    }
}

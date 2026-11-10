//! Single-attempt OpenAI `gpt-audio-1.5` speech synthesis over Chat Completions.
//! Consent, cost, cancellation and validation stay in `speech`; this adapter owns
//! the exact wire schema, built-in voice gate and bounded WAV decoding.
use crate::remote::{self, UploadDisclosure};
use crate::speech::{
    SpeechAudio, SpeechCapabilities, SpeechFormat, SpeechLanguage, SpeechOutput, SpeechPolicy,
    SpeechRequest, SpeechSynthesizer, SpeechUsage, SpeechVoice, SynthesisConsent,
    SynthesisEstimate, VoiceCategory, prepare_synthesis,
};
use crate::transport::{ApiKey, HttpRequest, HttpResponse, HttpTransport};
use crate::{Error, ExecutionHooks, Money, Operation, ProviderConfig, Support};
use base64::Engine as _;
use captions::{LanguageTag, NfcText};
use serde::Deserialize;
use serde_json::json;
use std::fmt;
use std::sync::Arc;

/// The only admitted OpenAI audio model. Deprecated Speech API models are refused.
pub const SPEECH_MODEL: &str = "gpt-audio-1.5";
const SPEECH_BACKEND: &str = "openai";
pub const SPEECH_ENDPOINT: &str = "https://api.openai.com/v1/chat/completions";
/// Observed live output: 24 kHz mono 16-bit PCM WAV.
pub const SPEECH_SAMPLE_RATE: u32 = 24_000;
/// Settings identity for composition-owned cache keys and provider configs.
const SPEECH_SETTINGS_FINGERPRINT: &str = "openai-speech-v1;model=gpt-audio-1.5;format=wav;pcm16;rate=24000;store=false;instructions=verbatim-v1;normalization=nfc";
/// Built-in voices from the OpenAI TTS guide. Custom voice IDs are voice cloning.
pub const SPEECH_VOICES: [&str; 13] = [
    "alloy", "ash", "ballad", "coral", "echo", "fable", "nova", "onyx", "sage", "shimmer", "verse",
    "marin", "cedar",
];
/// Exactly the observed language/script pairs; any other pair is refused before IO.
pub const SPEECH_LANGUAGES: [(&str, &str); 3] = [("vi", "Latn"), ("en", "Latn"), ("zh", "Hans")];
/// Chat Completions audio responses are base64 in JSON; audio is much larger than
/// transcript text, so this adapter asks the shared transport for a speech cap.
pub const MAX_SPEECH_RESPONSE_BYTES: usize = 48 * 1024 * 1024;
/// Wire guard covering worst-case JSON escaping of the contract's 64 KiB text.
pub const MAX_SPEECH_REQUEST_BYTES: usize = 512 * 1024;
/// `gpt-audio-1.5` documented maximum completion tokens.
pub const MAX_OUTPUT_TOKENS: u64 = 16_384;
/// Observed assistant audio rate: one token per 50 ms.
pub const AUDIO_TOKENS_PER_SECOND: u64 = 20;
const INPUT_TOKEN_HEADROOM: u64 = 1_024;
const COMPLETION_TOKEN_HEADROOM: u64 = 64;
/// Published list prices in integer micro-USD per million tokens (2026-11-04).
const PRICE_INPUT_MICROS_PER_MILLION: u64 = 2_500_000;
const PRICE_AUDIO_INPUT_MICROS_PER_MILLION: u64 = 32_000_000;
const PRICE_AUDIO_OUTPUT_MICROS_PER_MILLION: u64 = 64_000_000;
const PRICE_TEXT_OUTPUT_MICROS_PER_MILLION: u64 = 10_000_000;
const POLICY_URL: &str = "https://developers.openai.com/api/docs/guides/your-data";
const INSTRUCTIONS: &str = "You are a text-to-speech engine. Read the user's text exactly as written, in its language, with natural pacing. Do not add, omit, translate, or explain anything.";

/// Consent-gated OpenAI audio output over an injected blocking transport.
pub struct OpenAiSpeechSynthesizer {
    config: ProviderConfig,
    key: ApiKey,
    transport: Arc<dyn HttpTransport>,
    languages: Vec<SpeechLanguage>,
    voices: Vec<SpeechVoice>,
}

impl OpenAiSpeechSynthesizer {
    /// The one admitted configuration for this model id.
    pub fn provider_config(model: &str) -> Result<ProviderConfig, Error> {
        remote::provider_config(
            &[SPEECH_MODEL],
            model,
            SPEECH_SETTINGS_FINGERPRINT,
            SPEECH_BACKEND,
            super::BASE_URL,
            "openai",
            "OPENAI_API_KEY",
        )
    }

    /// Admits only `gpt-audio-1.5`, the `openai` backend and the official HTTPS base
    /// URL. The key is resolved by composition, never here.
    pub fn new(
        config: ProviderConfig,
        key: ApiKey,
        transport: Arc<dyn HttpTransport>,
    ) -> Result<Self, Error> {
        remote::admit(
            &config,
            &[SPEECH_MODEL],
            Some(SPEECH_SETTINGS_FINGERPRINT),
            SPEECH_BACKEND,
            super::BASE_URL,
            Some("OpenAI speech requires remote config"),
        )?;
        let languages = SPEECH_LANGUAGES
            .iter()
            .map(|(tag, script)| {
                Ok(SpeechLanguage {
                    language: LanguageTag::parse(tag).map_err(Error::Captions)?,
                    script: NfcText::from(*script),
                })
            })
            .collect::<Result<Vec<_>, Error>>()?;
        let voices = SPEECH_VOICES
            .iter()
            .map(|voice| SpeechVoice {
                id: NfcText::from(*voice),
                category: VoiceCategory::BuiltIn,
            })
            .collect();
        Ok(Self {
            config,
            key,
            transport,
            languages,
            voices,
        })
    }
}

impl fmt::Debug for OpenAiSpeechSynthesizer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OpenAiSpeechSynthesizer")
            .field("config", &self.config)
            .field("key", &self.key)
            .field("transport", &"[transport]")
            .field("languages", &self.languages)
            .field("voices", &self.voices)
            .finish()
    }
}

impl SpeechSynthesizer for OpenAiSpeechSynthesizer {
    fn config(&self) -> &ProviderConfig {
        &self.config
    }

    fn capabilities(&self, request: &SpeechRequest) -> SpeechCapabilities {
        SpeechCapabilities {
            provider: self.config.clone(),
            fingerprint: capability_fingerprint(request),
            languages: self.languages.clone(),
            voices: self.voices.clone(),
            outputs: vec![SpeechOutput {
                format: SpeechFormat::WavPcm16,
                sample_rate: SPEECH_SAMPLE_RATE,
            }],
            // Unknown limits stay None; the contract's 64 KiB text byte ceiling and
            // the output-token bound refuse oversized requests before IO.
            max_characters: None,
            streaming: Support::Unsupported,
            max_concurrency: None,
            requests_per_minute: None,
            cost_model: NfcText::from(
                "USD upper bound: input text at $2.50/1M tokens plus audio output at $64/1M tokens (20 audio tokens/s)",
            ),
            policy: SpeechPolicy {
                policy_url: NfcText::from(POLICY_URL),
                retention: NfcText::from(
                    "API data is not used to train models by default; abuse-monitoring logs are retained up to 30 days; Chat Completions audio application state can persist up to 1 hour; Zero Data Retention applies only when the organization is approved. Retention is not zero by default.",
                ),
                training: NfcText::from(
                    "API inputs and outputs are not used to train models without explicit consent.",
                ),
                voice_disclosure: NfcText::from(
                    "The output voice is AI-generated; disclose this to viewers.",
                ),
            },
        }
    }

    fn estimate(&self, request: &SpeechRequest) -> SynthesisEstimate {
        let Some(max_completion) = max_completion_tokens(request) else {
            return SynthesisEstimate::Unknown;
        };
        // A byte-level BPE token represents at least one UTF-8 byte, so text bytes
        // are a conservative input-token upper bound; graphemes are not.
        let Some(text_bytes) = u64::try_from(request.text().as_str().len()).ok() else {
            return SynthesisEstimate::Unknown;
        };
        let Some(input_tokens) = text_bytes.checked_add(INPUT_TOKEN_HEADROOM) else {
            return SynthesisEstimate::Unknown;
        };
        let Some(input) = price(input_tokens, PRICE_INPUT_MICROS_PER_MILLION) else {
            return SynthesisEstimate::Unknown;
        };
        // Price every completion token at the higher audio rate. This covers text
        // output, reasoning tokens and any unreported completion detail.
        let Some(output) = price(max_completion, PRICE_AUDIO_OUTPUT_MICROS_PER_MILLION) else {
            return SynthesisEstimate::Unknown;
        };
        let Some(total) = input.checked_add(output) else {
            return SynthesisEstimate::Unknown;
        };
        Money::new("USD", total).map_or(SynthesisEstimate::Unknown, SynthesisEstimate::UpperBound)
    }

    fn disclosure(&self, request: &SpeechRequest) -> Result<Option<UploadDisclosure>, Error> {
        self.capabilities(request).check(request)?;
        Ok(Some(remote::disclosure(remote::Disclosure {
            operation: Operation::Synthesize,
            provider: "OpenAI",
            model: self.config.model(),
            languages: NfcText::from(format!(
                "{}/{}",
                request.language().language.as_str(),
                request.language().script.as_str()
            )),
            audio_micros: request.max_duration_micros(),
            upload_bytes: u64::try_from(request.text().as_str().len())
                .map_err(|_| Error::Payload)?,
            estimate: remote::cost_ceiling(self.estimate(request)),
            links: &[("policy", POLICY_URL)],
        })))
    }

    fn synthesize(
        &self,
        request: &SpeechRequest,
        consent: &SynthesisConsent,
        hooks: &dyn ExecutionHooks,
    ) -> Result<SpeechAudio, Error> {
        prepare_synthesis(self, request, consent)?;
        let max_completion = max_completion_tokens(request).ok_or(Error::Payload)?;
        if max_completion > MAX_OUTPUT_TOKENS {
            return Err(Error::Payload);
        }
        let body = build_body(request, max_completion)?;
        check_cancelled(hooks)?;
        let response = self.transport.send_bounded(
            HttpRequest {
                method: "POST",
                url: SPEECH_ENDPOINT.into(),
                headers: vec![
                    (
                        "Authorization".into(),
                        format!("Bearer {}", self.key.expose()),
                    ),
                    ("Content-Type".into(), "application/json".into()),
                    ("Accept".into(), "application/json".into()),
                ],
                body,
            },
            MAX_SPEECH_RESPONSE_BYTES,
        )?;
        check_cancelled(hooks)?;
        let audio = parse_speech_response(&response, request)?;
        check_cancelled(hooks)?;
        Ok(audio)
    }
}

fn check_cancelled(hooks: &dyn ExecutionHooks) -> Result<(), Error> {
    if hooks.cancelled() {
        Err(Error::Cancelled)
    } else {
        Ok(())
    }
}

fn capability_fingerprint(request: &SpeechRequest) -> NfcText {
    NfcText::from(format!(
        "openai-gpt-audio-1.5;voice={};language={};script={};output=wav-pcm16-24000;instructions=verbatim-v1;normalization=NFC;settings={SPEECH_SETTINGS_FINGERPRINT}",
        request.voice().as_str(),
        request.language().language.as_str(),
        request.language().script.as_str(),
    ))
}

fn audio_tokens(duration_micros: u64) -> Option<u64> {
    let tokens = u128::from(duration_micros)
        .checked_mul(u128::from(AUDIO_TOKENS_PER_SECOND))?
        .div_ceil(1_000_000);
    u64::try_from(tokens).ok()
}

/// Audio tokens for the duration ceiling plus a conservative text-output bound.
/// UTF-8 bytes upper-bound byte-BPE tokens, including for one-grapheme runs of
/// combining marks; graphemes remain the contract's character count only.
fn max_completion_tokens(request: &SpeechRequest) -> Option<u64> {
    let text_tokens = u64::try_from(request.text().as_str().len()).ok()?;
    audio_tokens(request.max_duration_micros())?
        .checked_add(text_tokens)?
        .checked_add(COMPLETION_TOKEN_HEADROOM)
}

fn price(tokens: u64, micros_per_million: u64) -> Option<u64> {
    let micros = u128::from(tokens).checked_mul(u128::from(micros_per_million))?;
    u64::try_from(micros.div_ceil(1_000_000)).ok()
}

fn build_body(request: &SpeechRequest, max_completion: u64) -> Result<Vec<u8>, Error> {
    let body = serde_json::to_vec(&json!({
        "model": SPEECH_MODEL,
        "modalities": ["text", "audio"],
        "audio": {
            "voice": request.voice().as_str(),
            "format": "wav",
        },
        "store": false,
        "max_completion_tokens": max_completion,
        "messages": [
            {"role": "system", "content": INSTRUCTIONS},
            {"role": "user", "content": request.text().as_str()},
        ],
    }))
    .map_err(|_| Error::Payload)?;
    if body.len() > MAX_SPEECH_REQUEST_BYTES {
        return Err(Error::Payload);
    }
    Ok(body)
}

fn parse_speech_response(
    response: &HttpResponse,
    request: &SpeechRequest,
) -> Result<SpeechAudio, Error> {
    if response.body.len() > MAX_SPEECH_RESPONSE_BYTES {
        return Err(Error::Payload);
    }
    if !(200..=299).contains(&response.status) {
        return Err(speech_status_error(response));
    }
    let parsed: WireResponse = serde_json::from_slice(&response.body).map_err(|_| Error::Schema)?;
    if parsed.choices.len() != 1 {
        return Err(Error::Schema);
    }
    let choice = parsed.choices.first().ok_or(Error::Schema)?;
    // Valid WAV bytes and a matching transcript do not prove the model finished.
    // Fail closed on a missing/unknown status and classify filtering explicitly.
    match choice.finish_reason.as_deref() {
        Some("stop") => {}
        Some("content_filter") => return Err(Error::ContentPolicy),
        Some(_) | None => return Err(Error::OutputRejected),
    }
    let audio = choice.message.audio.as_ref().ok_or(Error::OutputRejected)?;
    let transcript = audio.transcript.as_deref().ok_or(Error::OutputRejected)?;
    // Normalize the provider transcript before exact comparison; a paraphrase,
    // truncation or missing transcript must never be placed.
    let normalized = NfcText::from(transcript);
    if normalized != *request.text() {
        return Err(Error::OutputRejected);
    }
    let decoded = decode_audio_data(&audio.data, request)?;
    let (samples, sample_rate) = decode_wav(&decoded, max_samples(request)?)?;
    let count = samples.len() as u128;
    let duration_micros = count
        .checked_mul(1_000_000)
        .and_then(|value| value.checked_add(u128::from(sample_rate) - 1))
        .ok_or(Error::OutputRejected)?
        / u128::from(sample_rate);
    let duration_micros = u64::try_from(duration_micros).map_err(|_| Error::OutputRejected)?;
    Ok(SpeechAudio {
        samples,
        sample_rate,
        duration_micros,
        fingerprint: capability_fingerprint(request),
        usage: speech_usage(&parsed.usage)?,
        transcript: Some(normalized),
    })
}

fn max_samples(request: &SpeechRequest) -> Result<u64, Error> {
    let by_duration = u128::from(request.max_duration_micros())
        .checked_mul(u128::from(SPEECH_SAMPLE_RATE))
        .ok_or(Error::OutputRejected)?
        / 1_000_000;
    let by_memory = u128::from(crate::MAX_PCM_BYTES) / 4;
    u64::try_from(by_duration.min(by_memory)).map_err(|_| Error::OutputRejected)
}

fn decode_audio_data(data: &str, request: &SpeechRequest) -> Result<Vec<u8>, Error> {
    if data.is_empty() {
        return Err(Error::OutputRejected);
    }
    // Base64 expands by 4/3. Reserve a small WAV header allowance so hostile
    // payloads are refused before allocation.
    let max_wav = max_samples(request)?
        .checked_mul(2)
        .and_then(|value| value.checked_add(4_096))
        .ok_or(Error::OutputRejected)?;
    let max_base64 = max_wav
        .checked_mul(4)
        .and_then(|value| value.checked_add(2))
        .ok_or(Error::OutputRejected)?
        / 3;
    if u64::try_from(data.len()).map_err(|_| Error::OutputRejected)? > max_base64 {
        return Err(Error::OutputRejected);
    }
    base64::engine::general_purpose::STANDARD
        .decode(data)
        .map_err(|_| Error::OutputRejected)
}

fn decode_wav(bytes: &[u8], max_samples: u64) -> Result<(Vec<f32>, u32), Error> {
    if bytes.get(0..4) != Some(b"RIFF") || bytes.get(8..12) != Some(b"WAVE") {
        return Err(Error::OutputRejected);
    }
    // OpenAI writes 0xFFFFFFFF placeholders for both RIFF and data lengths. A
    // finite RIFF size must cover exactly this body: a larger value is truncated
    // on the wire and a smaller one has bytes outside the declared container.
    let riff_size = u32_at(bytes, 4)?;
    let boundary = if riff_size == u32::MAX {
        bytes.len()
    } else {
        if riff_size < 4 {
            return Err(Error::OutputRejected);
        }
        let end = 8usize
            .checked_add(usize::try_from(riff_size).map_err(|_| Error::OutputRejected)?)
            .ok_or(Error::OutputRejected)?;
        if end != bytes.len() {
            return Err(Error::OutputRejected);
        }
        end
    };
    let mut position = 12usize;
    let mut format = None;
    let mut data = None;
    let mut complete = false;
    while position < boundary {
        let header = bytes
            .get(position..position.checked_add(8).ok_or(Error::OutputRejected)?)
            .ok_or(Error::OutputRejected)?;
        let id = header.get(0..4).ok_or(Error::OutputRejected)?;
        let declared = u32_at(header, 4)?;
        let start = position.checked_add(8).ok_or(Error::OutputRejected)?;
        let available = boundary.checked_sub(start).ok_or(Error::OutputRejected)?;
        let length = if declared == u32::MAX {
            available
        } else {
            usize::try_from(declared).map_err(|_| Error::OutputRejected)?
        };
        if length > available {
            return Err(Error::OutputRejected);
        }
        let chunk = bytes
            .get(start..start.checked_add(length).ok_or(Error::OutputRejected)?)
            .ok_or(Error::OutputRejected)?;
        match id {
            b"fmt " => {
                if format.is_some() {
                    return Err(Error::OutputRejected);
                }
                format = Some(parse_wav_format(chunk)?);
            }
            b"data" => {
                if data.is_some() {
                    return Err(Error::OutputRejected);
                }
                data = Some(chunk);
                if declared == u32::MAX {
                    complete = true;
                    break;
                }
            }
            _ => {}
        }
        let padded = length
            .checked_add(length & 1)
            .ok_or(Error::OutputRejected)?;
        position = start.checked_add(padded).ok_or(Error::OutputRejected)?;
        complete = position == boundary;
    }
    if !complete && position != boundary {
        return Err(Error::OutputRejected);
    }
    let sample_rate = format.ok_or(Error::OutputRejected)?;
    let data = data.ok_or(Error::OutputRejected)?;
    if data.len() % 2 != 0 {
        return Err(Error::OutputRejected);
    }
    let count = data.len() / 2;
    if count == 0 || u64::try_from(count).map_err(|_| Error::OutputRejected)? > max_samples {
        return Err(Error::OutputRejected);
    }
    let mut samples = Vec::with_capacity(count);
    for sample in data.as_chunks::<2>().0 {
        samples.push(f32::from(i16::from_le_bytes(*sample)) / 32_768.0);
    }
    Ok((samples, sample_rate))
}

fn parse_wav_format(chunk: &[u8]) -> Result<u32, Error> {
    if chunk.len() < 16 {
        return Err(Error::OutputRejected);
    }
    let format = u16_at(chunk, 0)?;
    let channels = u16_at(chunk, 2)?;
    let sample_rate = u32_at(chunk, 4)?;
    let byte_rate = u32_at(chunk, 8)?;
    let block_align = u16_at(chunk, 12)?;
    let bits = u16_at(chunk, 14)?;
    if format != 1
        || channels != 1
        || sample_rate != SPEECH_SAMPLE_RATE
        || bits != 16
        || block_align != 2
        || byte_rate != sample_rate.checked_mul(2).ok_or(Error::OutputRejected)?
    {
        return Err(Error::OutputRejected);
    }
    Ok(sample_rate)
}

fn u16_at(bytes: &[u8], offset: usize) -> Result<u16, Error> {
    let end = offset.checked_add(2).ok_or(Error::OutputRejected)?;
    let slice = bytes.get(offset..end).ok_or(Error::OutputRejected)?;
    Ok(u16::from_le_bytes(
        slice.try_into().map_err(|_| Error::OutputRejected)?,
    ))
}

fn u32_at(bytes: &[u8], offset: usize) -> Result<u32, Error> {
    let end = offset.checked_add(4).ok_or(Error::OutputRejected)?;
    let slice = bytes.get(offset..end).ok_or(Error::OutputRejected)?;
    Ok(u32::from_le_bytes(
        slice.try_into().map_err(|_| Error::OutputRejected)?,
    ))
}

fn speech_usage(usage: &WireUsage) -> Result<SpeechUsage, Error> {
    let completion = usage.completion_tokens;
    let (audio_tokens, text_tokens) = usage
        .completion_tokens_details
        .as_ref()
        .map_or((completion, 0), |details| {
            (details.audio_tokens, details.text_tokens)
        });
    let accounted = audio_tokens.checked_add(text_tokens).ok_or(Error::Schema)?;
    if accounted > completion {
        return Err(Error::Schema);
    }
    let other_tokens = completion - accounted;
    let (prompt_text, prompt_audio) = usage
        .prompt_tokens_details
        .as_ref()
        .map_or((usage.prompt_tokens, 0), |details| {
            (details.text_tokens, details.audio_tokens)
        });
    let prompt_accounted = prompt_text.checked_add(prompt_audio).ok_or(Error::Schema)?;
    if prompt_accounted > usage.prompt_tokens {
        return Err(Error::Schema);
    }
    // We send text only; an unreported prompt remainder is charged at the text
    // rate, while reported audio input uses the higher audio rate.
    let prompt_text = prompt_text
        .checked_add(usage.prompt_tokens - prompt_accounted)
        .ok_or(Error::Schema)?;
    let input = price(prompt_text, PRICE_INPUT_MICROS_PER_MILLION)
        .and_then(|value| {
            price(prompt_audio, PRICE_AUDIO_INPUT_MICROS_PER_MILLION)
                .and_then(|audio| value.checked_add(audio))
        })
        .ok_or(Error::Schema)?;
    let output = price(audio_tokens, PRICE_AUDIO_OUTPUT_MICROS_PER_MILLION)
        .and_then(|value| {
            price(text_tokens, PRICE_TEXT_OUTPUT_MICROS_PER_MILLION)
                .and_then(|text| value.checked_add(text))
                .and_then(|value| {
                    price(other_tokens, PRICE_AUDIO_OUTPUT_MICROS_PER_MILLION)
                        .and_then(|other| value.checked_add(other))
                })
        })
        .ok_or(Error::Schema)?;
    let actual = input.checked_add(output).ok_or(Error::Schema)?;
    Ok(SpeechUsage {
        billable_characters: None,
        input_tokens: Some(usage.prompt_tokens),
        output_tokens: Some(completion),
        actual_cost: Money::new("USD", actual).map_err(|_| Error::Schema)?,
    })
}

fn speech_status_error(response: &HttpResponse) -> Error {
    remote::status_error(response, |status, fields| {
        if matches!(status, 400 | 404 | 409 | 422) {
            if remote::matches(fields, &["content_policy", "content_filter", "moderation"]) {
                return Some(Error::ContentPolicy);
            }
            if remote::matches(fields, &["voice"]) {
                return Some(Error::UnsupportedVoice);
            }
        }
        (status == 429 && remote::quota(fields)).then_some(Error::Quota)
    })
}

#[derive(Deserialize)]
struct WireResponse {
    choices: Vec<WireChoice>,
    usage: WireUsage,
}

#[derive(Deserialize)]
struct WireChoice {
    message: WireMessage,
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct WireMessage {
    audio: Option<WireAudio>,
}

#[derive(Deserialize)]
struct WireAudio {
    data: String,
    transcript: Option<String>,
}

#[derive(Deserialize)]
struct WireUsage {
    prompt_tokens: u64,
    completion_tokens: u64,
    completion_tokens_details: Option<WireCompletionDetails>,
    prompt_tokens_details: Option<WirePromptDetails>,
}

#[derive(Deserialize)]
struct WireCompletionDetails {
    #[serde(default)]
    audio_tokens: u64,
    #[serde(default)]
    text_tokens: u64,
}

#[derive(Deserialize)]
struct WirePromptDetails {
    #[serde(default)]
    text_tokens: u64,
    #[serde(default)]
    audio_tokens: u64,
}

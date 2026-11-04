//! Safe whisper.cpp adapter. No transfers, job runner or native unsafe here.
use crate::{
    AudioWindow, Capabilities, CostEstimate, Error, Evidence, ExecutionHooks, Location, Operation,
    ProviderConfig, RecognitionSegment, Request, SampleRange, Scope, Stage, Support, TimingUnit,
    Transcriber,
    contracts::checkpoint,
    models::{Model, storage_error, verify_file},
};
use std::{collections::BTreeSet, path::Path, sync::Once};
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

#[allow(unsafe_code)]
mod callbacks;

pub const SAMPLE_RATE: u32 = 16_000;
/// Five-minute invocation ceiling, not a recommended hard chunk boundary.
/// Whisper chooses its own timestamp boundaries inside this window.
pub const MAX_SAMPLES: u64 = 300 * SAMPLE_RATE as u64;

// Provider-specific language profiles; keep language choices as data. Captions
// owns layout/segmentation profiles, while this adapter owns native decode policy.
const DEFAULT_DECODE: SamplingStrategy = SamplingStrategy::BeamSearch {
    beam_size: 5,
    patience: -1.0,
};
const LANGUAGE_DECODE: &[(&str, SamplingStrategy)] =
    &[("en", SamplingStrategy::Greedy { best_of: 1 })];

fn decode_profile(language: &str) -> SamplingStrategy {
    LANGUAGE_DECODE
        .iter()
        .find(|(tag, _)| *tag == language)
        .map_or_else(|| DEFAULT_DECODE.clone(), |(_, strategy)| strategy.clone())
}

/// Explicit device choice. Engine selects software fallback; adapter never retries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Device {
    Cpu,
    Gpu,
}

/// Raw subword bytes may split a UTF-8 character. Preserve them for captions' span
/// mapping; never decode each token lossily, call it a word, or invent its timing.
#[derive(Clone)]
pub struct NativeToken {
    pub bytes: Vec<u8>,
    pub range: Option<SampleRange>,
    pub probability: f32,
}
impl std::fmt::Debug for NativeToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeToken")
            .field("range", &self.range)
            .finish_non_exhaustive()
    }
}
/// Parallel arrays in native segment order. Segments use complete NFC text;
/// tokens retain original bytes. A conversion consumer must reconcile both texts.
pub struct NativeRecognition {
    pub segments: Vec<RecognitionSegment>,
    pub tokens: Vec<Vec<NativeToken>>,
}
/// Immutable model context; each invocation owns its native state. Engine bounds
/// concurrent invocations and retains/reuses the loaded adapter between jobs.
pub struct Whisper {
    context: WhisperContext,
    config: ProviderConfig,
    languages: Vec<captions::LanguageTag>,
    threads: i32,
}
impl Whisper {
    /// The caller supplies an immutable cache file. Verify size/hash before native
    /// loading, without buffering the model. Loading is synchronous; inference
    /// observes cancellation via native callbacks after each encoder/decode operation.
    /// Device GPU requires macOS Metal or the explicit whisper-vulkan build feature.
    pub fn load(
        model: &Model,
        path: &Path,
        device: Device,
        threads: u8,
        hooks: &dyn ExecutionHooks,
    ) -> Result<Self, Error> {
        model.validate()?;
        let default = crate::models::catalog()?.pop().ok_or(Error::Unsupported)?;
        if *model != default || threads == 0 || threads > 32 {
            return Err(Error::Unsupported);
        }
        if device == Device::Gpu && !cfg!(any(target_os = "macos", feature = "whisper-vulkan")) {
            return Err(Error::Unsupported);
        }
        checkpoint(hooks, Stage::Transcribe)?;
        let file = model.files.first().ok_or(Error::Unsupported)?;
        let mut reader = std::fs::File::open(path).map_err(storage_error)?;
        verify_file(&mut reader, file, hooks)?;
        // Native diagnostics can contain paths or text. With no logging backend
        // enabled these hooks discard all native log payloads process-wide.
        static LOGGING: Once = Once::new();
        LOGGING.call_once(whisper_rs::install_logging_hooks);
        let mut params = WhisperContextParameters::default();
        params.use_gpu(device == Device::Gpu);
        params.flash_attn(false);
        let context =
            WhisperContext::new_with_params(path, params).map_err(|_| Error::Inference)?;
        checkpoint(hooks, Stage::Transcribe)?;
        let config = ProviderConfig::new(
            "local-whisper".into(),
            model.id.as_str().into(),
            format!(
                "whisper-rs-0.16.0:cpp{}:{}:decode-default{DEFAULT_DECODE:?}:decode-languages{LANGUAGE_DECODE:?}:temperature0:fallback0:context-per-call:no-prompt:flash0:max300s:tokens-v1:threads{threads}:{device:?}",
                whisper_rs::WHISPER_CPP_VERSION, file.sha256
            )
            .into(),
            Location::Local {
                backend: "whisper.cpp".into(),
                model_ref: file.sha256.as_str().into(),
            },
        )?;
        Ok(Self {
            context,
            config,
            languages: model.languages.clone(),
            threads: i32::from(threads),
        })
    }
    /// Same validated invocation as Transcriber, additionally retaining raw tokens
    /// for the captions span owner. No source timing is assigned to translated text.
    pub fn recognize_tokens(
        &self,
        audio: AudioWindow<'_>,
        request: &Request<'_>,
        hooks: &dyn ExecutionHooks,
    ) -> Result<NativeRecognition, Error> {
        checkpoint(hooks, Stage::Transcribe)?;
        audio.validate(request.scope)?;
        if request.scope.provider != self.config {
            return Err(Error::Unsupported);
        }
        request.prepare(
            &self.capabilities(request.scope),
            &CostEstimate::Free,
            Operation::Transcribe,
        )?;
        if audio.sample_rate != SAMPLE_RATE || audio.range.len() > MAX_SAMPLES {
            return Err(Error::Unsupported);
        }
        if !request.scope.texts.is_empty() {
            return Err(Error::Invalid("transcription text payload"));
        }
        let mut state = self.context.create_state().map_err(|_| Error::Inference)?;
        let mut params = FullParams::new(decode_profile(request.scope.source_language.as_str()));
        params.set_n_threads(self.threads);
        params.set_language(Some(request.scope.source_language.as_str()));
        params.set_translate(false);
        params.set_no_context(true);
        params.set_temperature(0.0);
        params.set_temperature_inc(0.0);
        params.set_token_timestamps(true);
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        callbacks::run(&mut state, params, audio.samples, hooks)?;
        checkpoint(hooks, Stage::Transcribe)?;
        let mut segments = Vec::new();
        let mut tokens = Vec::new();
        for segment in state.as_iter() {
            checkpoint(hooks, Stage::Transcribe)?;
            if segments.len() >= 4096 {
                return Err(Error::Payload);
            }
            let text = segment
                .to_str()
                .map_err(|_| Error::Invalid("native UTF-8"))?;
            if text.trim().is_empty() {
                continue;
            }
            let range = timestamp_range(
                segment.start_timestamp(),
                segment.end_timestamp(),
                audio.range,
            )?;
            let mut native_tokens = Vec::new();
            if !(0..=4096).contains(&segment.n_tokens()) {
                return Err(Error::Payload);
            }
            for i in 0..segment.n_tokens() {
                let token = segment.get_token(i).ok_or(Error::Inference)?;
                if token.token_id() >= self.context.token_eot() {
                    continue;
                }
                let data = token.token_data();
                let probability = token.token_probability();
                if !probability.is_finite() || !(0.0..=1.0).contains(&probability) {
                    return Err(Error::Invalid("native probability"));
                }
                let bytes = token
                    .to_bytes()
                    .map_err(|_| Error::Invalid("native token"))?;
                if bytes.len() > 65536 {
                    return Err(Error::Payload);
                }
                // Missing, zero-width and out-of-segment token times stay absent.
                let timing = timestamp_range(data.t0, data.t1, audio.range)
                    .ok()
                    .filter(|r| range.contains(*r));
                native_tokens.push(NativeToken {
                    bytes: bytes.to_vec(),
                    range: timing,
                    probability,
                });
            }
            segments.push(RecognitionSegment {
                id: format!(
                    "whisper:{}:{}",
                    audio.range.start(),
                    segment.segment_index()
                )
                .into(),
                text: text.into(),
                range: Some(range),
                timing: Vec::new(),
                raw_tokens: Vec::new(),
            });
            tokens.push(native_tokens);
        }
        Ok(NativeRecognition { segments, tokens })
    }
}
impl Transcriber for Whisper {
    fn config(&self) -> &ProviderConfig {
        &self.config
    }
    fn estimate(&self, _scope: &Scope) -> CostEstimate {
        CostEstimate::Free
    }
    fn capabilities(&self, _scope: &Scope) -> Capabilities {
        Capabilities {
            model: self.config.model().clone(),
            mode: "transcribe".into(),
            operation: Operation::Transcribe,
            languages: self.languages.clone(),
            target_languages: Vec::new(),
            timing: BTreeSet::from([TimingUnit::Segment]),
            diarization: Support::Unsupported,
            live_audio_input: Support::Unsupported,
            file_result_stream: Support::Unsupported,
            max_upload_bytes: Some(MAX_SAMPLES * 4),
            max_duration_micros: Some(MAX_SAMPLES * 1_000_000 / u64::from(SAMPLE_RATE)),
            evidence: Evidence::Advertised,
            revision: "whisper-rs-0.16.0".into(),
        }
    }
    fn transcribe(
        &self,
        audio: AudioWindow<'_>,
        request: &Request<'_>,
        hooks: &dyn ExecutionHooks,
    ) -> Result<Vec<RecognitionSegment>, Error> {
        Ok(self.recognize_tokens(audio, request, hooks)?.segments)
    }
}
/// Native ticks are centiseconds. Convert once to absolute source samples,
/// retaining a clipped final endpoint for native padding; never shift twice.
fn timestamp_range(start: i64, end: i64, window: SampleRange) -> Result<SampleRange, Error> {
    let absolute = |t: i64| -> Result<u64, Error> {
        u64::try_from(t)
            .ok()
            .and_then(|v| v.checked_mul(u64::from(SAMPLE_RATE) / 100))
            .and_then(|v| window.start().checked_add(v))
            .ok_or(Error::Invalid("native timestamp"))
    };
    let range = SampleRange::new(absolute(start)?, absolute(end)?.min(window.end()))?;
    if !window.contains(range) {
        return Err(Error::Invalid("native timestamp outside audio"));
    }
    Ok(range)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn centiseconds_convert_once_and_clip_padding() {
        let window = SampleRange::new(160_000, 176_001).unwrap();
        assert_eq!(
            timestamp_range(0, 10, window).unwrap(),
            SampleRange::new(160_000, 161_600).unwrap()
        );
        assert_eq!(timestamp_range(50, 200, window).unwrap().end(), 176_001);
        for (a, b) in [(-1, 1), (0, 0), (3, 2), (200, 201), (i64::MAX, i64::MAX)] {
            assert!(timestamp_range(a, b, window).is_err());
        }
    }
}

//! Safe whisper.cpp adapter. No transfers, job runner or native unsafe here.
use crate::{
    AudioWindow, Capabilities, CostEstimate, Error, Evidence, ExecutionHooks, Location, Operation,
    Provenance, ProviderConfig, RawToken, RecognitionSegment, Request, SampleRange, Scope, Stage,
    Support, TimingUnit, Transcriber,
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

/// Cache identity for the loaded model. The decode policy is hashed because the
/// full `Debug` tables exceed the engine's 256-byte provider-version bound.
fn settings_fingerprint(cpp_version: &str, model_sha: &str, threads: u8, device: Device) -> String {
    use sha2::Digest as _;
    let policy = format!(
        "decode-default{DEFAULT_DECODE:?}:decode-languages{LANGUAGE_DECODE:?}:temperature0:fallback0:context-per-call:no-prompt:flash0:max300s:tokens-v2"
    );
    let digest = sha2::Sha256::digest(policy.as_bytes());
    let mut text = String::with_capacity(2 * digest.len());
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(text, "{byte:02x}");
    }
    format!("whisper-rs-0.16.0:cpp{cpp_version}:{model_sha}:{text}:threads{threads}:{device:?}")
}

/// Explicit device choice. Engine selects software fallback; adapter never retries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Device {
    Cpu,
    Gpu,
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
        let default = crate::models::catalog()?
            .into_iter()
            .find(|m| m.id == "whisper-large-v3-turbo")
            .ok_or(Error::Unsupported)?;
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
            settings_fingerprint(
                whisper_rs::WHISPER_CPP_VERSION,
                &file.sha256,
                threads,
                device,
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
    fn recognize(
        &self,
        audio: AudioWindow<'_>,
        request: &Request<'_>,
        hooks: &dyn ExecutionHooks,
    ) -> Result<Vec<RecognitionSegment>, Error> {
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
            let mut token_bytes = 0usize;
            if !(0..=4096).contains(&segment.n_tokens()) {
                return Err(Error::Payload);
            }
            for i in 0..segment.n_tokens() {
                let token = segment.get_token(i).ok_or(Error::Inference)?;
                if token.token_id() >= self.context.token_eot() {
                    continue;
                }
                let data = token.token_data();
                let bytes = token
                    .to_bytes()
                    .map_err(|_| Error::Invalid("native token"))?;
                token_bytes = token_bytes.checked_add(bytes.len()).ok_or(Error::Payload)?;
                if token_bytes > 16 * 1024 * 1024 {
                    return Err(Error::Payload);
                }
                native_tokens.push(native_token(
                    bytes,
                    data.t0,
                    data.t1,
                    token.token_probability(),
                    audio.range,
                    range,
                )?);
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
                raw_tokens: native_tokens,
            });
        }
        Ok(segments)
    }
}
impl Transcriber for Whisper {
    fn config(&self) -> &ProviderConfig {
        &self.config
    }
    /// Auto-detect the spoken language from a bounded window using the loaded model.
    /// The caller still supplies the detected tag explicitly to every transcription
    /// request; this method performs no recognition, provider fallback or chunking.
    fn detect_language(
        &self,
        audio: AudioWindow<'_>,
        hooks: &dyn ExecutionHooks,
    ) -> Result<captions::LanguageTag, Error> {
        checkpoint(hooks, Stage::Transcribe)?;
        if audio.sample_rate != SAMPLE_RATE
            || audio.range.len() > MAX_SAMPLES
            || audio.samples.len() as u64 != audio.range.len()
            || audio.samples.iter().any(|v| !v.is_finite())
        {
            return Err(Error::Invalid("language detection window"));
        }
        let mut state = self.context.create_state().map_err(|_| Error::Inference)?;
        state
            .pcm_to_mel(
                audio.samples,
                usize::try_from(self.threads).map_err(|_| Error::Inference)?,
            )
            .map_err(|_| Error::Inference)?;
        checkpoint(hooks, Stage::Transcribe)?;
        let (id, _) = state
            .lang_detect(
                0,
                usize::try_from(self.threads).map_err(|_| Error::Inference)?,
            )
            .map_err(|_| Error::Inference)?;
        checkpoint(hooks, Stage::Transcribe)?;
        let tag = whisper_rs::get_lang_str(id).ok_or(Error::UnsupportedLanguage)?;
        captions::LanguageTag::parse(tag).map_err(Error::Captions)
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
            timing: BTreeSet::from([TimingUnit::Segment, TimingUnit::Token]),
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
        self.recognize(audio, request, hooks)
    }
}
// Retain byte fragments and unknown timing; captions alone maps raw text to NFC.
fn native_token(
    bytes: &[u8],
    start: i64,
    end: i64,
    probability: f32,
    window: SampleRange,
    segment: SampleRange,
) -> Result<RawToken, Error> {
    if bytes.is_empty() || bytes.len() > 65536 {
        return Err(Error::Payload);
    }
    let confidence = captions::Confidence::try_from(probability)
        .map_err(|_| Error::Invalid("native probability"))?;
    // Missing, zero-width and out-of-segment token times stay absent.
    let range = timestamp_range(start, end, window)
        .ok()
        .filter(|r| segment.contains(*r));
    Ok(RawToken {
        bytes: bytes.to_vec(),
        range,
        provenance: Provenance::Native,
        confidence: Some(confidence),
    })
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
    fn settings_fingerprint_fits_cache_identity_bound() {
        let model_sha = "f".repeat(64);
        for device in [Device::Cpu, Device::Gpu] {
            let value = settings_fingerprint("1.8.3", &model_sha, 32, device);
            assert!(value.len() <= 256, "{} bytes: {value}", value.len());
        }
    }

    #[test]
    fn native_fragments_retain_missing_timing_and_shared_nfc_correspondence() {
        let window = SampleRange::new(160_000, 176_000).unwrap();
        let raw = "e\u{302}\u{301}中";
        let tokens = raw
            .as_bytes()
            .iter()
            .enumerate()
            .map(|(i, byte)| {
                let (start, end) = if i == 1 {
                    (-1, -1)
                } else {
                    (i as i64, i as i64 + 1)
                };
                native_token(&[*byte], start, end, 0.9, window, window).unwrap()
            })
            .collect::<Vec<_>>();
        let segment = RecognitionSegment {
            id: "native-fragments".into(),
            text: raw.into(),
            range: Some(window),
            timing: Vec::new(),
            raw_tokens: tokens.clone(),
        };
        // Exercise the same shared execution/conversion seam as the native adapter.
        struct Extracted {
            config: ProviderConfig,
            segment: RecognitionSegment,
        }
        impl Transcriber for Extracted {
            fn config(&self) -> &ProviderConfig {
                &self.config
            }
            fn capabilities(&self, _: &Scope) -> Capabilities {
                Capabilities {
                    model: self.config.model().clone(),
                    mode: "transcribe".into(),
                    operation: Operation::Transcribe,
                    languages: vec![captions::LanguageTag::parse("vi").unwrap()],
                    target_languages: Vec::new(),
                    timing: BTreeSet::from([TimingUnit::Segment, TimingUnit::Token]),
                    diarization: Support::Unsupported,
                    live_audio_input: Support::Unsupported,
                    file_result_stream: Support::Unsupported,
                    max_upload_bytes: None,
                    max_duration_micros: None,
                    evidence: Evidence::FixtureVerified,
                    revision: "native-extraction-fixture".into(),
                }
            }
            fn estimate(&self, _: &Scope) -> CostEstimate {
                CostEstimate::Free
            }
            fn transcribe(
                &self,
                _: AudioWindow<'_>,
                _: &Request<'_>,
                _: &dyn ExecutionHooks,
            ) -> Result<Vec<RecognitionSegment>, Error> {
                Ok(vec![self.segment.clone()])
            }
        }
        struct Hooks;
        impl ExecutionHooks for Hooks {
            fn cancelled(&self) -> bool {
                false
            }
            fn stage(&self, _: Stage) {}
        }
        let config = ProviderConfig::new(
            "local-whisper".into(),
            "fixture".into(),
            "fixture".into(),
            Location::Local {
                backend: "whisper.cpp".into(),
                model_ref: "fixture".into(),
            },
        )
        .unwrap();
        let scope = Scope {
            provider: config.clone(),
            operation: Operation::Transcribe,
            payload_fingerprint: "fixture".into(),
            mode: "transcribe".into(),
            texts: Vec::new(),
            source_language: captions::LanguageTag::parse("vi").unwrap(),
            target_language: None,
            source_samples: window,
            sample_rate: SAMPLE_RATE,
            max_bytes: 64000,
        };
        let pcm = vec![0.0; 16000];
        let result = crate::transcribe(
            &Extracted { config, segment },
            AudioWindow {
                samples: &pcm,
                range: window,
                sample_rate: SAMPLE_RATE,
            },
            &Request {
                scope: &scope,
                consent: None,
            },
            None,
            &Hooks,
        )
        .unwrap();
        assert_eq!(result.segments()[0].raw_tokens, tokens);
        let transcript = result.to_transcript().unwrap();
        let segment = &transcript.segments[0];
        assert!(segment.words.is_empty());
        let spans = segment.spans.as_ref().unwrap();
        assert_eq!(spans.raw(), raw);
        assert_eq!(spans.normalized().text().as_str(), "ế中");
        assert_eq!(spans.spans().len(), raw.len());
        for (i, span) in spans.spans().iter().enumerate() {
            assert_eq!(span.input().index, i);
            assert_eq!(span.input().raw, i..i + 1);
            assert_eq!(span.input().unit, TimingUnit::Token);
            assert_eq!(span.input().provenance, Provenance::Native);
            assert_eq!(span.input().confidence, tokens[i].confidence);
            assert_eq!(span.input().range.is_none(), i == 1);
        }
        assert_eq!(spans.spans()[0].normalized(), spans.spans()[4].normalized());
        let roundtrip: captions::Transcript =
            serde_json::from_str(&serde_json::to_string(&transcript).unwrap()).unwrap();
        assert_eq!(roundtrip, transcript);
        for (start, end) in [(-1, 1), (0, 0), (3, 2), (101, 102)] {
            assert!(
                native_token(b"a", start, end, 0.9, window, window)
                    .unwrap()
                    .range
                    .is_none()
            );
        }
        for probability in [f32::NAN, f32::INFINITY, -0.1, 1.1] {
            assert!(native_token(b"a", 0, 1, probability, window, window).is_err());
        }
        assert!(native_token(b"", 0, 1, 0.9, window, window).is_err());
    }
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

//! Runs only in the bounded native subprocess, never the app process.
use super::{
    AudioRequest, CHUNK_SAMPLES, Device, Invocation, MAX_HEADER, MAX_REPLY, MAX_SAMPLES, Reply,
    SAMPLE_RATE, pinned_model, read_frame, validate_options, verify_model, write_frame,
};
use crate::{Error, ExecutionHooks, RecognitionSegment, Stage};
use crate::{Provenance, RawToken};
use sherpa_onnx::{
    OfflineRecognizer, OfflineRecognizerConfig, OfflineRecognizerResult,
    OfflineTransducerModelConfig,
};
use std::io::{Read, Write};

struct Hooks;
impl ExecutionHooks for Hooks {
    fn cancelled(&self) -> bool {
        false
    }
    fn stage(&self, _: Stage) {}
}
pub(super) fn serve(input: &mut dyn Read, output: &mut dyn Write) -> Result<(), Error> {
    let result = initialize(input);
    let recognizer = match result {
        Ok(value) => value,
        Err(error) => {
            let reply = match error {
                Error::Unsupported => Reply::Unsupported,
                Error::ModelIntegrity => Reply::Integrity,
                _ => Reply::Failed,
            };
            reply_frame(output, &reply)?;
            return Ok(());
        }
    };
    reply_frame(output, &Reply::Ready)?;
    while let Some(bytes) = read_frame(input, MAX_HEADER as u64)? {
        let request: AudioRequest =
            serde_json::from_slice(&bytes).map_err(|_| Error::Invalid("native audio request"))?;
        let result = recognize(&recognizer, input, request.range);
        match result {
            Ok(segments) => reply_frame(output, &Reply::Recognition(segments))?,
            Err(_) => {
                reply_frame(output, &Reply::Failed)?;
                return Ok(());
            }
        }
    }
    Ok(())
}
fn reply_frame(output: &mut dyn Write, reply: &Reply) -> Result<(), Error> {
    let bytes = serde_json::to_vec(reply).map_err(|_| Error::Inference)?;
    if bytes.len() as u64 > MAX_REPLY {
        return Err(Error::Payload);
    }
    write_frame(output, &bytes).map_err(|_| Error::Inference)
}
fn initialize(input: &mut dyn Read) -> Result<OfflineRecognizer, Error> {
    let header = read_frame(input, MAX_HEADER as u64)?.ok_or(Error::Invalid("native header"))?;
    let invocation: Invocation =
        serde_json::from_slice(&header).map_err(|_| Error::Invalid("native header"))?;
    validate_options(&invocation.directory, invocation.device, invocation.threads)?;
    let model = pinned_model()?;
    verify_model(&model, &invocation.directory, &Hooks)?;
    let path = |name| {
        invocation
            .directory
            .join(name)
            .into_os_string()
            .into_string()
            .map_err(|_| Error::Unsupported)
    };
    let mut config = OfflineRecognizerConfig::default();
    config.feat_config.sample_rate = SAMPLE_RATE as i32;
    config.feat_config.feature_dim = 80;
    config.model_config.transducer = OfflineTransducerModelConfig {
        encoder: Some(path("encoder.int8.onnx")?),
        decoder: Some(path("decoder.int8.onnx")?),
        joiner: Some(path("joiner.int8.onnx")?),
    };
    config.model_config.tokens = Some(path("tokens.txt")?);
    config.model_config.num_threads = i32::from(invocation.threads);
    config.model_config.model_type = Some("transducer".into());
    config.model_config.debug = false;
    config.model_config.provider = Some(
        match invocation.device {
            Device::Cpu => "cpu",
            Device::CoreMl => "coreml",
        }
        .into(),
    );
    config.decoding_method = Some("greedy_search".into());
    let recognizer = OfflineRecognizer::create(&config).ok_or(Error::Unsupported)?;
    Ok(recognizer)
}
fn recognize(
    recognizer: &OfflineRecognizer,
    input: &mut dyn Read,
    range: crate::SampleRange,
) -> Result<Vec<RecognitionSegment>, Error> {
    if range.len() > MAX_SAMPLES {
        return Err(Error::Payload);
    }
    let count = usize::try_from(range.len()).map_err(|_| Error::Payload)?;
    let mut remaining = count;
    let mut start = range.start();
    let mut segments = Vec::new();
    // Only one decode chunk is buffered; the parent streams the invocation.
    while remaining > 0 {
        let count = remaining.min(CHUNK_SAMPLES);
        let mut samples = Vec::with_capacity(count);
        let mut buffer = [0u8; 4096];
        while samples.len() < count {
            let size = (count - samples.len()).min(1024) * 4;
            let bytes = buffer.get_mut(..size).ok_or(Error::Payload)?;
            input
                .read_exact(bytes)
                .map_err(|_| Error::Invalid("native audio"))?;
            for bytes in bytes.as_chunks::<4>().0 {
                let sample = f32::from_le_bytes(*bytes);
                if !sample.is_finite() {
                    return Err(Error::Invalid("native audio"));
                }
                samples.push(sample);
            }
        }
        let stream = recognizer.create_stream();
        stream.accept_waveform(SAMPLE_RATE as i32, &samples);
        recognizer.decode(&stream);
        if let Some(segment) = extract(stream.get_result().ok_or(Error::Inference)?, start)? {
            segments.push(segment);
        }
        remaining -= count;
        start = start.checked_add(count as u64).ok_or(Error::Payload)?;
    }
    Ok(segments)
}

fn extract(
    result: OfflineRecognizerResult,
    source_start: u64,
) -> Result<Option<RecognitionSegment>, Error> {
    if result.text.trim().is_empty() && result.tokens.is_empty() {
        return Ok(None);
    }
    if result.tokens.is_empty() || result.tokens.len() > 4096 {
        return Err(Error::Payload);
    }
    let mut size = 0usize;
    for token in &result.tokens {
        size = size.checked_add(token.len()).ok_or(Error::Payload)?;
        if size > 1024 * 1024 || token.is_empty() {
            return Err(Error::Payload);
        }
    }
    let raw = result.tokens.concat();
    // Native text trims the leading BPE space. Preserve exact lexical bytes and
    // validate that trimming is the only difference, rather than rewriting tokens.
    if raw.trim() != result.text.trim() {
        return Err(Error::Invalid("native token text"));
    }
    // RNNT emits token start points without durations. Neither adjacent starts
    // nor the decode-window boundary establish speech ends; do not invent ranges.
    // The explicitly selected aligner may supply them through shared assembly.
    Ok(Some(RecognitionSegment {
        id: format!("gipformer:{source_start}").into(),
        text: raw.into(),
        range: None,
        timing: Vec::new(),
        raw_tokens: result
            .tokens
            .into_iter()
            .map(|token| RawToken {
                bytes: token.into_bytes(),
                range: None,
                provenance: Provenance::Native,
                confidence: None,
            })
            .collect(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn start_only_native_tokens_never_become_word_or_segment_ranges() {
        let segment = extract(
            OfflineRecognizerResult {
                text: "e\u{302}\u{301} trung".into(),
                tokens: vec![" e\u{302}\u{301}".into(), " trung".into()],
                timestamps: Some(vec![0.1, 0.4]),
                durations: None,
            },
            160000,
        )
        .unwrap()
        .unwrap();
        assert!(segment.range.is_none());
        assert!(segment.timing.is_empty());
        assert_eq!(segment.text.as_str(), " ế trung");
        assert!(segment.raw_tokens.iter().all(|t| t.range.is_none()));
        assert_eq!(segment.raw_tokens[0].bytes, " e\u{302}\u{301}".as_bytes());
        assert!(
            extract(
                OfflineRecognizerResult {
                    text: "x".into(),
                    tokens: vec!["y".into()],
                    timestamps: None,
                    durations: None
                },
                0
            )
            .is_err()
        );
    }
    #[test]
    fn hostile_protocol_is_bounded_before_allocation_or_native_loading() {
        for bytes in [vec![], vec![255; 4], vec![3, 0, 0, 0, 0, 1, 2]] {
            assert!(initialize(&mut bytes.as_slice()).is_err());
        }
    }
}

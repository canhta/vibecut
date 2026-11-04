#![cfg(feature = "whisper")]
use ai::{
    AudioWindow, Error, ExecutionHooks, Operation, Request, SampleRange, Scope, Stage, Transcriber,
    whisper::{Device, SAMPLE_RATE, Whisper},
};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};
struct Hooks;
impl ExecutionHooks for Hooks {
    fn cancelled(&self) -> bool {
        false
    }
    fn stage(&self, _: Stage) {}
}
#[test]
#[ignore = "requires explicitly supplied pinned model and local mono 16k f32le speech"]
fn real_local_adapter_preserves_source_coordinates_raw_tokens_and_cancellation() {
    let path = PathBuf::from(std::env::var_os("VIBECUT_WHISPER_MODEL").expect("explicit model"));
    let input = PathBuf::from(std::env::var_os("VIBECUT_WHISPER_AUDIO").expect("explicit audio"));
    let language = std::env::var("VIBECUT_WHISPER_LANGUAGE").expect("explicit language");
    let data = std::fs::read(input).unwrap();
    assert_eq!(data.len() % 4, 0);
    let pcm: Vec<f32> = data
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| f32::from_le_bytes(*b))
        .collect();
    let model = ai::models::catalog().unwrap().remove(0);
    let adapter = Whisper::load(&model, &path, Device::Gpu, 4, &Hooks).unwrap();
    let range = SampleRange::new(160_000, 160_000 + pcm.len() as u64).unwrap();
    let scope = Scope {
        provider: adapter.config().clone(),
        operation: Operation::Transcribe,
        payload_fingerprint: "explicit-local-test-content".into(),
        mode: "transcribe".into(),
        texts: Vec::new(),
        source_language: captions::LanguageTag::parse(&language).unwrap(),
        target_language: None,
        source_samples: range,
        sample_rate: SAMPLE_RATE,
        max_bytes: data.len() as u64,
    };
    let request = Request {
        scope: &scope,
        consent: None,
    };
    let audio = AudioWindow {
        samples: &pcm,
        range,
        sample_rate: SAMPLE_RATE,
    };
    let raw = adapter.recognize_tokens(audio, &request, &Hooks).unwrap();
    assert!(!raw.segments.is_empty());
    assert_eq!(raw.segments.len(), raw.tokens.len());
    for (segment, tokens) in raw.segments.iter().zip(&raw.tokens) {
        assert!(range.contains(segment.range.unwrap()));
        assert!(!tokens.is_empty());
        let bytes: Vec<u8> = tokens
            .iter()
            .flat_map(|t| t.bytes.iter().copied())
            .collect();
        assert_eq!(
            captions::NfcText::from(String::from_utf8(bytes).unwrap()),
            segment.text
        );
        assert!(tokens.iter().any(|t| t.range.is_some()));
        for token in tokens {
            assert!(
                token
                    .range
                    .is_none_or(|r| segment.range.unwrap().contains(r))
            );
        }
    }
    let result = ai::transcribe(&adapter, audio, &request, None, &Hooks).unwrap();
    let transcript = result.to_transcript().unwrap();
    assert_eq!(transcript.segments.len(), raw.segments.len());
    // Cancel at the stage checkpoint after native inference. No partial output escapes.
    struct CancelAfterNative(AtomicUsize);
    impl ExecutionHooks for CancelAfterNative {
        fn cancelled(&self) -> bool {
            self.0.load(Ordering::SeqCst) >= 2
        }
        fn stage(&self, _: Stage) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    assert!(matches!(
        adapter.recognize_tokens(audio, &request, &CancelAfterNative(AtomicUsize::new(0))),
        Err(Error::Cancelled)
    ));
    let bad = AudioWindow {
        sample_rate: 8000,
        ..audio
    };
    assert!(adapter.recognize_tokens(bad, &request, &Hooks).is_err());
}

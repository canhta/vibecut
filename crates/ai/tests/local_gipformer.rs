#![cfg(feature = "gipformer")]
use ai::{
    AudioWindow, Error, ExecutionHooks, Operation, Request, SampleRange, Scope, Stage, Transcriber,
    gipformer::{Device, Gipformer, MAX_SAMPLES, SAMPLE_RATE},
};
use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
    time::{Duration, Instant},
};
struct Hooks;
impl ExecutionHooks for Hooks {
    fn cancelled(&self) -> bool {
        false
    }
    fn stage(&self, _: Stage) {}
}
#[test]
#[ignore = "requires explicitly supplied pinned model directory and local mono 16k f32le speech"]
fn real_gipformer_retains_untimed_tokens_reuses_and_cancels_isolated_native_work() {
    let directory = PathBuf::from(
        std::env::var_os("VIBECUT_GIPFORMER_MODEL").expect("explicit model directory"),
    );
    let audio_path =
        PathBuf::from(std::env::var_os("VIBECUT_GIPFORMER_AUDIO").expect("explicit audio"));
    let bytes = std::fs::read(audio_path).unwrap();
    assert_eq!(bytes.len() % 4, 0);
    let pcm = bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| f32::from_le_bytes(*b))
        .collect::<Vec<_>>();
    let model =
        ai::models::suggested_transcriber(&captions::LanguageTag::parse("vi").unwrap()).unwrap();
    let device = Device::Cpu;
    let adapter = Gipformer::load(
        &model,
        &directory,
        Path::new(env!("CARGO_BIN_EXE_vibecut-local-asr")),
        device,
        6,
        &Hooks,
    )
    .unwrap();
    let range = SampleRange::new(160000, 160000 + pcm.len() as u64).unwrap();
    let scope = Scope {
        provider: adapter.config().clone(),
        operation: Operation::Transcribe,
        payload_fingerprint: "explicit-local-test".into(),
        mode: "transcribe".into(),
        texts: Vec::new(),
        source_language: captions::LanguageTag::parse("vi").unwrap(),
        target_language: None,
        source_samples: range,
        sample_rate: SAMPLE_RATE,
        max_bytes: bytes.len() as u64,
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
    let result = ai::transcribe(&adapter, audio, &request, None, &Hooks).unwrap();
    assert!(!result.segments().is_empty());
    assert!(result.is_text_only());
    assert!(matches!(
        result.to_transcript(),
        Err(Error::TimingUnavailable)
    ));
    assert!(adapter.capabilities(&scope).timing.is_empty());
    for segment in result.segments() {
        assert!(segment.range.is_none());
        assert!(segment.timing.is_empty());
        assert!(!segment.raw_tokens.is_empty());
        assert!(
            segment
                .raw_tokens
                .iter()
                .all(|t| t.range.is_none() && t.provenance == ai::Provenance::Native)
        );
        let raw: Vec<u8> = segment
            .raw_tokens
            .iter()
            .flat_map(|t| &t.bytes)
            .copied()
            .collect();
        assert_eq!(
            captions::NfcText::from(String::from_utf8(raw).unwrap()),
            segment.text
        );
    }
    let roundtrip: Vec<ai::RecognitionSegment> =
        serde_json::from_slice(&serde_json::to_vec(result.segments()).unwrap()).unwrap();
    assert_eq!(roundtrip, result.segments());
    assert_eq!(
        ai::transcribe(&adapter, audio, &request, None, &Hooks)
            .unwrap()
            .segments(),
        result.segments()
    );
    struct Deadline {
        start: Instant,
        ack: AtomicUsize,
    }
    impl ExecutionHooks for Deadline {
        fn cancelled(&self) -> bool {
            self.start.elapsed() > Duration::from_millis(50)
        }
        fn stage(&self, _: Stage) {}
        fn cancelling(&self) {
            self.ack.fetch_add(1, Ordering::SeqCst);
        }
    }
    let long_pcm = pcm
        .iter()
        .copied()
        .cycle()
        .take(MAX_SAMPLES as usize)
        .collect::<Vec<_>>();
    let long_range = SampleRange::new(160000, 160000 + MAX_SAMPLES).unwrap();
    let long_scope = Scope {
        source_samples: long_range,
        max_bytes: MAX_SAMPLES * 4,
        ..scope.clone()
    };
    let deadline = Deadline {
        start: Instant::now(),
        ack: AtomicUsize::new(0),
    };
    let cancelled = ai::transcribe(
        &adapter,
        AudioWindow {
            samples: &long_pcm,
            range: long_range,
            sample_rate: SAMPLE_RATE,
        },
        &Request {
            scope: &long_scope,
            consent: None,
        },
        None,
        &deadline,
    );
    assert!(matches!(cancelled, Err(Error::Cancelled)));
    assert_eq!(deadline.ack.load(Ordering::SeqCst), 1);
    assert!(deadline.start.elapsed() < Duration::from_millis(500));
    eprintln!(
        "device={device:?} cancellation_elapsed_ms={}",
        deadline.start.elapsed().as_millis()
    );
    // A cancelled native process is replaced on the next invocation.
    assert_eq!(
        ai::transcribe(&adapter, audio, &request, None, &Hooks)
            .unwrap()
            .segments(),
        result.segments()
    );
}

#[test]
fn released_static_backend_refuses_coreml_instead_of_silent_cpu_fallback() {
    let model =
        ai::models::suggested_transcriber(&captions::LanguageTag::parse("vi").unwrap()).unwrap();
    assert!(matches!(
        Gipformer::load(
            &model,
            &std::env::temp_dir(),
            Path::new(env!("CARGO_BIN_EXE_vibecut-local-asr")),
            Device::CoreMl,
            4,
            &Hooks
        ),
        Err(Error::Unsupported)
    ));
}

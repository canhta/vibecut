#![cfg(feature = "whisper")]
use ai::{
    AudioWindow, Error, ExecutionHooks, Operation, Request, SampleRange, Scope, Stage, Transcriber,
    recognition::whisper::{Device, SAMPLE_RATE, Whisper},
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
    let model = ai::foundation::models::catalog().unwrap().remove(0);
    let device = if std::env::var_os("VIBECUT_WHISPER_CPU").is_some() {
        Device::Cpu
    } else {
        Device::Gpu
    };
    let adapter = Whisper::load(&model, &path, device, 4, &Hooks).unwrap();
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
    let raw = adapter.transcribe(audio, &request, &Hooks).unwrap();
    assert!(!raw.is_empty());
    let result = ai::transcribe(&adapter, audio, &request, None, &Hooks).unwrap();
    assert_eq!(result.segments(), raw);
    let transcript = result.to_transcript().unwrap();
    assert_eq!(transcript.segments.len(), raw.len());
    for (segment, stored) in raw.iter().zip(&transcript.segments) {
        assert!(range.contains(segment.range.unwrap()));
        assert!(!segment.raw_tokens.is_empty());
        assert!(stored.words.is_empty());
        let spans = stored.spans.as_ref().unwrap();
        let bytes: Vec<u8> = segment
            .raw_tokens
            .iter()
            .flat_map(|t| t.bytes.iter().copied())
            .collect();
        assert_eq!(spans.raw().as_bytes(), bytes);
        assert_eq!(spans.normalized().text(), &segment.text);
        assert_eq!(spans.spans().len(), segment.raw_tokens.len());
        assert!(segment.raw_tokens.iter().any(|t| t.range.is_some()));
        for (ordinal, (token, span)) in segment.raw_tokens.iter().zip(spans.spans()).enumerate() {
            assert_eq!(span.input().index, ordinal);
            assert_eq!(span.input().unit, ai::TimingUnit::Token);
            assert_eq!(span.input().provenance, ai::Provenance::Native);
            assert_eq!(span.input().confidence, token.confidence);
            assert_eq!(span.input().range.is_none(), token.range.is_none());
            if let Some(range) = token.range {
                assert_eq!(
                    span.input().range.unwrap().start(),
                    time::Tick::from_units(
                        range.start() as i64,
                        SAMPLE_RATE as i64,
                        time::Rounding::Floor
                    )
                    .unwrap()
                );
                assert_eq!(
                    span.input().range.unwrap().end(),
                    time::Tick::from_units(
                        range.end() as i64,
                        SAMPLE_RATE as i64,
                        time::Rounding::Floor
                    )
                    .unwrap()
                );
            }
        }
    }
    eprintln!(
        "native_segments={} native_tokens={} utf8_fragments={} missing_timing={}",
        raw.len(),
        raw.iter().map(|s| s.raw_tokens.len()).sum::<usize>(),
        raw.iter()
            .flat_map(|s| &s.raw_tokens)
            .filter(|t| std::str::from_utf8(&t.bytes).is_err())
            .count(),
        raw.iter()
            .flat_map(|s| &s.raw_tokens)
            .filter(|t| t.range.is_none())
            .count()
    );
    let roundtrip: captions::Transcript =
        serde_json::from_str(&serde_json::to_string(&transcript).unwrap()).unwrap();
    assert_eq!(roundtrip, transcript);
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
        adapter.transcribe(audio, &request, &CancelAfterNative(AtomicUsize::new(0))),
        Err(Error::Cancelled)
    ));
    // Cancel during accepted maximum-length native work, not at a checkpoint.
    // The same context must remain usable after each cancelled invocation.
    struct Deadline {
        start: std::time::Instant,
        acknowledged_ms: AtomicUsize,
    }
    impl ExecutionHooks for Deadline {
        fn cancelling(&self) {
            self.acknowledged_ms
                .store(self.start.elapsed().as_millis() as usize, Ordering::SeqCst);
        }
        fn progress(&self, _: Stage, percent: u8) {
            assert!(percent <= 100);
            assert_eq!(
                self.acknowledged_ms.load(Ordering::SeqCst),
                0,
                "progress after cancellation acknowledgement"
            );
        }
        fn cancelled(&self) -> bool {
            self.start.elapsed() >= std::time::Duration::from_secs(1)
        }
        fn stage(&self, _: Stage) {}
    }
    let long_pcm: Vec<_> = pcm
        .iter()
        .copied()
        .cycle()
        .take(ai::recognition::whisper::MAX_SAMPLES as usize)
        .collect();
    let long_range = SampleRange::new(160_000, 160_000 + long_pcm.len() as u64).unwrap();
    let long_scope = Scope {
        source_samples: long_range,
        max_bytes: long_pcm.len() as u64 * 4,
        ..scope.clone()
    };
    let long_request = Request {
        scope: &long_scope,
        consent: None,
    };
    for attempt in 0..3 {
        let deadline = Deadline {
            start: std::time::Instant::now(),
            acknowledged_ms: AtomicUsize::new(0),
        };
        let cancelled = adapter.transcribe(
            AudioWindow {
                samples: &long_pcm,
                range: long_range,
                sample_rate: SAMPLE_RATE,
            },
            &long_request,
            &deadline,
        );
        let elapsed = deadline.start.elapsed();
        let acknowledged = deadline.acknowledged_ms.load(Ordering::SeqCst);
        eprintln!(
            "device={device:?} attempt={attempt} acknowledged_ms={acknowledged} cancellation_delay_ms={}",
            elapsed
                .saturating_sub(std::time::Duration::from_secs(1))
                .as_millis()
        );
        assert!(matches!(cancelled, Err(Error::Cancelled)));
        assert!(
            (1000..1100).contains(&acknowledged),
            "acknowledgement took {acknowledged}ms"
        );
        let bound = match device {
            Device::Cpu => 5500,
            Device::Gpu => 2000,
        };
        assert!(
            elapsed < std::time::Duration::from_millis(bound),
            "cancellation took {elapsed:?}"
        );
    }
    struct PanickingHooks(std::time::Instant);
    impl ExecutionHooks for PanickingHooks {
        fn cancelled(&self) -> bool {
            assert!(
                self.0.elapsed() < std::time::Duration::from_millis(200),
                "synthetic hook failure"
            );
            false
        }
        fn stage(&self, _: Stage) {}
    }
    assert!(matches!(
        adapter.transcribe(audio, &request, &PanickingHooks(std::time::Instant::now())),
        Err(Error::Inference)
    ));
    let reused = adapter.transcribe(audio, &request, &Hooks).unwrap();
    assert!(!reused.is_empty());
    let bad = AudioWindow {
        sample_rate: 8000,
        ..audio
    };
    assert!(adapter.transcribe(bad, &request, &Hooks).is_err());
}

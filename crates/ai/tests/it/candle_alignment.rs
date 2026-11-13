#![cfg(feature = "candle-align")]
#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    reason = "explicit local model fixture"
)]
use ai::foundation::models;
use ai::recognition::alignment::{self, CandleAligner};
use ai::*;
use captions::{LanguageTag, NfcText};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
struct Hooks(AtomicBool);
impl ExecutionHooks for Hooks {
    fn cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
    fn stage(&self, _: Stage) {}
}
struct Deadline(Instant);
impl ExecutionHooks for Deadline {
    fn cancelled(&self) -> bool {
        self.0.elapsed() >= Duration::from_millis(100)
    }
    fn stage(&self, _: Stage) {}
}
struct TextOnly {
    config: ProviderConfig,
    text: NfcText,
}
impl Transcriber for TextOnly {
    fn config(&self) -> &ProviderConfig {
        &self.config
    }
    fn capabilities(&self, scope: &Scope) -> Capabilities {
        Capabilities {
            model: self.config.model().clone(),
            mode: scope.mode.clone(),
            operation: Operation::Transcribe,
            languages: vec![scope.source_language.clone()],
            target_languages: vec![],
            timing: Default::default(),
            diarization: Support::Unsupported,
            live_audio_input: Support::Unsupported,
            file_result_stream: Support::Unsupported,
            max_upload_bytes: None,
            max_duration_micros: None,
            evidence: Evidence::FixtureVerified,
            revision: "fixture".into(),
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
        Ok(vec![RecognitionSegment {
            id: "source-1".into(),
            text: self.text.clone(),
            range: None,
            timing: vec![],
            raw_tokens: vec![],
        }])
    }
}
#[test]
#[ignore = "requires approved immutable Candle model and local 16k PCM fixture"]
fn native_teacher_forcing_uses_shared_assembly_absolute_samples_and_cancellation() {
    let model = PathBuf::from(std::env::var_os("VIBECUT_CANDLE_MODEL_DIR").unwrap());
    let pcm = PathBuf::from(std::env::var_os("VIBECUT_ALIGN_PCM").unwrap());
    let segments = PathBuf::from(std::env::var_os("VIBECUT_ALIGN_SEGMENTS").unwrap());
    let bytes = std::fs::read(pcm).unwrap();
    assert!(bytes.len() <= 480000 * 4);
    let samples: Vec<_> = bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| f32::from_le_bytes(*b))
        .collect();
    let source: Vec<RecognitionSegment> =
        serde_json::from_slice(&std::fs::read(segments).unwrap()).unwrap();
    let hooks = Hooks(AtomicBool::new(false));
    let aligner = CandleAligner::load(&models::alignment_model().unwrap(), &model, &hooks).unwrap();
    let fake = TextOnly {
        config: ProviderConfig::new(
            "fixture".into(),
            "text-only".into(),
            "1".into(),
            Location::Local {
                backend: "fixture".into(),
                model_ref: "fixture".into(),
            },
        )
        .unwrap(),
        text: source[0].text.clone(),
    };
    let mut first = None;
    for offset in [0, 16_000 * 37] {
        let range = SampleRange::new(offset, offset + samples.len() as u64).unwrap();
        let audio = AudioWindow {
            samples: &samples,
            range,
            sample_rate: 16000,
        };
        let recognition = Scope {
            provider: fake.config.clone(),
            operation: Operation::Transcribe,
            payload_fingerprint: "synthetic-eval-input".into(),
            mode: "file".into(),
            texts: vec![],
            source_language: LanguageTag::parse("en").unwrap(),
            target_language: None,
            source_samples: range,
            sample_rate: 16000,
            max_bytes: samples.len() as u64 * 4,
        };
        let mut scope = recognition.clone();
        scope.provider = aligner.config().clone();
        scope.operation = Operation::Align;
        scope.mode = alignment::MODE.into();
        scope.texts = vec![fake.text.clone()];
        let result = transcribe(
            &fake,
            audio,
            &Request {
                scope: &recognition,
                consent: None,
            },
            Some(Alignment {
                provider: &aligner,
                request: Request {
                    scope: &scope,
                    consent: None,
                },
                unit: TimingUnit::Word,
            }),
            &hooks,
        )
        .unwrap();
        assert!(!result.is_text_only());
        let segment = &result.segments()[0];
        assert_eq!(segment.id.as_str(), "source-1");
        assert_eq!(segment.text, fake.text);
        assert!(
            segment
                .timing
                .iter()
                .all(|t| t.provenance == Provenance::Forced && range.contains(t.range))
        );
        let relative: Vec<_> = segment
            .timing
            .iter()
            .map(|t| {
                (
                    t.text.clone(),
                    t.range.start() - offset,
                    t.range.end() - offset,
                )
            })
            .collect();
        if let Some(ref first) = first {
            assert_eq!(
                &relative, first,
                "nonzero offsets must be applied exactly once"
            );
        } else {
            first = Some(relative);
        }
        let transcript = result.to_transcript().unwrap();
        assert!(!transcript.segments[0].words.is_empty());
        let deadline = Instant::now();
        let cancelled = aligner.align(
            audio,
            &source,
            &Request {
                scope: &scope,
                consent: None,
            },
            &Deadline(Instant::now()),
        );
        assert!(matches!(cancelled, Err(Error::Cancelled)));
        assert!(deadline.elapsed() < Duration::from_secs(5));
        assert!(matches!(
            aligner.align(
                AudioWindow {
                    samples: &vec![0.; samples.len()],
                    range,
                    sample_rate: 16000
                },
                &source,
                &Request {
                    scope: &scope,
                    consent: None
                },
                &hooks
            ),
            Err(Error::TimingUnavailable)
        ));
        assert!(matches!(
            aligner.align(
                audio,
                &[],
                &Request {
                    scope: &scope,
                    consent: None
                },
                &hooks
            ),
            Err(Error::Invalid(_))
        ));
    }
}

use ai::*;
use captions::{LanguageTag, NfcText};
use std::{
    collections::BTreeSet,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};

struct Hooks {
    cancelled: AtomicBool,
}
impl Hooks {
    fn new() -> Self {
        Self {
            cancelled: AtomicBool::new(false),
        }
    }
}
impl ExecutionHooks for Hooks {
    fn cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }
    fn stage(&self, _: Stage) {}
}
#[allow(clippy::unwrap_used, reason = "offline fixture constructor")]
fn config(remote: bool) -> ProviderConfig {
    let location = if remote {
        Location::Remote {
            backend: "fake".into(),
            base_url: "https://example.invalid/api".into(),
            auth_ref: "keychain:test".into(),
        }
    } else {
        Location::Local {
            backend: "fake".into(),
            model_ref: "sha256:fixture".into(),
        }
    };
    ProviderConfig::new(
        "test".into(),
        "fixture".into(),
        "settings-v1".into(),
        location,
    )
    .unwrap()
}
#[allow(clippy::unwrap_used, reason = "offline fixture constructor")]
fn scope(config: ProviderConfig, operation: Operation) -> Scope {
    Scope {
        provider: config,
        operation,
        payload_fingerprint: "audio-content-digest".into(),
        mode: "file".into(),
        texts: vec![],
        source_language: LanguageTag::parse("vi").unwrap(),
        target_language: None,
        source_samples: SampleRange::new(16000, 48000).unwrap(),
        sample_rate: 16000,
        max_bytes: 128000,
    }
}
#[allow(clippy::unwrap_used, reason = "offline fixture constructor")]
fn caps(scope: &Scope) -> Capabilities {
    Capabilities {
        model: scope.provider.model().clone(),
        mode: scope.mode.clone(),
        operation: scope.operation,
        languages: vec![
            LanguageTag::parse("vi").unwrap(),
            LanguageTag::parse("zz-ZZ").unwrap(),
        ],
        target_languages: vec![LanguageTag::parse("en").unwrap()],
        timing: BTreeSet::from([TimingUnit::Segment, TimingUnit::Word, TimingUnit::Character]),
        diarization: Support::Unknown,
        live_audio_input: Support::Unsupported,
        file_result_stream: Support::Unsupported,
        max_upload_bytes: Some(128000),
        max_duration_micros: Some(2000000),
        evidence: Evidence::FixtureVerified,
        revision: "fixture-v1".into(),
    }
}
struct Fake {
    config: ProviderConfig,
    result: Vec<RecognitionSegment>,
    calls: AtomicUsize,
    failure: bool,
    cancel: bool,
}
#[allow(clippy::unwrap_used, reason = "offline fake adapter fixture")]
impl Fake {
    fn new(config: ProviderConfig, fixture: &str) -> Self {
        Self {
            config,
            result: serde_json::from_str(fixture).unwrap(),
            calls: AtomicUsize::new(0),
            failure: false,
            cancel: false,
        }
    }
}
#[allow(clippy::unwrap_used, reason = "offline fake adapter fixture")]
impl Transcriber for Fake {
    #[allow(clippy::unwrap_used, reason = "offline fixture constructor")]
    fn config(&self) -> &ProviderConfig {
        &self.config
    }
    fn capabilities(&self, scope: &Scope) -> Capabilities {
        caps(scope)
    }
    fn estimate(&self, _: &Scope) -> CostEstimate {
        CostEstimate::UpperBound(Money::new("USD", 10).unwrap())
    }
    fn transcribe(
        &self,
        _: AudioWindow<'_>,
        _: &Request<'_>,
        _: &dyn ExecutionHooks,
    ) -> Result<Vec<RecognitionSegment>, Error> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        if self.failure {
            return Err(Error::RateLimit {
                retry_after_seconds: Some(2),
            });
        }
        if self.cancel {
            return Err(Error::Cancelled);
        }
        Ok(self.result.clone())
    }
}
impl Aligner for Fake {
    #[allow(clippy::unwrap_used, reason = "offline fixture constructor")]
    fn config(&self) -> &ProviderConfig {
        &self.config
    }
    fn capabilities(&self, scope: &Scope) -> Capabilities {
        caps(scope)
    }
    fn estimate(&self, _: &Scope) -> CostEstimate {
        CostEstimate::Free
    }
    fn align(
        &self,
        _: AudioWindow<'_>,
        _: &[RecognitionSegment],
        _: &Request<'_>,
        _: &dyn ExecutionHooks,
    ) -> Result<Vec<RecognitionSegment>, Error> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        if self.failure {
            return Err(Error::Auth);
        }
        Ok(self.result.clone())
    }
}
#[allow(clippy::unwrap_used, reason = "offline fake adapter fixture")]
impl Translator for Fake {
    #[allow(clippy::unwrap_used, reason = "offline fixture constructor")]
    fn config(&self) -> &ProviderConfig {
        &self.config
    }
    fn capabilities(&self, scope: &Scope) -> Capabilities {
        caps(scope)
    }
    fn estimate(&self, _: &Scope) -> CostEstimate {
        CostEstimate::Free
    }
    fn translate(
        &self,
        _: &NfcText,
        request: &Request<'_>,
        _: &dyn ExecutionHooks,
    ) -> Result<Translation, Error> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        Ok(Translation {
            source_language: request.scope.source_language.clone(),
            target_language: LanguageTag::parse("en").unwrap(),
            text: "Hello".into(),
        })
    }
}
const NATIVE: &str = include_str!("fixtures/native.json");
const TEXT: &str = include_str!("fixtures/text-only.json");
fn audio<'a>(samples: &'a [f32], scope: &Scope) -> AudioWindow<'a> {
    AudioWindow {
        samples,
        range: scope.source_samples,
        sample_rate: scope.sample_rate,
    }
}
#[test]
fn native_fixture_through_execution_preserves_nfc_source_coordinates_and_word_boundaries() {
    let scope = scope(config(false), Operation::Transcribe);
    let fake = Fake::new(scope.provider.clone(), NATIVE);
    let pcm = vec![0.0; 32000];
    let result = transcribe(
        &fake,
        audio(&pcm, &scope),
        &Request {
            scope: &scope,
            consent: None,
        },
        None,
        &Hooks::new(),
    )
    .unwrap();
    assert!(!result.is_text_only());
    assert_eq!(result.segments()[0].text.as_str(), "Xin chào");
    let transcript = result.to_transcript().unwrap();
    assert_eq!(
        transcript.segments[0].range.start(),
        time::Tick::from_units(16000, 16000, time::Rounding::Floor).unwrap()
    );
    assert_eq!(transcript.segments[0].words.len(), 2);
    assert_eq!(fake.calls.load(Ordering::Relaxed), 1);
}
#[test]
fn text_only_never_fabricates_timestamps() {
    let scope = scope(config(false), Operation::Transcribe);
    let fake = Fake::new(scope.provider.clone(), TEXT);
    let pcm = vec![0.0; 32000];
    let result = transcribe(
        &fake,
        audio(&pcm, &scope),
        &Request {
            scope: &scope,
            consent: None,
        },
        None,
        &Hooks::new(),
    )
    .unwrap();
    assert!(result.is_text_only());
    assert!(matches!(
        result.to_transcript(),
        Err(Error::TimingUnavailable)
    ));
}
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    reason = "offline fixture constructor"
)]
fn forced() -> Vec<RecognitionSegment> {
    let mut segments: Vec<RecognitionSegment> = serde_json::from_str(NATIVE).unwrap();
    segments.truncate(1);
    for span in &mut segments[0].timing {
        span.provenance = Provenance::Forced;
    }
    segments
}
#[test]
fn aligner_only_receives_missing_segments_and_native_output_is_unchanged() {
    let scope = scope(config(false), Operation::Transcribe);
    let mut fake = Fake::new(scope.provider.clone(), TEXT);
    let mut native: Vec<RecognitionSegment> = serde_json::from_str(NATIVE).unwrap();
    native.remove(0);
    fake.result.extend(native.clone());
    let mut align_scope = scope.clone();
    align_scope.operation = Operation::Align;
    align_scope.texts = vec!["Xin chào".into()];
    let mut aligner = Fake::new(align_scope.provider.clone(), TEXT);
    aligner.result = forced();
    let pcm = vec![0.0; 32000];
    let result = transcribe(
        &fake,
        audio(&pcm, &scope),
        &Request {
            scope: &scope,
            consent: None,
        },
        Some(Alignment {
            provider: &aligner,
            request: Request {
                scope: &align_scope,
                consent: None,
            },
            unit: TimingUnit::Segment,
        }),
        &Hooks::new(),
    )
    .unwrap();
    assert_eq!(result.segments()[1], native[0]);
    assert!(!result.is_text_only());
    assert!(result.alignment_fingerprint().is_some());
    assert_ne!(
        result.to_transcript().unwrap().fingerprint.settings,
        *scope.provider.settings_fingerprint()
    );
    assert_eq!(aligner.calls.load(Ordering::Relaxed), 1);
}
#[test]
fn unsupported_aligner_leaves_text_only_and_never_runs() {
    let mut scope = scope(config(false), Operation::Transcribe);
    scope.source_language = LanguageTag::parse("zz-ZZ").unwrap();
    let fake = Fake::new(scope.provider.clone(), TEXT);
    let mut align_scope = scope.clone();
    align_scope.operation = Operation::Align;
    let aligner = Fake::new(align_scope.provider.clone(), TEXT);
    let pcm = vec![0.0; 32000];
    let result = transcribe(
        &fake,
        audio(&pcm, &scope),
        &Request {
            scope: &scope,
            consent: None,
        },
        Some(Alignment {
            provider: &aligner,
            request: Request {
                scope: &align_scope,
                consent: None,
            },
            unit: TimingUnit::Token,
        }),
        &Hooks::new(),
    )
    .unwrap();
    assert!(result.is_text_only());
    assert_eq!(aligner.calls.load(Ordering::Relaxed), 0);
}
#[test]
fn consent_binds_endpoint_model_settings_payload_operation_languages_and_text() {
    let scope = scope(config(true), Operation::Transcribe);
    let token = Consent::grant(scope.clone(), Money::new("USD", 10).unwrap()).unwrap();
    let cost = CostEstimate::UpperBound(Money::new("USD", 10).unwrap());
    assert!(authorize(&scope, &cost, Some(&token)).is_ok());
    for field in 0..7 {
        let mut changed = scope.clone();
        match field {
            0 => changed.payload_fingerprint = "another-source".into(),
            1 => changed.mode = "live".into(),
            2 => changed.source_language = LanguageTag::parse("en").unwrap(),
            3 => changed.texts = vec!["secret".into()],
            4 => {
                changed.provider = ProviderConfig::new(
                    "test".into(),
                    "different-model".into(),
                    "settings-v1".into(),
                    scope.provider.location().clone(),
                )
                .unwrap()
            }
            5 => {
                changed.provider = ProviderConfig::new(
                    "test".into(),
                    "fixture".into(),
                    "different-settings".into(),
                    scope.provider.location().clone(),
                )
                .unwrap()
            }
            _ => {
                changed.provider = ProviderConfig::new(
                    "test".into(),
                    "fixture".into(),
                    "settings-v1".into(),
                    Location::Remote {
                        backend: "fake".into(),
                        base_url: "https://other.invalid".into(),
                        auth_ref: "keychain:test".into(),
                    },
                )
                .unwrap()
            }
        }
        assert!(matches!(
            authorize(&changed, &cost, Some(&token)),
            Err(Error::ConsentRequired)
        ));
    }
    assert!(matches!(
        authorize(&scope, &CostEstimate::Unknown, Some(&token)),
        Err(Error::Cost)
    ));
    assert!(matches!(
        authorize(
            &scope,
            &CostEstimate::UpperBound(Money::new("EUR", 1).unwrap()),
            Some(&token)
        ),
        Err(Error::Cost)
    ));
    assert!(matches!(
        authorize(
            &scope,
            &CostEstimate::UpperBound(Money::new("USD", 11).unwrap()),
            Some(&token)
        ),
        Err(Error::Cost)
    ));
    let mut chunk = scope.clone();
    chunk.source_samples = SampleRange::new(16000, 24000).unwrap();
    chunk.max_bytes = 32000;
    assert!(authorize(&chunk, &cost, Some(&token)).is_ok());
    chunk.source_samples = SampleRange::new(0, 24000).unwrap();
    assert!(matches!(
        authorize(&chunk, &cost, Some(&token)),
        Err(Error::ConsentRequired)
    ));
}
#[test]
fn refusal_cancellation_and_hostile_audio_never_call_provider() {
    let scope = scope(config(true), Operation::Transcribe);
    let fake = Fake::new(scope.provider.clone(), NATIVE);
    let pcm = vec![0.0; 32000];
    assert!(matches!(
        transcribe(
            &fake,
            audio(&pcm, &scope),
            &Request {
                scope: &scope,
                consent: None
            },
            None,
            &Hooks::new()
        ),
        Err(Error::ConsentRequired)
    ));
    let hooks = Hooks::new();
    hooks.cancelled.store(true, Ordering::Relaxed);
    assert!(matches!(
        transcribe(
            &fake,
            audio(&pcm, &scope),
            &Request {
                scope: &scope,
                consent: None
            },
            None,
            &hooks
        ),
        Err(Error::Cancelled)
    ));
    let token = Consent::grant(scope.clone(), Money::new("USD", 10).unwrap()).unwrap();
    let mut pcm = pcm;
    pcm[0] = f32::NAN;
    assert!(matches!(
        transcribe(
            &fake,
            audio(&pcm, &scope),
            &Request {
                scope: &scope,
                consent: Some(&token)
            },
            None,
            &Hooks::new()
        ),
        Err(Error::Invalid(_))
    ));
    assert_eq!(fake.calls.load(Ordering::Relaxed), 0);
}
#[test]
fn native_malformed_timing_and_duplicate_ids_are_rejected() {
    for variant in 0..5 {
        let scope = scope(config(false), Operation::Transcribe);
        let mut fake = Fake::new(scope.provider.clone(), NATIVE);
        match variant {
            0 => fake.result[0].range = Some(SampleRange::new(0, 32000).unwrap()),
            1 => fake.result[0].timing[0].text = "different".into(),
            2 => fake.result[0].timing[0].range = SampleRange::new(32000, 40000).unwrap(),
            3 => fake.result[1].id = fake.result[0].id.clone(),
            _ => fake.result[0].timing[0].provenance = Provenance::Forced,
        }
        let pcm = vec![0.0; 32000];
        assert!(matches!(
            transcribe(
                &fake,
                audio(&pcm, &scope),
                &Request {
                    scope: &scope,
                    consent: None
                },
                None,
                &Hooks::new()
            ),
            Err(Error::Invalid(_))
        ));
    }
}
#[test]
fn adapter_errors_are_typed_and_not_hidden_by_fallback() {
    let scope = scope(config(false), Operation::Transcribe);
    let mut fake = Fake::new(scope.provider.clone(), TEXT);
    fake.failure = true;
    let pcm = vec![0.0; 32000];
    assert!(matches!(
        transcribe(
            &fake,
            audio(&pcm, &scope),
            &Request {
                scope: &scope,
                consent: None
            },
            None,
            &Hooks::new()
        ),
        Err(Error::RateLimit {
            retry_after_seconds: Some(2)
        })
    ));
}
#[test]
fn config_roundtrip_schema_validation_url_safety_and_redaction() {
    let config = config(true);
    let serialized = serde_json::to_string(&config).unwrap();
    assert_eq!(
        serde_json::from_str::<ProviderConfig>(&serialized).unwrap(),
        config
    );
    assert!(
        serde_json::from_str::<ProviderConfig>(
            &serialized.replace("\"schema_version\":1", "\"schema_version\":2")
        )
        .is_err()
    );
    for bad in [
        "http://example.invalid",
        "https://user:secret@example.invalid",
        "https://example.invalid?key=secret",
        "https://example.invalid/#secret",
        "not a url",
    ] {
        assert!(
            ProviderConfig::new(
                "id".into(),
                "model".into(),
                "settings".into(),
                Location::Remote {
                    backend: "fake".into(),
                    base_url: bad.into(),
                    auth_ref: "secret-ref".into()
                }
            )
            .is_err()
        );
    }
    let debug = format!("{config:?}");
    assert!(!debug.contains("example.invalid"));
    assert!(!debug.contains("keychain:test"));
    assert!(serde_json::from_str::<SampleRange>("[2,1]").is_err());
    assert!(serde_json::from_str::<SampleRange>("[0,0]").is_err());
    assert!(serde_json::from_str::<RecognitionSegment>("{\"id\":").is_err());
}
#[test]
fn exact_model_mode_locale_and_payload_limits_are_enforced() {
    let mut scope = scope(config(false), Operation::Transcribe);
    let c = caps(&scope);
    scope.source_language = LanguageTag::parse("vi-VN").unwrap();
    assert!(matches!(c.check(&scope), Err(Error::UnsupportedLanguage)));
    scope.source_language = LanguageTag::parse("vi").unwrap();
    scope.mode = "live".into();
    assert!(matches!(c.check(&scope), Err(Error::Unsupported)));
    scope.mode = "file".into();
    scope.max_bytes += 1;
    assert!(matches!(c.check(&scope), Err(Error::Payload)));
    scope.max_bytes -= 1;
    scope.source_samples = SampleRange::new(16000, 48001).unwrap();
    assert!(matches!(c.check(&scope), Err(Error::Payload)));
}
#[test]
fn translation_reuses_policy_binds_exact_text_and_never_returns_timing() {
    let mut scope = scope(config(true), Operation::Translate);
    scope.target_language = Some(LanguageTag::parse("en").unwrap());
    scope.texts = vec!["Xin chào".into()];
    let token = Consent::grant(scope.clone(), Money::new("USD", 0).unwrap()).unwrap();
    let fake = Fake::new(scope.provider.clone(), TEXT);
    let result = translate(
        &fake,
        &"Xin chào".into(),
        &Request {
            scope: &scope,
            consent: Some(&token),
        },
        &Hooks::new(),
    )
    .unwrap();
    assert_eq!(result.text.as_str(), "Hello");
    assert!(matches!(
        translate(
            &fake,
            &"Other private text".into(),
            &Request {
                scope: &scope,
                consent: Some(&token)
            },
            &Hooks::new()
        ),
        Err(Error::ConsentRequired)
    ));
    assert_eq!(fake.calls.load(Ordering::Relaxed), 1);
}
#[test]
fn overlap_merge_uses_source_samples_and_refuses_different_speech_or_source() {
    let scope = scope(config(false), Operation::Transcribe);
    let fake = Fake::new(scope.provider.clone(), NATIVE);
    let pcm = vec![0.0; 32000];
    let hooks = Hooks::new();
    let result = transcribe(
        &fake,
        audio(&pcm, &scope),
        &Request {
            scope: &scope,
            consent: None,
        },
        None,
        &hooks,
    )
    .unwrap();
    let merged = assemble(vec![result.clone(), result.clone()], &hooks).unwrap();
    assert_eq!(merged.segments().len(), 2);
    let mut other = Fake::new(scope.provider.clone(), NATIVE);
    other.result[0].text = "Different".into();
    other.result[0].timing.clear();
    let other = transcribe(
        &other,
        audio(&pcm, &scope),
        &Request {
            scope: &scope,
            consent: None,
        },
        None,
        &hooks,
    )
    .unwrap();
    assert!(matches!(
        assemble(vec![result.clone(), other], &hooks),
        Err(Error::Invalid(_))
    ));
    let mut second_scope = scope.clone();
    second_scope.payload_fingerprint = "different audio".into();
    let other = transcribe(
        &fake,
        audio(&pcm, &second_scope),
        &Request {
            scope: &second_scope,
            consent: None,
        },
        None,
        &hooks,
    )
    .unwrap();
    assert!(matches!(
        assemble(vec![result, other], &hooks),
        Err(Error::Invalid(_))
    ));
}

#[test]
fn alignment_must_preserve_text_and_ids_and_remote_alignment_requires_separate_consent() {
    for variant in 0..3 {
        let scope = scope(config(false), Operation::Transcribe);
        let fake = Fake::new(scope.provider.clone(), TEXT);
        let mut align_scope = scope.clone();
        align_scope.operation = Operation::Align;
        align_scope.texts = vec!["Xin chào".into()];
        if variant == 2 {
            align_scope.provider = config(true);
        }
        let mut aligner = Fake::new(align_scope.provider.clone(), TEXT);
        aligner.result = forced();
        if variant == 0 {
            aligner.result[0].text = "Incorrect".into();
        }
        if variant == 1 {
            aligner.result[0].id = "incorrect-id".into();
        }
        let pcm = vec![0.0; 32000];
        let result = transcribe(
            &fake,
            audio(&pcm, &scope),
            &Request {
                scope: &scope,
                consent: None,
            },
            Some(Alignment {
                provider: &aligner,
                request: Request {
                    scope: &align_scope,
                    consent: None,
                },
                unit: TimingUnit::Word,
            }),
            &Hooks::new(),
        );
        if variant == 2 {
            assert!(matches!(result, Err(Error::ConsentRequired)));
            assert_eq!(aligner.calls.load(Ordering::Relaxed), 0);
        } else {
            assert!(matches!(result, Err(Error::Invalid(_))));
        }
    }
}
#[test]
fn word_output_preserves_speaker_and_validated_confidence() {
    let scope = scope(config(false), Operation::Transcribe);
    let mut fake = Fake::new(scope.provider.clone(), NATIVE);
    fake.result[0].timing[0].speaker = Some("speaker-id".into());
    fake.result[0].timing[0].confidence = Some(captions::Confidence::try_from(0.8).unwrap());
    let pcm = vec![0.0; 32000];
    let result = transcribe(
        &fake,
        audio(&pcm, &scope),
        &Request {
            scope: &scope,
            consent: None,
        },
        None,
        &Hooks::new(),
    )
    .unwrap();
    let transcript = result.to_transcript().unwrap();
    assert_eq!(
        transcript.segments[0].words[0]
            .speaker
            .as_ref()
            .unwrap()
            .as_str(),
        "speaker-id"
    );
    assert_eq!(
        f32::from(transcript.segments[0].words[0].confidence.unwrap()),
        0.8
    );
    assert!(
        serde_json::from_str::<TimedText>(
            r#"{"text":"Xin","unit":"word","range":[1,2],"provenance":"native","confidence":2.0}"#
        )
        .is_err()
    );
}

#[test]
fn character_layers_remain_explicit_and_never_become_words() {
    let scope = scope(config(false), Operation::Transcribe);
    let mut fake = Fake::new(scope.provider.clone(), NATIVE);
    fake.result[1].timing = vec![
        TimedText {
            text: "世".into(),
            unit: TimingUnit::Character,
            range: SampleRange::new(32000, 40000).unwrap(),
            provenance: Provenance::Native,
            speaker: None,
            confidence: None,
        },
        TimedText {
            text: "界".into(),
            unit: TimingUnit::Character,
            range: SampleRange::new(40000, 48000).unwrap(),
            provenance: Provenance::Native,
            speaker: None,
            confidence: None,
        },
    ];
    let pcm = vec![0.0; 32000];
    let result = transcribe(
        &fake,
        audio(&pcm, &scope),
        &Request {
            scope: &scope,
            consent: None,
        },
        None,
        &Hooks::new(),
    )
    .unwrap();
    assert_eq!(result.segments()[1].timing[0].unit, TimingUnit::Character);
    assert!(matches!(result.to_transcript(), Err(Error::Unsupported)));
}
#[test]
fn alignment_preserves_existing_native_unit_layers() {
    let scope = scope(config(false), Operation::Transcribe);
    let mut fake = Fake::new(scope.provider.clone(), NATIVE);
    fake.result.truncate(1);
    let native = TimedText {
        text: "X".into(),
        unit: TimingUnit::Character,
        range: SampleRange::new(16000, 17000).unwrap(),
        provenance: Provenance::Native,
        speaker: None,
        confidence: None,
    };
    fake.result[0].timing = vec![native.clone()];
    let mut align_scope = scope.clone();
    align_scope.operation = Operation::Align;
    align_scope.texts = vec!["Xin chào".into()];
    let mut aligner = Fake::new(align_scope.provider.clone(), TEXT);
    aligner.result = forced();
    let pcm = vec![0.0; 32000];
    let result = transcribe(
        &fake,
        audio(&pcm, &scope),
        &Request {
            scope: &scope,
            consent: None,
        },
        Some(Alignment {
            provider: &aligner,
            request: Request {
                scope: &align_scope,
                consent: None,
            },
            unit: TimingUnit::Word,
        }),
        &Hooks::new(),
    )
    .unwrap();
    assert!(result.segments()[0].timing.contains(&native));
    assert_eq!(
        result.segments()[0]
            .timing
            .iter()
            .filter(|t| t.provenance == Provenance::Forced)
            .count(),
        2
    );
}

#[test]
fn review_mixed_native_and_aligned_chunks_can_assemble() {
    let first_scope = scope(config(false), Operation::Transcribe);
    let native = Fake::new(first_scope.provider.clone(), NATIVE);
    let pcm = vec![0.0; 32000];
    let hooks = Hooks::new();
    let first = transcribe(
        &native,
        audio(&pcm, &first_scope),
        &Request {
            scope: &first_scope,
            consent: None,
        },
        None,
        &hooks,
    )
    .unwrap();
    let mut second_scope = first_scope.clone();
    second_scope.source_samples = SampleRange::new(48000, 80000).unwrap();
    let mut second_fake = Fake::new(second_scope.provider.clone(), TEXT);
    second_fake.result[0].id = "second-chunk".into();
    let mut align_scope = second_scope.clone();
    align_scope.operation = Operation::Align;
    align_scope.texts = vec!["Xin chào".into()];
    let mut aligner = Fake::new(align_scope.provider.clone(), TEXT);
    aligner.result = forced();
    aligner.result[0].id = "second-chunk".into();
    aligner.result[0].range = Some(SampleRange::new(48000, 64000).unwrap());
    for span in &mut aligner.result[0].timing {
        span.range =
            SampleRange::new(span.range.start() + 32000, span.range.end() + 32000).unwrap();
    }
    let second = transcribe(
        &second_fake,
        audio(&pcm, &second_scope),
        &Request {
            scope: &second_scope,
            consent: None,
        },
        Some(Alignment {
            provider: &aligner,
            request: Request {
                scope: &align_scope,
                consent: None,
            },
            unit: TimingUnit::Word,
        }),
        &hooks,
    )
    .unwrap();
    let alignment = second.alignment_fingerprint().unwrap().clone();
    assert!(first.alignment_fingerprint().is_none());
    for chunks in [
        vec![first.clone(), second.clone()],
        vec![second.clone(), first.clone()],
    ] {
        let merged = assemble(chunks, &hooks).unwrap();
        assert_eq!(merged.segments().len(), 3);
        assert_eq!(merged.window(), SampleRange::new(16000, 80000).unwrap());
        assert_eq!(merged.alignment_fingerprint(), Some(&alignment));
        assert_eq!(merged.to_transcript().unwrap().segments.len(), 3);
        assert_ne!(
            merged.to_transcript().unwrap().fingerprint.settings,
            *first_scope.provider.settings_fingerprint()
        );
    }
    let mut other_align_scope = align_scope.clone();
    other_align_scope.provider = ProviderConfig::new(
        "another-aligner".into(),
        "fixture".into(),
        "settings-v2".into(),
        align_scope.provider.location().clone(),
    )
    .unwrap();
    let mut other_aligner = Fake::new(other_align_scope.provider.clone(), TEXT);
    other_aligner.result = aligner.result.clone();
    let conflicting = transcribe(
        &second_fake,
        audio(&pcm, &second_scope),
        &Request {
            scope: &second_scope,
            consent: None,
        },
        Some(Alignment {
            provider: &other_aligner,
            request: Request {
                scope: &other_align_scope,
                consent: None,
            },
            unit: TimingUnit::Word,
        }),
        &hooks,
    )
    .unwrap();
    assert!(matches!(
        assemble(vec![second, conflicting], &hooks),
        Err(Error::Invalid("incompatible alignment fingerprints"))
    ));
}

#[test]
fn review_single_response_does_not_discard_distinct_equal_text_speech() {
    let scope = scope(config(false), Operation::Transcribe);
    let mut fake = Fake::new(scope.provider.clone(), NATIVE);
    fake.result[0].text = "Yes".into();
    fake.result[0].timing.clear();
    fake.result[0].range = Some(SampleRange::new(16000, 40000).unwrap());
    fake.result[1].text = "Yes".into();
    fake.result[1].timing.clear();
    fake.result[1].range = Some(SampleRange::new(32000, 48000).unwrap());
    let pcm = vec![0.0; 32000];
    let result = transcribe(
        &fake,
        audio(&pcm, &scope),
        &Request {
            scope: &scope,
            consent: None,
        },
        None,
        &Hooks::new(),
    );
    assert!(
        result.as_ref().is_err() || result.as_ref().unwrap().segments().len() == 2,
        "distinct IDs and speech ranges must not silently lose a segment"
    );
}

#[test]
fn review_equal_text_overlap_requires_identical_event_identity_and_all_metadata() {
    let scope = scope(config(false), Operation::Transcribe);
    let pcm = vec![0.0; 32000];
    let hooks = Hooks::new();
    let mut fake = Fake::new(scope.provider.clone(), NATIVE);
    fake.result.truncate(1);
    let first = transcribe(
        &fake,
        audio(&pcm, &scope),
        &Request {
            scope: &scope,
            consent: None,
        },
        None,
        &hooks,
    )
    .unwrap();
    for variant in 0..4 {
        let mut changed = Fake::new(scope.provider.clone(), NATIVE);
        changed.result.truncate(1);
        match variant {
            0 => changed.result[0].id = "another-event".into(),
            1 => {
                changed.result[0].range = Some(SampleRange::new(24000, 40000).unwrap());
                changed.result[0].timing.clear();
            }
            2 => changed.result[0].timing[0].speaker = Some("another-speaker".into()),
            _ => {
                changed.result[0].timing[0].confidence =
                    Some(captions::Confidence::try_from(0.5).unwrap())
            }
        }
        let second = transcribe(
            &changed,
            audio(&pcm, &scope),
            &Request {
                scope: &scope,
                consent: None,
            },
            None,
            &hooks,
        )
        .unwrap();
        assert!(matches!(
            assemble(vec![first.clone(), second], &hooks),
            Err(Error::Invalid("ambiguous overlapping recognition"))
        ));
    }
    let identical = assemble(vec![first.clone(), first], &hooks).unwrap();
    assert_eq!(identical.segments().len(), 1);
}

#[test]
fn review_same_text_repetition_at_disjoint_times_keeps_both_events() {
    let scope = scope(config(false), Operation::Transcribe);
    let mut fake = Fake::new(scope.provider.clone(), NATIVE);
    for segment in &mut fake.result {
        segment.text = "Yes".into();
        segment.timing.clear();
    }
    let pcm = vec![0.0; 32000];
    let result = transcribe(
        &fake,
        audio(&pcm, &scope),
        &Request {
            scope: &scope,
            consent: None,
        },
        None,
        &Hooks::new(),
    )
    .unwrap();
    assert_eq!(result.segments().len(), 2);
    assert_eq!(result.segments()[1].range.unwrap().end(), 48000);
}

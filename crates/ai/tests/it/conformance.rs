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
        timing: BTreeSet::from([
            TimingUnit::Segment,
            TimingUnit::Word,
            TimingUnit::Character,
            TimingUnit::Token,
        ]),
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
    untimed: bool,
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
            untimed: false,
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
        let mut caps = caps(scope);
        if self.untimed {
            caps.timing.clear();
        }
        caps
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
        window: &TranslationWindow,
        request: &Request<'_>,
        _: &dyn ExecutionHooks,
    ) -> Result<Translation, Error> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        Ok(Translation {
            source_language: request.scope.source_language.clone(),
            target_language: LanguageTag::parse("en").unwrap(),
            texts: [(window.segments.first().unwrap().id.clone(), "Hello".into())].into(),
            number_warnings: vec![],
        })
    }
}
const NATIVE: &str = include_str!("../fixtures/native.json");
const TEXT: &str = include_str!("../fixtures/text-only.json");
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
            unit: TimingUnit::Grapheme,
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
fn wire_upload_ceiling_does_not_bound_in_memory_pcm() {
    let mut scope = scope(config(true), Operation::Transcribe);
    scope.max_bytes = 64_000;
    let token = Consent::grant(scope.clone(), Money::new("USD", 10).unwrap()).unwrap();
    let fake = Fake::new(scope.provider.clone(), TEXT);
    let pcm = vec![0.0; 32_000];
    let result = transcribe(
        &fake,
        audio(&pcm, &scope),
        &Request {
            scope: &scope,
            consent: Some(&token),
        },
        None,
        &Hooks::new(),
    );
    assert!(result.is_ok());
    assert_eq!(fake.calls.load(Ordering::Relaxed), 1);
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
    let window = TranslationWindow {
        source_language: scope.source_language.clone(),
        target_language: scope.target_language.clone().unwrap(),
        segments: vec![TranslationSegment {
            id: "1".into(),
            text: "Xin chào".into(),
        }],
        before: vec![],
        after: vec![],
        glossary: vec![],
        style: "".into(),
    };
    scope.texts = vec![window.consent_text().unwrap()];
    let token = Consent::grant(scope.clone(), Money::new("USD", 0).unwrap()).unwrap();
    let fake = Fake::new(scope.provider.clone(), TEXT);
    let result = translate(
        &fake,
        &window,
        &Request {
            scope: &scope,
            consent: Some(&token),
        },
        &Hooks::new(),
    )
    .unwrap();
    assert_eq!(
        result.texts.get(&NfcText::from("1")).unwrap().as_str(),
        "Hello"
    );
    let mut other = window.clone();
    other.style = "Other private text".into();
    assert!(matches!(
        translate(
            &fake,
            &other,
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
    // Same text at disjoint times stays two events.
    let mut repeated = Fake::new(scope.provider.clone(), NATIVE);
    for segment in &mut repeated.result {
        segment.text = "Yes".into();
        segment.timing.clear();
    }
    let repeated = transcribe(
        &repeated,
        audio(&pcm, &scope),
        &Request {
            scope: &scope,
            consent: None,
        },
        None,
        &hooks,
    )
    .unwrap();
    assert_eq!(repeated.segments().len(), 2);
    assert_eq!(repeated.segments()[1].range.unwrap().end(), 48000);
    // Distinct IDs and overlapping speech ranges in one response keep both.
    let mut overlapping = Fake::new(scope.provider.clone(), NATIVE);
    overlapping.result[0].text = "Yes".into();
    overlapping.result[0].timing.clear();
    overlapping.result[0].range = Some(SampleRange::new(16000, 40000).unwrap());
    overlapping.result[1].text = "Yes".into();
    overlapping.result[1].timing.clear();
    overlapping.result[1].range = Some(SampleRange::new(32000, 48000).unwrap());
    let overlapping = transcribe(
        &overlapping,
        audio(&pcm, &scope),
        &Request {
            scope: &scope,
            consent: None,
        },
        None,
        &hooks,
    );
    assert!(
        overlapping.as_ref().is_err() || overlapping.as_ref().unwrap().segments().len() == 2,
        "distinct IDs and speech ranges must not silently lose a segment"
    );
    // Equal text overlap is refused unless the whole event identity and metadata match.
    let mut single = Fake::new(scope.provider.clone(), NATIVE);
    single.result.truncate(1);
    let first = transcribe(
        &single,
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
    assert_eq!(
        assemble(vec![first.clone(), first], &hooks)
            .unwrap()
            .segments()
            .len(),
        1
    );
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
    let transcript = result.to_transcript().unwrap();
    assert!(transcript.segments[1].words.is_empty());
    let spans = transcript.segments[1].spans.as_ref().unwrap();
    assert_eq!(spans.spans().len(), 2);
    assert!(
        spans
            .spans()
            .iter()
            .all(|s| s.input().unit == TimingUnit::Character)
    );
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
fn mixed_native_and_aligned_chunks_assemble_under_one_fingerprint() {
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

#[allow(clippy::unwrap_used, reason = "offline raw token fixture")]
fn raw_result(raw: &str) -> RecognitionSegment {
    RecognitionSegment {
        id: "raw-segment".into(),
        text: raw.into(),
        range: Some(SampleRange::new(16000, 16000 + raw.len() as u64 * 100).unwrap()),
        timing: vec![],
        raw_tokens: raw
            .bytes()
            .enumerate()
            .map(|(i, b)| RawToken {
                bytes: vec![b],
                range: Some(
                    SampleRange::new(16000 + i as u64 * 100, 16100 + i as u64 * 100).unwrap(),
                ),
                provenance: Provenance::Native,
                confidence: Some(captions::Confidence::try_from(0.9).unwrap()),
            })
            .collect(),
    }
}
#[test]
fn raw_tokens_through_public_execution_retain_utf8_correspondence_and_source_samples() {
    let scope = scope(config(false), Operation::Transcribe);
    let raw = "e\u{302}\u{301}中👩🏽‍💻";
    let segment = raw_result(raw);
    let mut fake = Fake::new(scope.provider.clone(), TEXT);
    fake.result = vec![segment.clone()];
    let pcm = vec![0.0; 32000];
    let recognition = transcribe(
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
    assert_eq!(recognition.segments()[0].raw_tokens, segment.raw_tokens);
    let merged = assemble(
        vec![recognition.clone(), recognition.clone()],
        &Hooks::new(),
    )
    .unwrap();
    assert_eq!(merged.segments().len(), 1);
    let transcript = merged.to_transcript().unwrap();
    let segment = &transcript.segments[0];
    assert!(segment.words.is_empty());
    let spans = segment.spans.as_ref().unwrap();
    assert_eq!(spans.raw(), raw);
    assert_eq!(spans.normalized().text().as_str(), "ế中👩🏽‍💻");
    assert_eq!(spans.spans().len(), raw.len());
    for (i, span) in spans.spans().iter().enumerate() {
        assert_eq!(span.input().index, i);
        assert_eq!(span.input().unit, TimingUnit::Token);
        assert_eq!(span.input().provenance, Provenance::Native);
        assert_eq!(span.input().raw, i..i + 1);
        assert_eq!(
            span.input().range.unwrap().start(),
            time::Tick::from_units(16000 + i as i64 * 100, 16000, time::Rounding::Floor).unwrap()
        );
        assert_eq!(f32::from(span.input().confidence.unwrap()), 0.9);
    }
    assert_eq!(spans.spans()[0].normalized(), spans.spans()[4].normalized());
    assert_eq!(segment.range.start(), time::Tick(time::TICKS_PER_SECOND));
    let roundtrip: captions::Transcript =
        serde_json::from_str(&serde_json::to_string(&transcript).unwrap()).unwrap();
    assert_eq!(roundtrip, transcript);
    let profile = captions::LanguageRegistry::standard()
        .unwrap()
        .profile(&scope.source_language)
        .clone();
    let rules = captions::SegmentRules::from_profile(&profile, time::FrameRate::FPS_24).unwrap();
    let generated = captions::segment_transcript(
        &transcript,
        &profile,
        &rules,
        &captions::LayoutHooks::default(),
    )
    .unwrap();
    assert!(
        generated
            .cues
            .iter()
            .all(|c| c.timing == captions::TimingKind::SourceSpans)
    );
    assert_eq!(generated.cues[0].cue.range.start(), segment.range.start());
    assert_eq!(
        generated.cues.last().unwrap().cue.range.end(),
        segment.range.end()
    );
    // Token metadata is part of overlap identity, not disposable adapter detail.
    fake.result[0].raw_tokens[0].confidence = None;
    let changed = transcribe(
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
    assert!(assemble(vec![recognition, changed], &Hooks::new()).is_err());
}
#[test]
fn raw_tokens_missing_timing_and_character_layers_survive_conversion_without_fake_words() {
    let scope = scope(config(false), Operation::Transcribe);
    let mut fake = Fake::new(scope.provider.clone(), TEXT);
    let mut native = raw_result("e\u{302}\u{301}中");
    native.raw_tokens[1].range = None;
    native.timing = vec![TimedText {
        text: "ế".into(),
        unit: TimingUnit::Character,
        range: SampleRange::new(16000, 16500).unwrap(),
        provenance: Provenance::Native,
        speaker: Some("speaker".into()),
        confidence: None,
    }];
    fake.result = vec![native];
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
    .unwrap()
    .to_transcript()
    .unwrap();
    let spans = result.segments[0].spans.as_ref().unwrap();
    assert!(spans.spans()[1].input().range.is_none());
    let character = spans.spans().last().unwrap();
    assert_eq!(character.input().unit, TimingUnit::Character);
    assert_eq!(character.input().raw, 0..5);
    assert_eq!(
        character.input().speaker.as_ref().unwrap().as_str(),
        "speaker"
    );
    assert!(result.segments[0].words.is_empty());
}
#[test]
fn hostile_raw_tokens_fail_through_public_execution_without_lossy_decoding() {
    let scope = scope(config(false), Operation::Transcribe);
    let mut fake = Fake::new(scope.provider.clone(), TEXT);
    let native = raw_result("中");
    let pcm = vec![0.0; 32000];
    for case in 0..8 {
        let mut invalid = native.clone();
        match case {
            0 => invalid.raw_tokens.pop().map(|_| ()).unwrap(),
            1 => invalid.raw_tokens[0].bytes.clear(),
            2 => invalid.raw_tokens[0].range = Some(SampleRange::new(0, 1).unwrap()),
            3 => invalid.raw_tokens[1].range = invalid.raw_tokens[0].range,
            4 => invalid.raw_tokens[0].provenance = Provenance::Forced,
            5 => invalid.text = "other".into(),
            6 => invalid.range = None,
            _ => invalid.timing.push(TimedText {
                text: "中".into(),
                unit: TimingUnit::Token,
                range: invalid.range.unwrap(),
                provenance: Provenance::Native,
                speaker: None,
                confidence: None,
            }),
        }
        fake.result = vec![invalid];
        assert!(
            transcribe(
                &fake,
                audio(&pcm, &scope),
                &Request {
                    scope: &scope,
                    consent: None
                },
                None,
                &Hooks::new()
            )
            .is_err(),
            "case {case}"
        );
    }
    assert!(!format!("{:?}", native.raw_tokens[0]).contains("bytes"));
}

#[test]
fn alignment_preserves_native_token_layer_in_either_transport_representation() {
    for native_raw in [false, true] {
        let scope = scope(config(false), Operation::Transcribe);
        let mut native = raw_result("Xin chào");
        native.id = "one".into();
        native.range = Some(SampleRange::new(16000, 32000).unwrap());
        if !native_raw {
            native.raw_tokens.clear();
            native.timing.push(TimedText {
                text: native.text.clone(),
                unit: TimingUnit::Token,
                range: native.range.unwrap(),
                provenance: Provenance::Native,
                speaker: None,
                confidence: None,
            });
        }
        let mut fake = Fake::new(scope.provider.clone(), TEXT);
        fake.result = vec![native.clone()];
        let mut align_scope = scope.clone();
        align_scope.operation = Operation::Align;
        align_scope.texts = vec![native.text.clone()];
        let mut aligner = Fake::new(align_scope.provider.clone(), TEXT);
        aligner.result = forced();
        aligner.result[0].raw_tokens = raw_result("Xin chào")
            .raw_tokens
            .into_iter()
            .map(|mut t| {
                t.provenance = Provenance::Forced;
                t
            })
            .collect();
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
        if native_raw {
            assert_eq!(result.segments()[0].raw_tokens, native.raw_tokens);
        } else {
            assert!(result.segments()[0].raw_tokens.is_empty());
            assert!(result.segments()[0].timing.contains(&native.timing[0]));
        }
        let transcript = result.to_transcript().unwrap();
        assert_eq!(transcript.segments[0].words.len(), 2);
        assert!(
            transcript.segments[0]
                .spans
                .as_ref()
                .unwrap()
                .spans()
                .iter()
                .filter(|s| s.input().unit == TimingUnit::Token)
                .all(|s| s.input().provenance == Provenance::Native)
        );
    }
}

#[test]
fn token_alignment_reconciles_only_matching_ordinals_and_preserves_native_evidence() {
    for case in 0..11 {
        let scope = scope(config(false), Operation::Transcribe);
        let mut original = raw_result("abcd");
        if case != 0 {
            original.raw_tokens[1].range = None;
            original.raw_tokens[3].range = None;
        }
        if case == 9 || case == 10 {
            for token in &mut original.raw_tokens {
                token.range = None;
            }
        }
        if case == 4 {
            original.raw_tokens[1].bytes = b"bc".to_vec();
            original.raw_tokens.remove(2);
        }
        let mut native = Fake::new(scope.provider.clone(), TEXT);
        native.result = vec![original.clone()];
        let mut align_scope = scope.clone();
        align_scope.operation = Operation::Align;
        align_scope.texts = vec![original.text.clone()];
        let mut aligned = raw_result("abcd");
        for token in &mut aligned.raw_tokens {
            token.provenance = Provenance::Forced;
            token.confidence = None;
        }
        // A different forced range for an already timed token must not replace it.
        aligned.raw_tokens[0].range = Some(SampleRange::new(16000, 16050).unwrap());
        match case {
            2 => aligned.raw_tokens[1].range = None,
            3 => {
                // Equal concatenated text is insufficient: token ordinals changed.
                aligned.raw_tokens[0].bytes.extend(b"b");
                aligned.raw_tokens.remove(1);
            }
            4 => {
                // Same count and concatenated text, different byte boundaries.
                aligned.raw_tokens[0].bytes = b"ab".to_vec();
                aligned.raw_tokens.remove(1);
            }
            5 => {
                aligned.raw_tokens.clear();
                aligned.timing.push(TimedText {
                    text: aligned.text.clone(),
                    unit: TimingUnit::Token,
                    range: aligned.range.unwrap(),
                    provenance: Provenance::Forced,
                    confidence: None,
                    speaker: None,
                });
            }
            6 => {
                // Both inputs are valid independently, but the filled range
                // overlaps a retained native timestamp.
                aligned.raw_tokens[1].range = Some(SampleRange::new(16050, 16200).unwrap());
            }
            7 => aligned.range = Some(SampleRange::new(16000, 17000).unwrap()),
            8 => {
                aligned.range = Some(SampleRange::new(16000, 17000).unwrap());
                aligned.raw_tokens[3].range = Some(SampleRange::new(16400, 16500).unwrap());
            }
            10 => {
                for token in &mut aligned.raw_tokens {
                    token.range = None;
                }
            }
            _ => {}
        }
        let mut aligner = Fake::new(align_scope.provider.clone(), TEXT);
        aligner.result = vec![aligned.clone()];
        let pcm = vec![0.0; 32000];
        let result = transcribe(
            &native,
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
        );
        assert_eq!(
            aligner.calls.load(Ordering::Relaxed),
            usize::from(case != 0),
            "case {case}"
        );
        if matches!(case, 2..=6 | 8 | 10) {
            assert!(
                matches!(result, Err(Error::Invalid(_))),
                "case {case}: {result:?}"
            );
            continue;
        }
        let result = result.unwrap();
        let segment = &result.segments()[0];
        assert_eq!(segment.range, original.range);
        assert_eq!(result.alignment_fingerprint().is_some(), case != 0);
        for (i, token) in segment.raw_tokens.iter().enumerate() {
            let mut expected = original.raw_tokens[i].clone();
            if expected.range.is_none() {
                expected.range = aligned.raw_tokens[i].range;
                expected.provenance = Provenance::Forced;
            }
            assert_eq!(token, &expected, "case {case}, token {i}");
        }
        let transcript = result.to_transcript().unwrap();
        let spans = transcript.segments[0].spans.as_ref().unwrap().spans();
        for (i, span) in spans.iter().enumerate() {
            assert_eq!(span.input().index, i);
            assert_eq!(span.input().raw, i..i + 1);
            assert!(span.input().range.is_some());
            assert_eq!(span.input().provenance, segment.raw_tokens[i].provenance);
        }
    }
}

#[test]
fn untimed_raw_bytes_need_no_timing_capability_but_refuse_invented_ranges() {
    let scope = scope(config(false), Operation::Transcribe);
    let mut fake = Fake::new(scope.provider.clone(), TEXT);
    let mut segment = raw_result("e\u{302}\u{301}中");
    segment.range = None;
    for token in &mut segment.raw_tokens {
        token.range = None;
    }
    fake.result = vec![segment.clone()];
    fake.untimed = true;
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
    assert_eq!(result.segments()[0].raw_tokens, segment.raw_tokens);
    assert!(matches!(
        result.to_transcript(),
        Err(Error::TimingUnavailable)
    ));
    let roundtrip: RecognitionSegment =
        serde_json::from_str(&serde_json::to_string(&result.segments()[0]).unwrap()).unwrap();
    assert_eq!(roundtrip, segment);
    let mut align_scope = scope.clone();
    align_scope.operation = Operation::Align;
    align_scope.texts = vec![segment.text.clone()];
    let mut aligner = Fake::new(align_scope.provider.clone(), TEXT);
    let mut aligned = segment.clone();
    aligned.range = Some(scope.source_samples);
    for token in &mut aligned.raw_tokens {
        token.provenance = Provenance::Forced;
    }
    // The aligner supplies ordinal-preserving full ranges rather than native
    // start points being extended into guessed ends.
    for (i, token) in aligned.raw_tokens.iter_mut().enumerate() {
        token.range =
            Some(SampleRange::new(16000 + i as u64 * 100, 16100 + i as u64 * 100).unwrap());
    }
    aligner.result = vec![aligned];
    let timed = transcribe(
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
    let transcript = timed.to_transcript().unwrap();
    let spans = transcript.segments[0].spans.as_ref().unwrap();
    assert_eq!(spans.raw(), "e\u{302}\u{301}中");
    assert_eq!(spans.spans().len(), segment.raw_tokens.len());
    assert!(transcript.segments[0].words.is_empty());
    for (i, span) in spans.spans().iter().enumerate() {
        assert_eq!(span.input().index, i);
        assert_eq!(span.input().provenance, Provenance::Forced);
        assert!(span.input().range.is_some());
    }
    let roundtrip: captions::Transcript =
        serde_json::from_str(&serde_json::to_string(&transcript).unwrap()).unwrap();
    assert_eq!(roundtrip, transcript);
    fake.result[0].raw_tokens[0].range = Some(SampleRange::new(16000, 16100).unwrap());
    assert!(
        transcribe(
            &fake,
            audio(&pcm, &scope),
            &Request {
                scope: &scope,
                consent: None
            },
            None,
            &Hooks::new()
        )
        .is_err()
    );
    fake.result[0].range = Some(scope.source_samples);
    assert!(
        transcribe(
            &fake,
            audio(&pcm, &scope),
            &Request {
                scope: &scope,
                consent: None
            },
            None,
            &Hooks::new()
        )
        .is_err()
    );
}

#[test]
fn separate_alignment_rejects_a_different_recognition_window_or_sample_rate() {
    let source = scope(config(false), Operation::Transcribe);
    let fake = Fake::new(source.provider.clone(), TEXT);
    let pcm = vec![0.0; 32000];
    let recognition = transcribe(
        &fake,
        audio(&pcm, &source),
        &Request {
            scope: &source,
            consent: None,
        },
        None,
        &Hooks::new(),
    )
    .unwrap();
    for (range, rate) in [
        (SampleRange::new(0, 32000).unwrap(), 16000),
        (source.source_samples, 32000),
    ] {
        let mut target = source.clone();
        target.operation = Operation::Align;
        target.source_samples = range;
        target.sample_rate = rate;
        let mut aligner = Fake::new(target.provider.clone(), TEXT);
        aligner.result = forced();
        let result = align(
            recognition.clone(),
            &aligner,
            audio(&pcm, &target),
            &target,
            None,
            TimingUnit::Word,
            &Hooks::new(),
        );
        assert!(
            matches!(result, Err(Error::Invalid("alignment request mismatch"))),
            "mismatched recognition coordinates were accepted: {result:?}"
        );
        assert_eq!(aligner.calls.load(Ordering::Relaxed), 0);
    }
}

//! Remote model identity, price acknowledgement and explicit catalog requests.
#![allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "offline provider fixtures"
)]
use ai::transport::{ApiKey, HttpRequest, HttpResponse, HttpTransport};
use ai::{self, *};
use captions::LanguageTag;
use std::sync::{Arc, Mutex};
use time::{SourceRange, TICKS_PER_SECOND, Tick};

struct Hooks;
impl ExecutionHooks for Hooks {
    fn cancelled(&self) -> bool {
        false
    }
    fn stage(&self, _: Stage) {}
}
struct Transport {
    requests: Mutex<Vec<HttpRequest>>,
    status: u16,
    body: Vec<u8>,
}
impl HttpTransport for Transport {
    fn send(&self, request: HttpRequest) -> Result<HttpResponse, Error> {
        self.requests.lock().unwrap().push(request);
        Ok(HttpResponse {
            status: self.status,
            headers: vec![],
            body: self.body.clone(),
        })
    }
}
enum Adapter {
    Audio(Box<dyn Transcriber>),
    Translation(Box<dyn Translator>),
    Clips(Box<dyn clips::ClipSuggester>),
    Chapters(Box<dyn chapters::ChapterGenerator>),
    Fillers(Box<dyn fillers::FillerClassifier>),
    Speech(Box<dyn speech::SpeechSynthesizer>),
}
fn adapter(
    provider: &str,
    operation: Operation,
    model: &str,
    transport: Arc<Transport>,
) -> Adapter {
    let key = || ApiKey::new("synthetic").unwrap();
    match (provider, operation) {
        ("openai", Operation::Transcribe) => Adapter::Audio(Box::new(
            openai::OpenAiTranscriber::new(
                openai::OpenAiTranscriber::provider_config(model).unwrap(),
                key(),
                transport,
            )
            .unwrap(),
        )),
        ("elevenlabs", Operation::Transcribe) => Adapter::Audio(Box::new(
            elevenlabs::ElevenLabsTranscriber::new(
                elevenlabs::ElevenLabsTranscriber::provider_config(model).unwrap(),
                key(),
                transport,
            )
            .unwrap(),
        )),
        ("openai", Operation::Translate) => Adapter::Translation(Box::new(
            openai::OpenAiTranslator::new(
                openai::OpenAiTranslator::provider_config(model).unwrap(),
                key(),
                transport,
            )
            .unwrap(),
        )),
        ("anthropic", Operation::Translate) => Adapter::Translation(Box::new(
            anthropic::AnthropicTranslator::new(
                anthropic::AnthropicTranslator::provider_config(model).unwrap(),
                key(),
                transport,
            )
            .unwrap(),
        )),
        ("openai", Operation::SuggestClips) => Adapter::Clips(Box::new(
            clips::OpenAiClipSuggester::new(
                clips::OpenAiClipSuggester::provider_config(model).unwrap(),
                key(),
                transport,
            )
            .unwrap(),
        )),
        ("anthropic", Operation::SuggestClips) => Adapter::Clips(Box::new(
            clips::AnthropicClipSuggester::new(
                clips::AnthropicClipSuggester::provider_config(model).unwrap(),
                key(),
                transport,
            )
            .unwrap(),
        )),
        ("openai", Operation::GenerateChapters) => Adapter::Chapters(Box::new(
            chapters::OpenAiChapterGenerator::new(
                chapters::OpenAiChapterGenerator::provider_config(model).unwrap(),
                key(),
                transport,
            )
            .unwrap(),
        )),
        ("anthropic", Operation::GenerateChapters) => Adapter::Chapters(Box::new(
            chapters::AnthropicChapterGenerator::new(
                chapters::AnthropicChapterGenerator::provider_config(model).unwrap(),
                key(),
                transport,
            )
            .unwrap(),
        )),
        ("openai", Operation::ClassifyFillers) => Adapter::Fillers(Box::new(
            fillers::OpenAiFillerClassifier::new(
                fillers::OpenAiFillerClassifier::provider_config(model).unwrap(),
                key(),
                transport,
            )
            .unwrap(),
        )),
        ("anthropic", Operation::ClassifyFillers) => Adapter::Fillers(Box::new(
            fillers::AnthropicFillerClassifier::new(
                fillers::AnthropicFillerClassifier::provider_config(model).unwrap(),
                key(),
                transport,
            )
            .unwrap(),
        )),
        ("openai", Operation::Synthesize) => Adapter::Speech(Box::new(
            openai::OpenAiSpeechSynthesizer::new(
                openai::OpenAiSpeechSynthesizer::provider_config(model).unwrap(),
                key(),
                transport,
            )
            .unwrap(),
        )),
        ("elevenlabs", Operation::Synthesize) => {
            let settings = elevenlabs::speech::ElevenLabsSpeechSettings::default();
            Adapter::Speech(Box::new(
                elevenlabs::speech::ElevenLabsSpeech::new(
                    elevenlabs::speech::ElevenLabsSpeech::provider_config(model, &settings)
                        .unwrap(),
                    key(),
                    transport,
                    settings,
                    vec![speech::SpeechVoice {
                        id: "stock-fixture".into(),
                        category: speech::VoiceCategory::Stock,
                    }],
                )
                .unwrap(),
            ))
        }
        _ => panic!("unsupported fixture pair"),
    }
}
fn language() -> LanguageTag {
    LanguageTag::parse("en").unwrap()
}
fn range() -> SourceRange {
    SourceRange::new(Tick::ZERO, Tick(TICKS_PER_SECOND * 4)).unwrap()
}
fn consent(scope: &Scope, acknowledged: bool) -> Consent {
    if acknowledged {
        Consent::grant_unknown_price(
            scope.clone(),
            &UnknownPriceAcknowledgement::grant(scope.provider.clone(), scope.operation),
        )
        .unwrap()
    } else {
        Consent::grant(scope.clone(), Money::new("USD", 1_000_000).unwrap()).unwrap()
    }
}
impl Adapter {
    fn invoke(&self, acknowledged: bool) -> Result<(), Error> {
        let window = TranslationWindow {
            source_language: language(),
            target_language: LanguageTag::parse("vi").unwrap(),
            segments: vec![TranslationSegment {
                id: "segment".into(),
                text: "Hello".into(),
            }],
            before: vec![],
            after: vec![],
            glossary: vec![],
            style: "subtitle".into(),
        };
        match self {
            Self::Audio(p) => {
                let scope = Scope::transcription(
                    p.config().clone(),
                    "file".into(),
                    language(),
                    SampleRange::new(0, 16_000).unwrap(),
                    16_000,
                    "audio".into(),
                )
                .unwrap();
                assert_eq!(p.estimate(&scope), CostEstimate::Unknown);
                let consent = consent(&scope, acknowledged);
                p.transcribe(
                    AudioWindow {
                        samples: &vec![0.1; 16_000],
                        range: scope.source_samples,
                        sample_rate: 16_000,
                    },
                    &Request {
                        scope: &scope,
                        consent: Some(&consent),
                    },
                    &Hooks,
                )
                .map(|_| ())
            }
            Self::Translation(p) => {
                let scope = Scope::translation(
                    p.config().clone(),
                    "context-window".into(),
                    &window,
                    "text".into(),
                )
                .unwrap();
                assert_eq!(p.estimate(&scope), CostEstimate::Unknown);
                let consent = consent(&scope, acknowledged);
                p.translate(
                    &window,
                    &Request {
                        scope: &scope,
                        consent: Some(&consent),
                    },
                    &Hooks,
                )
                .map(|_| ())
            }
            Self::Clips(p) => {
                let input = clips::ClipInput {
                    segments: vec![clips::ClipSegment {
                        id: "segment".into(),
                        text: "Hello".into(),
                        range: range(),
                    }],
                    settings: clips::ClipSettings {
                        count: 1,
                        min_duration: Tick(TICKS_PER_SECOND),
                        max_duration: Tick(TICKS_PER_SECOND * 10),
                        min_segment_duration: Tick(TICKS_PER_SECOND),
                        max_segments: 4,
                        overlap_tolerance: 0.2,
                        language: language(),
                        keywords: vec![],
                    },
                };
                let scope =
                    Scope::clip_suggestion(p.config().clone(), &input, "text".into()).unwrap();
                assert_eq!(p.estimate(&scope), CostEstimate::Unknown);
                let consent = consent(&scope, acknowledged);
                p.suggest(
                    &input,
                    &Request {
                        scope: &scope,
                        consent: Some(&consent),
                    },
                    &Hooks,
                )
                .map(|_| ())
            }
            Self::Chapters(p) => {
                let input = chapters::ChapterInput {
                    segments: vec![chapters::ChapterSegment {
                        id: "segment".into(),
                        text: "Hello".into(),
                        range: range(),
                    }],
                    settings: chapters::ChapterSettings {
                        language: language(),
                        max_chapters: 1,
                        duration: Tick(TICKS_PER_SECOND * 4),
                    },
                };
                let scope = Scope::chapters(p.config().clone(), &input, "text".into()).unwrap();
                assert_eq!(p.estimate(&scope), CostEstimate::Unknown);
                let consent = consent(&scope, acknowledged);
                p.propose(
                    &input,
                    &Request {
                        scope: &scope,
                        consent: Some(&consent),
                    },
                    &Hooks,
                )
                .map(|_| ())
            }
            Self::Fillers(p) => {
                let input = fillers::FillerInput {
                    segments: vec![fillers::FillerSegment {
                        id: "segment".into(),
                        text: "Um".into(),
                        words: vec![fillers::FillerWord {
                            id: "word".into(),
                            text: "Um".into(),
                        }],
                    }],
                    settings: fillers::FillerSettings {
                        language: language(),
                        include_markers: false,
                    },
                };
                let scope = Scope::fillers(p.config().clone(), &input, "text".into()).unwrap();
                assert_eq!(p.estimate(&scope), CostEstimate::Unknown);
                let consent = consent(&scope, acknowledged);
                p.classify(
                    &input,
                    &Request {
                        scope: &scope,
                        consent: Some(&consent),
                    },
                    &Hooks,
                )
                .map(|_| ())
            }
            Self::Speech(p) => {
                let elevenlabs = p.config().id().as_str() == "elevenlabs";
                let request = speech::SpeechRequest::new(
                    p.config().clone(),
                    "Hello".into(),
                    if elevenlabs { "stock-fixture" } else { "alloy" }.into(),
                    speech::SpeechLanguage {
                        language: language(),
                        script: "Latn".into(),
                    },
                    speech::SpeechOutput {
                        format: if elevenlabs {
                            speech::SpeechFormat::Pcm16
                        } else {
                            speech::SpeechFormat::WavPcm16
                        },
                        sample_rate: 24_000,
                    },
                    1_000_000,
                )
                .unwrap();
                let caps = p.capabilities(&request);
                let estimate = p.estimate(&request);
                assert_eq!(estimate, speech::SynthesisEstimate::Unknown);
                let consent = if acknowledged {
                    speech::SynthesisConsent::grant_unknown_price(
                        &request,
                        &caps,
                        &UnknownPriceAcknowledgement::grant(
                            p.config().clone(),
                            Operation::Synthesize,
                        ),
                    )?
                } else {
                    speech::SynthesisConsent::grant(
                        &request,
                        &caps,
                        estimate,
                        Money::new("USD", 1_000_000).unwrap(),
                    )?
                };
                p.synthesize(&request, &consent, &Hooks).map(|_| ())
            }
        }
    }
}

#[test]
fn every_remote_feature_sends_custom_model_only_after_scoped_unknown_price_acknowledgement() {
    let pairs = [
        ("openai", Operation::Transcribe),
        ("elevenlabs", Operation::Transcribe),
        ("openai", Operation::Translate),
        ("anthropic", Operation::Translate),
        ("openai", Operation::SuggestClips),
        ("anthropic", Operation::SuggestClips),
        ("openai", Operation::GenerateChapters),
        ("anthropic", Operation::GenerateChapters),
        ("openai", Operation::ClassifyFillers),
        ("anthropic", Operation::ClassifyFillers),
        ("openai", Operation::Synthesize),
        ("elevenlabs", Operation::Synthesize),
    ];
    for (provider, operation) in pairs {
        let model = format!("fixture-{provider}-{operation:?}");
        let response: &[u8] = match provider {
            "anthropic" => include_bytes!("../fixtures/remote-models/anthropic-unavailable.json"),
            "elevenlabs" => include_bytes!("../fixtures/remote-models/elevenlabs-unavailable.json"),
            _ => include_bytes!("../fixtures/remote-models/openai-unavailable.json"),
        };
        let transport = Arc::new(Transport {
            requests: Mutex::new(vec![]),
            status: 404,
            body: response.to_vec(),
        });
        let adapter = adapter(provider, operation, &model, transport.clone());
        assert!(
            matches!(adapter.invoke(false), Err(Error::Cost)),
            "{provider}/{operation:?}"
        );
        assert!(transport.requests.lock().unwrap().is_empty());
        let error = adapter.invoke(true).unwrap_err();
        assert!(
            matches!(&error, Error::ModelUnavailable { provider:p, model:m, .. } if p.as_str()==provider && m.as_str()==model),
            "{provider}/{operation:?}: {error:?}"
        );
        assert!(!format!("{error}").contains("synthetic-sensitive-body"));
        let requests = transport.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        if let Ok(body) = serde_json::from_slice::<serde_json::Value>(&requests[0].body) {
            assert_eq!(
                body["model_id"].as_str().or_else(|| body["model"].as_str()),
                Some(model.as_str())
            );
        } else {
            assert!(
                requests[0]
                    .body
                    .windows(model.len())
                    .any(|bytes| bytes == model.as_bytes())
            );
        }
    }
}

#[test]
fn refresh_is_one_bounded_content_free_request_and_keeps_empty_lists_explicit() {
    for provider in ["openai", "anthropic", "elevenlabs"] {
        let body = if provider == "elevenlabs" {
            br#"[{"model_id":"fixture-b"},{"model_id":"fixture-a"},{"model_id":"fixture-a"}]"#
                .to_vec()
        } else {
            br#"{"data":[{"id":"fixture-b"},{"id":"fixture-a"},{"id":"fixture-a"}]}"#.to_vec()
        };
        let transport = Transport {
            requests: Mutex::new(vec![]),
            status: 200,
            body,
        };
        let models = ai::list_remote_models(
            provider,
            &ApiKey::new("synthetic").unwrap(),
            &transport,
            &Hooks,
        )
        .unwrap();
        assert_eq!(
            models.iter().map(|id| id.as_str()).collect::<Vec<_>>(),
            ["fixture-a", "fixture-b"]
        );
        let requests = transport.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].method, "GET");
        assert!(requests[0].body.is_empty());
        assert!(!format!("{:?}", requests[0]).contains("synthetic"));
    }
    let empty = Transport {
        requests: Mutex::new(vec![]),
        status: 200,
        body: br#"{"data":[]}"#.to_vec(),
    };
    assert!(
        ai::list_remote_models("openai", &ApiKey::new("synthetic").unwrap(), &empty, &Hooks)
            .unwrap()
            .is_empty()
    );
}
#[test]
fn unknown_price_acknowledgement_is_model_and_operation_bound_and_ids_have_byte_limits() {
    let config = openai::OpenAiTranslator::provider_config("fixture-a").unwrap();
    let other = openai::OpenAiTranslator::provider_config("fixture-b").unwrap();
    assert_ne!(config.settings_fingerprint(), other.settings_fingerprint());
    let window = TranslationWindow {
        source_language: language(),
        target_language: LanguageTag::parse("vi").unwrap(),
        segments: vec![TranslationSegment {
            id: "one".into(),
            text: "Hello".into(),
        }],
        before: vec![],
        after: vec![],
        glossary: vec![],
        style: "subtitle".into(),
    };
    let scope = Scope::translation(
        config.clone(),
        "context-window".into(),
        &window,
        "text".into(),
    )
    .unwrap();
    for ack in [
        UnknownPriceAcknowledgement::grant(other, Operation::Translate),
        UnknownPriceAcknowledgement::grant(config.clone(), Operation::Transcribe),
    ] {
        assert!(matches!(
            Consent::grant_unknown_price(scope.clone(), &ack),
            Err(Error::ConsentRequired)
        ));
    }
    let ack = UnknownPriceAcknowledgement::grant(config, Operation::Translate);
    assert!(Consent::grant_unknown_price(scope, &ack).is_ok());
    for invalid in [
        "".to_owned(),
        " ".into(),
        "bad\nmodel".into(),
        "x".repeat(defaults::MAX_MODEL_BYTES + 1),
    ] {
        assert!(openai::OpenAiTranslator::provider_config(&invalid).is_err());
    }
    assert!(
        openai::OpenAiTranslator::provider_config(&"x".repeat(defaults::MAX_MODEL_BYTES)).is_ok()
    );
}
#[test]
fn refresh_rejects_oversized_malformed_and_invalid_ids_without_provider_body_leakage() {
    let key = ApiKey::new("synthetic").unwrap();
    for body in [b"synthetic-sensitive-body".to_vec(),br#"{"data":[{"id":""}]}"#.to_vec(),serde_json::to_vec(&serde_json::json!({"data":vec![serde_json::json!({"id":"fixture"});MAX_REMOTE_MODELS+1]})).unwrap()] {
        let transport=Transport {requests:Mutex::new(vec![]),status:200,body};
        let error=list_remote_models("openai",&key,&transport,&Hooks).unwrap_err();
        assert!(matches!(error,Error::Schema|Error::Payload));assert!(!error.to_string().contains("synthetic-sensitive-body"));
    }
    let transport = Transport {
        requests: Mutex::new(vec![]),
        status: 401,
        body: br#"{"error":{"message":"synthetic-sensitive-body"}}"#.to_vec(),
    };
    assert!(matches!(
        list_remote_models("openai", &key, &transport, &Hooks),
        Err(Error::Auth)
    ));
    struct Cancel;
    impl ExecutionHooks for Cancel {
        fn cancelled(&self) -> bool {
            true
        }
        fn stage(&self, _: Stage) {}
    }
    let before = transport.requests.lock().unwrap().len();
    assert!(matches!(
        list_remote_models("openai", &key, &transport, &Cancel),
        Err(Error::Cancelled)
    ));
    assert_eq!(transport.requests.lock().unwrap().len(), before);
}

#[test]
fn missing_required_model_capability_is_typed_and_redacted() {
    let transport = Arc::new(Transport {
        requests: Mutex::new(vec![]),
        status: 400,
        body: include_bytes!("../fixtures/remote-models/unsupported-capability.json").to_vec(),
    });
    let adapter = adapter(
        "openai",
        Operation::GenerateChapters,
        "fixture-incompatible",
        transport.clone(),
    );
    let error = adapter.invoke(true).unwrap_err();
    assert!(matches!(
        error,
        Error::ModelUnavailable {
            message: "required capability is not supported",
            ..
        }
    ));
    assert!(!error.to_string().contains("synthetic-sensitive-body"));
    assert_eq!(transport.requests.lock().unwrap().len(), 1);
}

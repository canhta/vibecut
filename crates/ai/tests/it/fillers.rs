#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test fixtures"
)]
use ai::{
    Consent, CostEstimate, Error, ExecutionHooks, Location, Money, Operation, ProviderConfig,
    Request, SampleRange, Scope, Stage,
    fillers::*,
    transport::{ApiKey, HttpRequest, HttpResponse, HttpTransport},
};
use captions::{LanguageTag, NfcText};
use serde_json::{Value, json};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

#[derive(Default)]
struct Hooks(AtomicBool);
impl ExecutionHooks for Hooks {
    fn cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
    fn stage(&self, _: Stage) {}
}

fn input(include_markers: bool) -> FillerInput {
    FillerInput {
        segments: vec![FillerSegment {
            id: "4:cue-1".into(),
            text: "Um brave".into(),
            words: vec![
                FillerWord {
                    id: "filler:4:cue-1:0".into(),
                    text: "Um".into(),
                },
                FillerWord {
                    id: "filler:4:cue-1:100".into(),
                    text: "brave".into(),
                },
            ],
        }],
        settings: FillerSettings {
            language: LanguageTag::parse("en").unwrap(),
            include_markers,
        },
    }
}
fn config(anthropic: bool) -> ProviderConfig {
    ProviderConfig::new(
        "fillers".into(),
        if anthropic {
            ai::anthropic::MODEL
        } else {
            OPENAI_MODEL
        }
        .into(),
        if anthropic {
            ANTHROPIC_SETTINGS
        } else {
            OPENAI_SETTINGS
        }
        .into(),
        Location::Remote {
            backend: if anthropic { "anthropic" } else { "openai" }.into(),
            base_url: if anthropic {
                ai::anthropic::BASE_URL
            } else {
                ai::openai::BASE_URL
            }
            .into(),
            auth_ref: "secret-store".into(),
        },
    )
    .unwrap()
}
fn scope(w: &FillerInput, p: &dyn FillerClassifier) -> Scope {
    Scope {
        provider: p.config().clone(),
        operation: Operation::ClassifyFillers,
        payload_fingerprint: "synthetic".into(),
        mode: MODE.into(),
        texts: vec![w.consent_text().unwrap()],
        source_language: w.settings.language.clone(),
        target_language: None,
        source_samples: SampleRange::new(0, 1).unwrap(),
        sample_rate: 1,
        max_bytes: MAX_WIRE,
    }
}
struct Transport {
    response: Mutex<Option<HttpResponse>>,
    calls: AtomicUsize,
    requests: Mutex<Vec<HttpRequest>>,
}
impl HttpTransport for Transport {
    fn send(&self, r: HttpRequest) -> Result<HttpResponse, Error> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        self.requests.lock().unwrap().push(r);
        Ok(self.response.lock().unwrap().take().unwrap())
    }
}
fn envelope(anthropic: bool, body: Value) -> HttpResponse {
    let text = body.to_string();
    HttpResponse {
        status: 200,
        headers: vec![],
        body: serde_json::to_vec(&if anthropic {
            json!({"type":"message","role":"assistant","stop_reason":"end_turn","content":[{"type":"text","text":text}]})
        } else {
            json!({"status":"completed","output":[{"type":"message","role":"assistant","status":"completed","content":[{"type":"output_text","text":text}]}]})
        })
        .unwrap(),
    }
}
fn remote(anthropic: bool, body: Value) -> (Box<dyn FillerClassifier>, Arc<Transport>) {
    let transport = Arc::new(Transport {
        response: Mutex::new(Some(envelope(anthropic, body))),
        calls: AtomicUsize::new(0),
        requests: Mutex::new(vec![]),
    });
    let key = ApiKey::new("synthetic-key").unwrap();
    let generator: Box<dyn FillerClassifier> = if anthropic {
        Box::new(AnthropicFillerClassifier::new(config(true), key, transport.clone()).unwrap())
    } else {
        Box::new(OpenAiFillerClassifier::new(config(false), key, transport.clone()).unwrap())
    };
    (generator, transport)
}
fn run(p: &dyn FillerClassifier, w: &FillerInput) -> Result<Vec<FillerFinding>, Error> {
    let scope = scope(w, p);
    let consent = Consent::grant(scope.clone(), Money::new("USD", 1_000_000).unwrap()).unwrap();
    classify_fillers(
        p,
        w,
        &Request {
            scope: &scope,
            consent: Some(&consent),
        },
        &Hooks::default(),
    )
}

#[test]
fn exact_ids_resolve_and_unknown_duplicate_marker_ids_refuse() {
    let w = input(false);
    let findings = w
        .resolve(FillerProposal {
            fillers: vec![FillerFinding {
                id: "filler:4:cue-1:100".into(),
                kind: captions::FillerKind::FilledPause,
            }],
        })
        .unwrap();
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].id.as_str(), "filler:4:cue-1:100");
    for fillers in [
        vec![FillerFinding {
            id: "filler:4:cue-1:999".into(),
            kind: captions::FillerKind::FilledPause,
        }],
        vec![
            FillerFinding {
                id: "filler:4:cue-1:0".into(),
                kind: captions::FillerKind::FilledPause,
            },
            FillerFinding {
                id: "filler:4:cue-1:0".into(),
                kind: captions::FillerKind::FilledPause,
            },
        ],
        vec![FillerFinding {
            id: "filler:4:cue-1:0".into(),
            kind: captions::FillerKind::DiscourseMarker,
        }],
    ] {
        assert_eq!(
            w.resolve(FillerProposal { fillers }).unwrap_err(),
            FillerRefusal::InvalidSegments
        );
    }
    let markers = input(true);
    assert!(
        markers
            .resolve(FillerProposal {
                fillers: vec![FillerFinding {
                    id: "filler:4:cue-1:0".into(),
                    kind: captions::FillerKind::DiscourseMarker,
                }],
            })
            .is_ok()
    );
}

#[test]
fn empty_unsupported_duplicate_and_oversized_inputs_refuse_before_consent() {
    let mut empty = input(false);
    empty.segments.clear();
    assert_eq!(
        empty.validate().unwrap_err(),
        FillerRefusal::EmptyTranscript
    );
    let mut unsupported = input(false);
    unsupported.settings.language = LanguageTag::parse("fr").unwrap();
    assert_eq!(
        unsupported.validate().unwrap_err(),
        FillerRefusal::UnsupportedLanguage
    );
    let mut duplicate = input(false);
    duplicate.segments[0].words[1].id = duplicate.segments[0].words[0].id.clone();
    assert_eq!(
        duplicate.validate().unwrap_err(),
        FillerRefusal::InvalidSegments
    );
    let mut blank = input(false);
    blank.segments[0].words[0].text = "   ".into();
    assert_eq!(
        blank.validate().unwrap_err(),
        FillerRefusal::InvalidSegments
    );
    let mut oversized = input(false);
    oversized.segments[0].text = NfcText::from("x".repeat(MAX_INPUT_BYTES + 1));
    assert_eq!(oversized.validate().unwrap_err(), FillerRefusal::Payload);
}

#[test]
fn consent_cost_and_wire_ceiling_precede_io() {
    for anthropic in [false, true] {
        let (p, t) = remote(
            anthropic,
            json!({"fillers":[{"id":"filler:4:cue-1:100","kind":"filled_pause"}]}),
        );
        let w = input(false);
        let scope = scope(&w, p.as_ref());
        assert!(matches!(
            classify_fillers(
                p.as_ref(),
                &w,
                &Request {
                    scope: &scope,
                    consent: None,
                },
                &Hooks::default(),
            ),
            Err(Error::ConsentRequired)
        ));
        let mut changed = w.clone();
        changed.segments[0].words[0].text = "Changed".into();
        let consent = Consent::grant(scope.clone(), Money::new("USD", 1_000_000).unwrap()).unwrap();
        assert!(matches!(
            classify_fillers(
                p.as_ref(),
                &changed,
                &Request {
                    scope: &scope,
                    consent: Some(&consent),
                },
                &Hooks::default(),
            ),
            Err(Error::ConsentRequired)
        ));
        let low = Consent::grant(scope.clone(), Money::new("USD", 0).unwrap()).unwrap();
        assert!(matches!(
            classify_fillers(
                p.as_ref(),
                &w,
                &Request {
                    scope: &scope,
                    consent: Some(&low),
                },
                &Hooks::default(),
            ),
            Err(Error::Cost)
        ));
        assert_eq!(t.calls.load(Ordering::Relaxed), 0);
    }
}

#[test]
fn adapters_return_exact_ids_and_name_their_provider() {
    for anthropic in [false, true] {
        let (p, t) = remote(
            anthropic,
            json!({"fillers":[{"id":"filler:4:cue-1:100","kind":"filled_pause"}]}),
        );
        let findings = run(p.as_ref(), &input(false)).unwrap();
        assert_eq!(findings.len(), 1);
        let requests = t.requests.lock().unwrap();
        let wire: Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(wire["model"], p.config().model().as_str());
        assert!(wire["text"]["format"]["name"] == "fillers" || anthropic);
        if anthropic {
            assert_eq!(requests[0].url, ai::anthropic::ENDPOINT);
            assert_eq!(wire["output_config"]["format"]["type"], "json_schema");
        } else {
            assert_eq!(requests[0].url, ai::openai::TRANSLATION_ENDPOINT);
            assert_eq!(wire["store"], false);
            assert_eq!(wire["text"]["format"]["strict"], true);
        }
        assert!(!format!("{:?}", requests[0]).contains("synthetic-key"));
        drop(requests);
        let disclosure = if anthropic {
            AnthropicFillerClassifier::new(
                config(true),
                ApiKey::new("synthetic-key").unwrap(),
                Arc::new(Transport {
                    response: Mutex::new(None),
                    calls: AtomicUsize::new(0),
                    requests: Mutex::new(vec![]),
                }),
            )
            .unwrap()
            .disclosure(&input(false), &scope(&input(false), p.as_ref()))
        } else {
            OpenAiFillerClassifier::new(
                config(false),
                ApiKey::new("synthetic-key").unwrap(),
                Arc::new(Transport {
                    response: Mutex::new(None),
                    calls: AtomicUsize::new(0),
                    requests: Mutex::new(vec![]),
                }),
            )
            .unwrap()
            .disclosure(&input(false), &scope(&input(false), p.as_ref()))
        }
        .unwrap();
        assert_eq!(
            disclosure.provider,
            if anthropic { "Anthropic" } else { "OpenAI" }
        );
        assert!(matches!(disclosure.estimate, CostEstimate::UpperBound(_)));
    }
}

#[test]
fn an_unknown_returned_id_fails_the_invocation() {
    let (p, _) = remote(
        false,
        json!({"fillers":[{"id":"filler:4:cue-1:999","kind":"filled_pause"}]}),
    );
    assert!(matches!(
        run(p.as_ref(), &input(false)),
        Err(Error::Fillers(FillerRefusal::InvalidSegments))
    ));
}

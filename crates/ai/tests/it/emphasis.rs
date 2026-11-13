#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test fixtures"
)]
use ai::foundation::transport::ApiKey;
use ai::text::emphasis::*;
use ai::{
    Consent, CostEstimate, Error, ExecutionHooks, Money, Operation, ProviderConfig, Request,
    SampleRange, Scope, Stage,
};
use captions::{LanguageTag, NfcText};
use serde_json::{Value, json};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use transport::{HttpRequest, HttpResponse, HttpTransport};

#[derive(Default)]
struct Hooks(AtomicBool);
impl ExecutionHooks for Hooks {
    fn cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
    fn stage(&self, _: Stage) {}
}

fn input() -> EmphasisInput {
    let words = |text: &str| {
        text.split(' ')
            .enumerate()
            .map(|(position, word)| EmphasisWord {
                position: position as u32,
                text: word.into(),
            })
            .collect()
    };
    EmphasisInput {
        segments: vec![
            EmphasisSegment {
                id: "4:cue-1".into(),
                text: "Quarter revenue grew 40 percent".into(),
                words: words("Quarter revenue grew 40 percent"),
            },
            EmphasisSegment {
                id: "4:cue-2".into(),
                text: "and that is all".into(),
                words: words("and that is all"),
            },
        ],
        settings: EmphasisSettings {
            language: LanguageTag::parse("en").unwrap(),
            max_per_cue: 3,
        },
    }
}
fn config(anthropic: bool) -> ProviderConfig {
    if anthropic {
        AnthropicEmphasisSelector::provider_config("claude-sonnet-5-5").unwrap()
    } else {
        OpenAiEmphasisSelector::provider_config("gpt-6-luna").unwrap()
    }
}
fn scope(w: &EmphasisInput, p: &dyn EmphasisSelector) -> Scope {
    Scope {
        provider: p.config().clone(),
        operation: Operation::SelectEmphasis,
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
/// A recorded provider reply, or a recorded transport failure.
struct Transport {
    response: Mutex<Option<Result<HttpResponse, transport::Error>>>,
    calls: AtomicUsize,
    requests: Mutex<Vec<HttpRequest>>,
}
impl HttpTransport for Transport {
    fn send(&self, r: HttpRequest) -> Result<HttpResponse, transport::Error> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        self.requests.lock().unwrap().push(r);
        self.response.lock().unwrap().take().unwrap()
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
fn remote_with(
    anthropic: bool,
    response: Result<HttpResponse, transport::Error>,
) -> (Box<dyn EmphasisSelector>, Arc<Transport>) {
    let transport = Arc::new(Transport {
        response: Mutex::new(Some(response)),
        calls: AtomicUsize::new(0),
        requests: Mutex::new(vec![]),
    });
    let key = ApiKey::new("synthetic-key").unwrap();
    let selector: Box<dyn EmphasisSelector> = if anthropic {
        Box::new(AnthropicEmphasisSelector::new(config(true), key, transport.clone()).unwrap())
    } else {
        Box::new(OpenAiEmphasisSelector::new(config(false), key, transport.clone()).unwrap())
    };
    (selector, transport)
}
fn remote(anthropic: bool, body: Value) -> (Box<dyn EmphasisSelector>, Arc<Transport>) {
    remote_with(anthropic, Ok(envelope(anthropic, body)))
}
fn run(p: &dyn EmphasisSelector, w: &EmphasisInput) -> Result<Vec<EmphasisSelection>, Error> {
    let scope = scope(w, p);
    let consent = Consent::grant(scope.clone(), Money::new("USD", 1_000_000).unwrap()).unwrap();
    select_emphasis(
        p,
        w,
        &Request {
            scope: &scope,
            consent: Some(&consent),
        },
        &Hooks::default(),
    )
}
fn selection(segment: &str, positions: Vec<u32>) -> EmphasisSelection {
    EmphasisSelection {
        segment: segment.into(),
        positions,
    }
}

#[test]
fn exact_positions_resolve_and_unknown_duplicate_or_excess_results_refuse() {
    let w = input();
    let resolved = w
        .resolve(EmphasisProposal {
            selections: vec![selection("4:cue-1", vec![4, 3])],
        })
        .unwrap();
    assert_eq!(resolved, vec![selection("4:cue-1", vec![3, 4])]);
    for selections in [
        vec![selection("9:cue-9", vec![0])],
        vec![selection("4:cue-1", vec![5])],
        vec![selection("4:cue-1", vec![1, 1])],
        vec![selection("4:cue-1", vec![0, 1, 2, 3])],
        vec![selection("4:cue-1", vec![0]), selection("4:cue-1", vec![1])],
        // A position that is a word of another cue is still not a word of this one.
        vec![selection("4:cue-2", vec![4])],
    ] {
        assert_eq!(
            w.resolve(EmphasisProposal { selections }).unwrap_err(),
            EmphasisRefusal::InvalidSegments
        );
    }
}

#[test]
fn empty_unsupported_duplicate_and_oversized_inputs_refuse_before_consent() {
    let mut empty = input();
    empty.segments.clear();
    assert_eq!(
        empty.validate().unwrap_err(),
        EmphasisRefusal::EmptyTranscript
    );
    let mut unsupported = input();
    unsupported.settings.language = LanguageTag::parse("fr").unwrap();
    assert_eq!(
        unsupported.validate().unwrap_err(),
        EmphasisRefusal::UnsupportedLanguage
    );
    let mut duplicate = input();
    duplicate.segments[0].words[1].position = 0;
    assert_eq!(
        duplicate.validate().unwrap_err(),
        EmphasisRefusal::InvalidSegments
    );
    let mut limit = input();
    limit.settings.max_per_cue = 4;
    assert_eq!(
        limit.validate().unwrap_err(),
        EmphasisRefusal::InvalidSegments
    );
    let mut oversized = input();
    oversized.segments[0].text = NfcText::from("x".repeat(MAX_INPUT_BYTES + 1));
    assert_eq!(oversized.validate().unwrap_err(), EmphasisRefusal::Payload);
}

#[test]
fn consent_cost_and_wire_ceiling_precede_io() {
    for anthropic in [false, true] {
        let (p, t) = remote(
            anthropic,
            json!({"selections":[{"segment":"4:cue-1","positions":[3]}]}),
        );
        let w = input();
        let scope = scope(&w, p.as_ref());
        assert!(matches!(
            select_emphasis(
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
            select_emphasis(
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
            select_emphasis(
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
        assert_eq!(
            t.calls.load(Ordering::Relaxed),
            0,
            "no upload without consent"
        );
    }
}

#[test]
fn recorded_replies_resolve_and_the_upload_discloses_its_provider() {
    for anthropic in [false, true] {
        let (p, t) = remote(
            anthropic,
            json!({"selections":[{"segment":"4:cue-1","positions":[4,3]},{"segment":"4:cue-2","positions":[]}]}),
        );
        let selections = run(p.as_ref(), &input()).unwrap();
        assert_eq!(
            selections,
            vec![
                selection("4:cue-1", vec![3, 4]),
                selection("4:cue-2", vec![])
            ]
        );
        let requests = t.requests.lock().unwrap();
        let wire: Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(wire["model"], p.config().model().as_str());
        assert!(wire["text"]["format"]["name"] == "emphasis" || anthropic);
        if anthropic {
            assert_eq!(requests[0].url, ai::providers::anthropic::ENDPOINT);
            assert_eq!(wire["output_config"]["format"]["type"], "json_schema");
        } else {
            assert_eq!(requests[0].url, ai::providers::openai::TRANSLATION_ENDPOINT);
            assert_eq!(wire["store"], false);
            assert_eq!(wire["text"]["format"]["strict"], true);
        }
        assert!(!format!("{:?}", requests[0]).contains("synthetic-key"));
        drop(requests);
        let disclosure = p.disclosure(&scope(&input(), p.as_ref())).unwrap().unwrap();
        assert_eq!(
            disclosure.provider.as_str(),
            if anthropic { "Anthropic" } else { "OpenAI" }
        );
        assert!(matches!(disclosure.estimate, CostEstimate::UpperBound(_)));
        assert!(disclosure.link("policy").is_some());
    }
}

#[test]
fn a_bad_reply_or_a_failed_provider_is_a_typed_failure_never_a_lexicon_result() {
    let (p, _) = remote(
        false,
        json!({"selections":[{"segment":"4:cue-1","positions":[99]}]}),
    );
    assert!(matches!(
        run(p.as_ref(), &input()),
        Err(Error::Emphasis(EmphasisRefusal::InvalidSegments))
    ));
    let (p, _) = remote(false, json!({"unexpected":true}));
    assert!(matches!(run(p.as_ref(), &input()), Err(Error::Schema)));
    for anthropic in [false, true] {
        let (p, t) = remote_with(
            anthropic,
            Ok(HttpResponse {
                status: 500,
                headers: vec![],
                body: b"{}".to_vec(),
            }),
        );
        assert!(run(p.as_ref(), &input()).is_err());
        assert_eq!(
            t.calls.load(Ordering::Relaxed),
            1,
            "one attempt, no retry here"
        );
    }
}

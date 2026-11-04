#![allow(
    clippy::unwrap_used,
    clippy::panic,
    reason = "offline fixtures and assertions"
)]
use ai::{openai::*, *};
use captions::{LanguageTag, NfcText};
use serde_json::{Value, json};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
struct Hooks(AtomicBool);
impl ExecutionHooks for Hooks {
    fn cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
    fn stage(&self, _: Stage) {}
}
struct Transport {
    response: Value,
    calls: AtomicUsize,
    request: Mutex<Option<HttpRequest>>,
    cancel: Option<Arc<Hooks>>,
}
impl HttpTransport for Transport {
    fn send(&self, r: HttpRequest) -> Result<HttpResponse, Error> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        *self.request.lock().unwrap() = Some(r);
        if let Some(h) = &self.cancel {
            h.0.store(true, Ordering::Relaxed);
        }
        Ok(HttpResponse {
            status: 200,
            headers: vec![],
            body: serde_json::to_vec(&self.response).unwrap(),
        })
    }
}
fn lang(s: &str) -> LanguageTag {
    LanguageTag::parse(s).unwrap()
}
fn config() -> ProviderConfig {
    ProviderConfig::new(
        "openai".into(),
        TRANSLATION_MODEL.into(),
        TRANSLATION_SETTINGS.into(),
        Location::Remote {
            backend: "openai".into(),
            base_url: BASE_URL.into(),
            auth_ref: "test".into(),
        },
    )
    .unwrap()
}
fn window(source: &str, target: &str) -> TranslationWindow {
    TranslationWindow {
        source_language: lang(source),
        target_language: lang(target),
        segments: vec![TranslationSegment {
            id: "cue-1".into(),
            text: "VibeCut 12 kg".into(),
        }],
        before: vec!["earlier context".into()],
        after: vec!["later context".into()],
        glossary: vec![GlossaryTerm {
            term: "VibeCut".into(),
            translation: None,
        }],
        style: "concise".into(),
    }
}
fn scope(w: &TranslationWindow) -> Scope {
    Scope {
        provider: config(),
        operation: Operation::Translate,
        payload_fingerprint: "test".into(),
        mode: TRANSLATION_MODE.into(),
        texts: vec![w.consent_text().unwrap()],
        source_language: w.source_language.clone(),
        target_language: Some(w.target_language.clone()),
        source_samples: SampleRange::new(0, 1).unwrap(),
        sample_rate: 1,
        max_bytes: 65536,
    }
}
fn response(text: Value) -> Value {
    json!({"status":"completed","output":[{"type":"message","status":"completed","role":"assistant","content":[{"type":"output_text","text":text.to_string()}]}]})
}
fn provider(body: Value) -> (OpenAiTranslator, Arc<Transport>) {
    let t = Arc::new(Transport {
        response: body,
        calls: AtomicUsize::new(0),
        request: Mutex::new(None),
        cancel: None,
    });
    (
        OpenAiTranslator::new(config(), ApiKey::new("secret-test-key").unwrap(), t.clone())
            .unwrap(),
        t,
    )
}
#[test]
fn all_six_directions_and_both_chinese_scripts_use_strict_contextual_requests() {
    for (source, target) in [
        ("vi", "en"),
        ("vi", "zh-Hans"),
        ("vi", "zh-Hant"),
        ("en", "vi"),
        ("en", "zh-Hans"),
        ("en", "zh-Hant"),
        ("zh", "vi"),
        ("zh", "en"),
    ] {
        let w = window(source, target);
        let s = scope(&w);
        let (p, t) = provider(response(
            json!({"translations":[{"id":"cue-1","text":"VibeCut 12 kg translated"}]}),
        ));
        let c = Consent::grant(s.clone(), Money::new("USD", 1_000_000).unwrap()).unwrap();
        let r = translate(
            &p,
            &w,
            &Request {
                scope: &s,
                consent: Some(&c),
            },
            &Hooks(AtomicBool::new(false)),
        )
        .unwrap();
        assert_eq!(r.source_language, lang(source));
        assert_eq!(r.target_language, lang(target));
        assert!(r.number_warnings.is_empty());
        let guard = t.request.lock().unwrap();
        let req = guard.as_ref().unwrap();
        assert_eq!(req.url, TRANSLATION_ENDPOINT);
        let body: Value = serde_json::from_slice(&req.body).unwrap();
        assert_eq!(body["store"], false);
        assert_eq!(body["background"], false);
        assert_eq!(body["text"]["format"]["strict"], true);
        let input: Value = serde_json::from_str(body["input"].as_str().unwrap()).unwrap();
        assert_eq!(input["target_language"], target);
        assert_eq!(input["before"][0], "earlier context");
        assert_eq!(input["after"][0], "later context");
        assert!(!format!("{req:?}").contains("secret-test-key"));
        assert!(!format!("{w:?}").contains("earlier context"));
        let d = p.disclosure(&s).unwrap();
        assert!(d.retention.contains("30 days"));
        assert!(d.training.contains("not used"));
        assert_eq!(t.calls.load(Ordering::Relaxed), 1);
    }
}
#[test]
fn every_context_glossary_style_id_and_language_change_invalidates_consent() {
    let w = window("vi", "en");
    let s = scope(&w);
    let c = Consent::grant(s.clone(), Money::new("USD", 1_000_000).unwrap()).unwrap();
    let (p, t) = provider(response(
        json!({"translations":[{"id":"cue-1","text":"VibeCut 12 kg"}]}),
    ));
    for change in 0..8 {
        let mut changed = w.clone();
        match change {
            0 => changed.before.push("private".into()),
            1 => changed.after.push("private".into()),
            2 => changed.glossary.clear(),
            3 => changed.style = "other".into(),
            4 => changed.segments.first_mut().unwrap().id = "other".into(),
            5 => changed.segments.first_mut().unwrap().text = "other".into(),
            6 => changed.source_language = lang("en"),
            _ => changed.target_language = lang("zh-Hant"),
        }
        assert!(matches!(
            p.translate(
                &changed,
                &Request {
                    scope: &s,
                    consent: Some(&c)
                },
                &Hooks(AtomicBool::new(false))
            ),
            Err(Error::ConsentRequired)
        ));
    }
    assert!(matches!(
        translate(
            &p,
            &w,
            &Request {
                scope: &s,
                consent: None
            },
            &Hooks(AtomicBool::new(false))
        ),
        Err(Error::ConsentRequired)
    ));
    let low = Consent::grant(s.clone(), Money::new("USD", 0).unwrap()).unwrap();
    assert!(matches!(
        translate(
            &p,
            &w,
            &Request {
                scope: &s,
                consent: Some(&low)
            },
            &Hooks(AtomicBool::new(false))
        ),
        Err(Error::Cost)
    ));
    assert_eq!(t.calls.load(Ordering::Relaxed), 0);
}
#[test]
fn semantic_id_glossary_and_empty_output_failures_are_typed_without_partial_success() {
    let w = window("vi", "en");
    let s = scope(&w);
    let c = Consent::grant(s.clone(), Money::new("USD", 1_000_000).unwrap()).unwrap();
    for entries in [
        json!([]),
        json!([{"id":"cue-1","text":"VibeCut 12"},{"id":"cue-1","text":"VibeCut 12"}]),
        json!([{"id":"other","text":"VibeCut 12"}]),
        json!([{"id":"cue-1","text":" "}]),
        json!([{"id":"cue-1","text":"renamed 12"}]),
        json!([{"id":"cue-1","text":"VibeCut 12","extra":1}]),
    ] {
        let (p, t) = provider(response(json!({"translations":entries})));
        assert!(matches!(
            translate(
                &p,
                &w,
                &Request {
                    scope: &s,
                    consent: Some(&c)
                },
                &Hooks(AtomicBool::new(false))
            ),
            Err(Error::Schema)
        ));
        assert_eq!(t.calls.load(Ordering::Relaxed), 1);
    }
    let (p, _) = provider(response(
        json!({"translations":[{"id":"cue-1","text":"VibeCut 13 kg"}]}),
    ));
    let result = translate(
        &p,
        &w,
        &Request {
            scope: &s,
            consent: Some(&c),
        },
        &Hooks(AtomicBool::new(false)),
    )
    .unwrap();
    assert_eq!(result.number_warnings, vec![NfcText::from("cue-1")]);
    let mut fixed = w.clone();
    fixed.glossary.push(GlossaryTerm {
        term: "kg".into(),
        translation: Some("kilograms".into()),
    });
    assert!(matches!(
        validate_translation(&fixed, &result),
        Err(Error::Schema)
    ));
    let mut valid_fixed = result.clone();
    valid_fixed
        .texts
        .insert("cue-1".into(), "VibeCut 13 kilograms".into());
    assert_eq!(
        validate_translation(&fixed, &valid_fixed).unwrap(),
        vec![NfcText::from("cue-1")]
    );
}
#[test]
fn shared_parser_rejects_refusal_incomplete_malformed_oversize_and_extra_messages() {
    for (body, kind) in [
        (json!({"status":"incomplete","output":[]}), 0),
        (
            json!({"status":"completed","output":[{"type":"message","role":"assistant","status":"completed","content":[{"type":"refusal","refusal":"private"}]}]}),
            1,
        ),
        (response(json!({"translations":[]})), 2),
    ] {
        let r = HttpResponse {
            status: 200,
            headers: vec![],
            body: serde_json::to_vec(&body).unwrap(),
        };
        let parsed = structured::parse::<Value>(&r);
        match kind {
            0 => assert!(matches!(parsed, Err(Error::Incomplete))),
            1 => assert!(matches!(parsed, Err(Error::Refused))),
            _ => assert!(parsed.is_ok()),
        }
    }
    for body in [b"{truncated".to_vec(), vec![b'x'; MAX_RESPONSE_BYTES + 1]] {
        assert!(
            structured::parse::<Value>(&HttpResponse {
                status: 200,
                headers: vec![],
                body
            })
            .is_err()
        );
    }
    let mut extra = response(json!({"translations":[]}));
    let item = extra["output"][0].clone();
    extra["output"].as_array_mut().unwrap().push(item);
    assert!(matches!(
        structured::parse::<Value>(&HttpResponse {
            status: 200,
            headers: vec![],
            body: serde_json::to_vec(&extra).unwrap()
        }),
        Err(Error::Schema)
    ));
    let (p, t) = provider(response(json!({"translations":[]})));
    let w = window("vi", "en");
    let s = scope(&w);
    let c = Consent::grant(s.clone(), Money::new("USD", 1_000_000).unwrap()).unwrap();
    assert!(matches!(
        translate(
            &p,
            &w,
            &Request {
                scope: &s,
                consent: Some(&c)
            },
            &Hooks(AtomicBool::new(true))
        ),
        Err(Error::Cancelled)
    ));
    assert_eq!(t.calls.load(Ordering::Relaxed), 0);
}
#[test]
fn cancellation_after_network_discards_response_and_unsupported_providers_do_not_claim_translation()
{
    let h = Arc::new(Hooks(AtomicBool::new(false)));
    let t = Arc::new(Transport {
        response: response(json!({"translations":[{"id":"cue-1","text":"VibeCut 12 kg"}]})),
        calls: AtomicUsize::new(0),
        request: Mutex::new(None),
        cancel: Some(h.clone()),
    });
    let p = OpenAiTranslator::new(config(), ApiKey::new("test").unwrap(), t.clone()).unwrap();
    let w = window("vi", "en");
    let s = scope(&w);
    let c = Consent::grant(s.clone(), Money::new("USD", 1_000_000).unwrap()).unwrap();
    assert!(matches!(
        translate(
            &p,
            &w,
            &Request {
                scope: &s,
                consent: Some(&c)
            },
            h.as_ref()
        ),
        Err(Error::Cancelled)
    ));
    let mut asr_config = serde_json::to_value(config()).unwrap();
    asr_config["model"] = json!("gpt-transcribe");
    assert!(matches!(
        OpenAiTranslator::new(
            serde_json::from_value(asr_config).unwrap(),
            ApiKey::new("test").unwrap(),
            t
        ),
        Err(Error::Unsupported)
    ));
}

#[test]
fn translation_must_obey_consented_wire_ceiling() {
    let w = window("vi", "en");
    let mut s = scope(&w);
    s.max_bytes = w.consent_text().unwrap().as_str().len() as u64;
    let consent = Consent::grant(s.clone(), Money::new("USD", 1_000_000).unwrap()).unwrap();
    let (p, t) = provider(response(
        json!({"translations":[{"id":"cue-1","text":"VibeCut 12 kg"}]}),
    ));
    let result = translate(
        &p,
        &w,
        &Request {
            scope: &s,
            consent: Some(&consent),
        },
        &Hooks(AtomicBool::new(false)),
    );
    let sent_bytes = t
        .request
        .lock()
        .unwrap()
        .as_ref()
        .map(|r| r.body.len())
        .unwrap_or(0);
    assert!(
        sent_bytes <= s.max_bytes as usize,
        "wire body {sent_bytes} exceeds consent ceiling {}",
        s.max_bytes
    );
    assert!(matches!(result, Err(Error::Payload)));
}
#[test]
fn translation_rejects_nonofficial_processors() {
    for url in [
        "https://third-party.example/v1",
        "https://api.openai.com.evil.example/v1",
        "https://api.openai.com/v2",
        "https://eu.api.openai.com/v1",
    ] {
        let mut cfg = serde_json::to_value(config()).unwrap();
        cfg["location"]["base_url"] = json!(url);
        let t = Arc::new(Transport {
            response: json!({}),
            calls: AtomicUsize::new(0),
            request: Mutex::new(None),
            cancel: None,
        });
        assert!(matches!(
            OpenAiTranslator::new(
                serde_json::from_value(cfg).unwrap(),
                ApiKey::new("test").unwrap(),
                t
            ),
            Err(Error::Unsupported)
        ));
    }
}

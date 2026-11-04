#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "test fixtures"
)]
use ai::{
    Consent, CostEstimate, Error, ExecutionHooks, Location, Money, Operation, ProviderConfig,
    Request, SampleRange, Scope, Stage,
    clips::*,
    transport::{ApiKey, HttpRequest, HttpResponse, HttpTransport, UreqTransport},
};
use captions::LanguageTag;
use serde_json::{Value, json};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use time::{SourceRange, TICKS_PER_SECOND, Tick};
#[derive(Default)]
struct Hooks(AtomicBool);
impl ExecutionHooks for Hooks {
    fn cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
    fn stage(&self, _: Stage) {}
}
fn tick(n: i64) -> Tick {
    Tick(n * TICKS_PER_SECOND)
}
fn input(language: &str, n: usize) -> ClipInput {
    ClipInput {
        segments: (0..n)
            .map(|i| ClipSegment {
                id: format!("s{i}").into(),
                text: match language {
                    "vi" => "Bí quyết tiết kiệm thời gian.",
                    "zh" | "zh-Hans" | "zh-Hant" => "节省时间的方法。",
                    _ => "The secret to saving time.",
                }
                .into(),
                range: SourceRange::new(tick(i as i64 * 5), tick(i as i64 * 5 + 4)).unwrap(),
            })
            .collect(),
        settings: ClipSettings {
            count: 3,
            min_duration: tick(4),
            max_duration: tick(15),
            overlap_tolerance: 1.0,
            language: LanguageTag::parse(language).unwrap(),
            keywords: vec![],
        },
    }
}
fn config(anthropic: bool) -> ProviderConfig {
    ProviderConfig::new(
        "clips".into(),
        if anthropic {
            ai::anthropic::MODEL
        } else {
            ai::openai::TRANSLATION_MODEL
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
fn scope(w: &ClipInput, p: &dyn ClipSuggester) -> Scope {
    Scope {
        provider: p.config().clone(),
        operation: Operation::SuggestClips,
        payload_fingerprint: "synthetic".into(),
        mode: MODE.into(),
        texts: vec![w.consent_text().unwrap()],
        source_language: w.settings.language.clone(),
        target_language: None,
        source_samples: SampleRange::new(0, 1000).unwrap(),
        sample_rate: 1,
        max_bytes: MAX_WIRE,
    }
}
fn proposal(a: &str, b: &str, score: f64) -> Value {
    json!({"start_segment_id":a,"end_segment_id":b,"title":"Time saving","hook":"Try this","reason":"Complete useful thought","score":score})
}
fn response(anthropic: bool, clips: Value) -> HttpResponse {
    let text = json!({"clips":clips}).to_string();
    HttpResponse {status:200,headers:vec![],body:serde_json::to_vec(&if anthropic {json!({"type":"message","role":"assistant","stop_reason":"end_turn","content":[{"type":"text","text":text}]})}else{json!({"status":"completed","output":[{"type":"message","role":"assistant","status":"completed","content":[{"type":"output_text","text":text}]}]})}).unwrap()}
}
struct Transport {
    response: Mutex<Option<HttpResponse>>,
    calls: AtomicUsize,
    requests: Mutex<Vec<HttpRequest>>,
    cancel: Option<Arc<Hooks>>,
    fail: bool,
}
impl HttpTransport for Transport {
    fn send(&self, r: HttpRequest) -> Result<HttpResponse, Error> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        self.requests.lock().unwrap().push(r);
        if let Some(h) = &self.cancel {
            h.0.store(true, Ordering::Relaxed);
        }
        if self.fail {
            return Err(Error::Network);
        }
        Ok(self.response.lock().unwrap().take().unwrap())
    }
}
fn provider(anthropic: bool, body: HttpResponse) -> (Box<dyn ClipSuggester>, Arc<Transport>) {
    let t = Arc::new(Transport {
        response: Mutex::new(Some(body)),
        calls: AtomicUsize::new(0),
        requests: Mutex::new(vec![]),
        cancel: None,
        fail: false,
    });
    (remote(anthropic, t.clone()), t)
}
fn remote(a: bool, t: Arc<dyn HttpTransport>) -> Box<dyn ClipSuggester> {
    let key = ApiKey::new("synthetic-key").unwrap();
    if a {
        Box::new(AnthropicClipSuggester::new(config(a), key, t).unwrap())
    } else {
        Box::new(OpenAiClipSuggester::new(config(a), key, t).unwrap())
    }
}
fn run(p: &dyn ClipSuggester, w: &ClipInput) -> Result<ClipSuggestions, Error> {
    let windows = w.windows()?;
    let scopes: Vec<_> = windows.iter().map(|w| scope(w, p)).collect();
    let consents: Vec<_> = scopes
        .iter()
        .map(|s| Consent::grant(s.clone(), Money::new("USD", 1_000_000).unwrap()).unwrap())
        .collect();
    let requests: Vec<_> = scopes
        .iter()
        .zip(&consents)
        .map(|(s, c)| Request {
            scope: s,
            consent: Some(c),
        })
        .collect();
    suggest_clips(p, w, &requests, &Hooks::default())
}
#[test]
fn remote_contracts_share_id_duration_dedupe_and_wire_validation() {
    for a in [false, true] {
        let w = input("vi", 6);
        let (p, t) = provider(
            a,
            response(
                a,
                json!([
                    proposal("s0", "s1", 0.3),
                    proposal("s0", "s1", 0.9),
                    proposal("s0", "s5", 1.0)
                ]),
            ),
        );
        let r = run(p.as_ref(), &w).unwrap();
        assert_eq!(r.clips.len(), 1);
        assert_eq!(r.clips[0].proposal.score, 0.9);
        assert_eq!(
            r.clips[0].range,
            SourceRange::new(tick(0), tick(9)).unwrap()
        );
        assert_eq!(r.clips[0].source, ClipSource::Llm);
        assert!(
            r.dropped
                .iter()
                .any(|d| d.reason == ClipDropReason::Duration)
        );
        assert!(
            r.dropped
                .iter()
                .any(|d| d.reason == ClipDropReason::Duplicate)
        );
        assert_eq!(t.calls.load(Ordering::Relaxed), 1);
        let requests = t.requests.lock().unwrap();
        let wire: Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(wire["model"], p.config().model().as_str());
        if a {
            assert_eq!(requests[0].url, ai::anthropic::ENDPOINT);
            assert_eq!(wire["output_config"]["format"]["type"], "json_schema");
        } else {
            assert_eq!(requests[0].url, ai::openai::TRANSLATION_ENDPOINT);
            assert_eq!(wire["store"], false);
            assert_eq!(wire["text"]["format"]["strict"], true);
        }
        assert!(!format!("{:?}", requests[0]).contains("synthetic-key"));
    }
}
#[test]
fn hostile_schema_ids_scores_and_timestamps_are_refused() {
    for a in [false, true] {
        for mut v in [
            proposal("missing", "s1", 0.5),
            proposal("s2", "s1", 0.5),
            proposal("s0", "s1", -0.1),
            proposal("s0", "s1", 1.1),
        ] {
            let (p, _) = provider(a, response(a, json!([v.clone()])));
            assert!(matches!(
                run(p.as_ref(), &input("en", 3)),
                Err(Error::Schema)
            ));
            v["start_time"] = json!(0);
            let (p, _) = provider(a, response(a, json!([v])));
            assert!(matches!(
                run(p.as_ref(), &input("en", 3)),
                Err(Error::Schema)
            ));
        }
        for clips in [json!(null), json!([{"score":0.1}]), json!({})] {
            let (p, _) = provider(a, response(a, clips));
            assert!(matches!(
                run(p.as_ref(), &input("en", 3)),
                Err(Error::Schema)
            ));
        }
    }
}
#[test]
fn consent_settings_language_cost_and_encoded_wire_ceiling_precede_io() {
    for a in [false, true] {
        let (p, t) = provider(a, response(a, json!([])));
        let w = input("en", 3);
        let s = scope(&w, p.as_ref());
        let c = Consent::grant(s.clone(), Money::new("USD", 1_000_000).unwrap()).unwrap();
        assert!(matches!(
            suggest_clips(
                p.as_ref(),
                &w,
                &[Request {
                    scope: &s,
                    consent: None
                }],
                &Hooks::default()
            ),
            Err(Error::ConsentRequired)
        ));
        let mut changed = w.clone();
        changed.settings.overlap_tolerance = 0.1;
        assert!(matches!(
            suggest_clips(
                p.as_ref(),
                &changed,
                &[Request {
                    scope: &s,
                    consent: Some(&c)
                }],
                &Hooks::default()
            ),
            Err(Error::ConsentRequired)
        ));
        let low = Consent::grant(s.clone(), Money::new("USD", 0).unwrap()).unwrap();
        assert!(matches!(
            suggest_clips(
                p.as_ref(),
                &w,
                &[Request {
                    scope: &s,
                    consent: Some(&low)
                }],
                &Hooks::default()
            ),
            Err(Error::Cost)
        ));
        let mut tiny = s.clone();
        tiny.max_bytes = w.consent_text().unwrap().as_str().len() as u64;
        let c = Consent::grant(tiny.clone(), Money::new("USD", 1_000_000).unwrap()).unwrap();
        assert!(matches!(
            suggest_clips(
                p.as_ref(),
                &w,
                &[Request {
                    scope: &tiny,
                    consent: Some(&c)
                }],
                &Hooks::default()
            ),
            Err(Error::Payload)
        ));
        assert_eq!(t.calls.load(Ordering::Relaxed), 0);
    }
}
#[test]
fn cancellation_and_network_failure_never_fall_back() {
    for a in [false, true] {
        let t = Arc::new(Transport {
            response: Mutex::new(None),
            calls: AtomicUsize::new(0),
            requests: Mutex::new(vec![]),
            cancel: None,
            fail: true,
        });
        let p = remote(a, t.clone());
        assert!(matches!(
            run(p.as_ref(), &input("en", 3)),
            Err(Error::Network)
        ));
        assert_eq!(t.calls.load(Ordering::Relaxed), 1);
        let h = Arc::new(Hooks::default());
        let t = Arc::new(Transport {
            response: Mutex::new(Some(response(a, json!([])))),
            calls: AtomicUsize::new(0),
            requests: Mutex::new(vec![]),
            cancel: Some(h.clone()),
            fail: false,
        });
        let p = remote(a, t.clone());
        let w = input("en", 3);
        let s = scope(&w, p.as_ref());
        let c = Consent::grant(s.clone(), Money::new("USD", 1_000_000).unwrap()).unwrap();
        assert!(matches!(
            suggest_clips(
                p.as_ref(),
                &w,
                &[Request {
                    scope: &s,
                    consent: Some(&c)
                }],
                h.as_ref()
            ),
            Err(Error::Cancelled)
        ));
        assert_eq!(t.calls.load(Ordering::Relaxed), 1);
    }
}
#[test]
fn heuristic_is_explicit_free_and_multilingual_with_exact_source_edges() {
    for lang in ["en", "vi", "zh", "zh-Hans", "zh-Hant"] {
        let p = HeuristicClipSuggester::new().unwrap();
        let w = input(lang, 5);
        let s = scope(&w, &p);
        assert_eq!(p.estimate(&s), CostEstimate::Free);
        let result = suggest_clips(
            &p,
            &w,
            &[Request {
                scope: &s,
                consent: None,
            }],
            &Hooks::default(),
        )
        .unwrap();
        assert_eq!(result.clips.len(), 3);
        for c in result.clips {
            assert_eq!(c.source, ClipSource::Heuristic);
            assert!(
                w.segments
                    .iter()
                    .any(|s| s.range.start() == c.range.start())
            );
            assert!(w.segments.iter().any(|s| s.range.end() == c.range.end()));
            assert!((tick(4)..=tick(15)).contains(&c.range.duration()));
        }
    }
}
#[test]
fn long_transcript_windows_cover_all_legal_ranges_and_merge() {
    let w = input("en", 300);
    let windows = w.windows().unwrap();
    assert!(windows.len() > 2);
    for start in 0..w.segments.len() {
        for end in start..w.segments.len() {
            if w.segments[end].range.end().0 - w.segments[start].range.start().0
                > w.settings.max_duration.0
            {
                break;
            }
            assert!(
                windows
                    .iter()
                    .any(|v| v.segments.iter().any(|s| s.id == w.segments[start].id)
                        && v.segments.iter().any(|s| s.id == w.segments[end].id))
            );
        }
    }
    let p = HeuristicClipSuggester::new().unwrap();
    let r = run(&p, &w).unwrap();
    assert_eq!(r.clips.len(), 3);
    let mut dense = w.clone();
    dense.settings.max_duration = tick(10000);
    assert!(matches!(dense.windows(), Err(Error::Payload)));
}
#[test]
fn validation_has_bounded_inputs_and_normalizes_nfc() {
    let mut w = input("vi", 2);
    w.segments[0].text = "Vie\u{302}\u{323}t Nam".into();
    assert_eq!(w.segments[0].text.as_str(), "Việt Nam");
    let mut bad = w.clone();
    bad.segments[1].id = bad.segments[0].id.clone();
    assert!(bad.validate().is_err());
    let mut bad = w.clone();
    bad.segments[1].range = bad.segments[0].range;
    assert!(bad.validate().is_err());
    w.settings.overlap_tolerance = f64::NAN;
    assert!(w.validate().is_err());
}
#[test]
fn provider_failures_are_typed() {
    for a in [false, true] {
        for (status, expected) in [
            (401, "auth"),
            (429, "rate"),
            (500, "server"),
            (413, "payload"),
        ] {
            let (p, _) = provider(
                a,
                HttpResponse {
                    status,
                    headers: vec![],
                    body: vec![],
                },
            );
            let e = run(p.as_ref(), &input("en", 3)).unwrap_err();
            assert!(matches!(
                (expected, e),
                ("auth", Error::Auth)
                    | ("rate", Error::RateLimit { .. })
                    | ("server", Error::Server)
                    | ("payload", Error::Payload)
            ));
        }
        let mut r = response(a, json!([]));
        let mut body: Value = serde_json::from_slice(&r.body).unwrap();
        if a {
            body["stop_reason"] = json!("max_tokens");
        } else {
            body["status"] = json!("incomplete");
        }
        r.body = serde_json::to_vec(&body).unwrap();
        let (p, _) = provider(a, r);
        assert!(matches!(
            run(p.as_ref(), &input("en", 3)),
            Err(Error::Incomplete)
        ));
    }
}
#[test]
#[ignore = "requires VIBECUT_LIVE_AI=1 and both API keys; one bounded synthetic request per provider"]
fn live_clip_providers() {
    assert_eq!(std::env::var("VIBECUT_LIVE_AI").as_deref(), Ok("1"));
    for a in [false, true] {
        let key = ApiKey::new(
            std::env::var(if a {
                "ANTHROPIC_API_KEY"
            } else {
                "OPENAI_API_KEY"
            })
            .unwrap(),
        )
        .unwrap();
        let t = Arc::new(UreqTransport::new());
        let p: Box<dyn ClipSuggester> = if a {
            Box::new(AnthropicClipSuggester::new(config(a), key, t).unwrap())
        } else {
            Box::new(OpenAiClipSuggester::new(config(a), key, t).unwrap())
        };
        let mut w = input("en", 3);
        w.settings.count = 1;
        let r = run(p.as_ref(), &w).unwrap();
        assert!(!r.clips.is_empty());
        if let Some(dir) = std::env::var_os("VIBECUT_CLIP_SMOKE_OUTPUT_DIR") {
            let dir = std::path::PathBuf::from(dir);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(
                dir.join(if a { "anthropic.json" } else { "openai.json" }),
                serde_json::to_vec_pretty(&r).unwrap(),
            )
            .unwrap();
        }

        eprintln!(
            "{}: {} validated synthetic clips",
            if a { "Anthropic" } else { "OpenAI" },
            r.clips.len()
        );
    }
}

#[test]
fn recovery_offer_executes_nothing_and_excludes_cancellation() {
    assert_eq!(
        clip_recovery_choice(&Error::Network),
        Some(ClipRecoveryChoice::RerunLocally)
    );
    assert_eq!(clip_recovery_choice(&Error::Cancelled), None);
    assert_eq!(clip_recovery_choice(&Error::ConsentRequired), None);
}
#[test]
fn fixed_synthetic_ranking_baseline_en_vi_zh() {
    let mut baseline_hits = 0;
    let mut keyword_hits = 0;
    for (lang, keyword, text) in [
        ("en", "secret", "The secret saves hours."),
        ("vi", "bí quyết", "Bí quyết giúp tiết kiệm thời gian."),
        ("zh", "方法", "这个方法节省时间。"),
    ] {
        let p = HeuristicClipSuggester::new().unwrap();
        let mut w = input(lang, 3);
        w.settings.count = 1;
        w.settings.max_duration = tick(4);
        w.segments[0].text = "Welcome. Thank you.".into();
        w.segments[1].text = text.into();
        w.segments[2].text = "Goodbye. See you.".into();
        let baseline = run(&p, &w).unwrap();
        baseline_hits += usize::from(baseline.clips[0].proposal.start_segment_id.as_str() == "s1");
        w.settings.keywords = vec![keyword.into()];
        let ranked = run(&p, &w).unwrap();
        keyword_hits += usize::from(ranked.clips[0].proposal.start_segment_id.as_str() == "s1");
        assert_eq!(ranked.clips[0].range.duration(), tick(4));
    }
    assert_eq!(baseline_hits, 0);
    assert_eq!(keyword_hits, 3);
    eprintln!(
        "synthetic must-clip hit@1: sentence/pause-only baseline {baseline_hits}/3; keyword ranking {keyword_hits}/3; duration compliance 3/3"
    );
}
#[test]
fn overlap_threshold_and_near_duplicate_keep_highest_score() {
    for a in [false, true] {
        let mut w = input("en", 5);
        w.settings.count = 5;
        w.settings.overlap_tolerance = 0.25;
        let (p, _) = provider(
            a,
            response(
                a,
                json!([
                    proposal("s0", "s1", 0.7),
                    proposal("s1", "s2", 0.9),
                    proposal("s3", "s4", 0.8)
                ]),
            ),
        );
        let r = run(p.as_ref(), &w).unwrap();
        assert_eq!(r.clips.len(), 2);
        assert_eq!(r.clips[0].proposal.score, 0.9);
        assert!(
            r.dropped
                .iter()
                .any(|d| d.reason == ClipDropReason::Overlap)
        );
        w.settings.overlap_tolerance = 1.0;
        let (p, _) = provider(
            a,
            response(
                a,
                json!([proposal("s0", "s2", 0.7), proposal("s1", "s2", 0.9)]),
            ),
        );
        let r = run(p.as_ref(), &w).unwrap();
        assert_eq!(r.clips.len(), 1);
        assert_eq!(r.clips[0].proposal.score, 0.9);
        assert!(
            r.dropped
                .iter()
                .any(|d| d.reason == ClipDropReason::Duplicate)
        );
    }
}

#[test]
fn review_dense_long_local_input() {
    let p = HeuristicClipSuggester::new().unwrap();
    let mut w = input("en", 600);
    w.settings.count = 1;
    w.settings.max_duration = tick(120);
    for (i, s) in w.segments.iter_mut().enumerate() {
        s.range = SourceRange::new(tick(i as i64), tick(i as i64 + 1)).unwrap();
    }
    w.validate().unwrap();
    let windows = w.windows().unwrap();
    assert_eq!(windows.len(), 60);
    assert!(windows.iter().map(|w| w.segments.len()).sum::<usize>() > 4096);
    let result = run(&p, &w).expect("valid ten-minute local transcript should return one clip");
    assert_eq!(result.clips.len(), 1);
    assert_eq!(result.clips[0].source, ClipSource::Heuristic);
    assert_eq!(
        result.clips[0].range,
        SourceRange::new(tick(0), tick(4)).unwrap()
    );
    assert!(result.dropped.len() <= 4096);
}

#[test]
fn repeated_window_observations_keep_highest_score_first_tie_and_bounded_diagnostics() {
    struct RepeatedWindows {
        anthropic: bool,
        calls: AtomicUsize,
    }
    impl HttpTransport for RepeatedWindows {
        fn send(&self, _: HttpRequest) -> Result<HttpResponse, Error> {
            let call = self.calls.fetch_add(1, Ordering::Relaxed);
            let score = [0.2, 0.9, 0.9, 0.4][call];
            let mut valid = proposal("s100", "s103", score);
            valid["title"] = json!(format!("window {call}"));
            let mut proposals = vec![valid; 64];
            proposals.extend(vec![proposal("s100", "s100", score); 64]);
            Ok(response(self.anthropic, json!(proposals)))
        }
    }
    for anthropic in [false, true] {
        let transport = Arc::new(RepeatedWindows {
            anthropic,
            calls: AtomicUsize::new(0),
        });
        let p = remote(anthropic, transport.clone());
        let mut w = input("en", 150);
        w.settings.count = 1;
        w.settings.max_duration = tick(120);
        for (i, s) in w.segments.iter_mut().enumerate() {
            s.range = SourceRange::new(tick(i as i64), tick(i as i64 + 1)).unwrap();
        }
        assert_eq!(w.windows().unwrap().len(), 4);
        let result = run(p.as_ref(), &w).unwrap();
        assert_eq!(transport.calls.load(Ordering::Relaxed), 4);
        assert_eq!(result.clips.len(), 1);
        assert_eq!(result.clips[0].proposal.score, 0.9);
        assert_eq!(result.clips[0].proposal.title.as_str(), "window 1");
        assert_eq!(result.dropped.len(), 2);
        assert_eq!(
            result
                .dropped
                .iter()
                .filter(|d| d.reason == ClipDropReason::Duplicate)
                .count(),
            1
        );
        assert_eq!(
            result
                .dropped
                .iter()
                .filter(|d| d.reason == ClipDropReason::Duration)
                .count(),
            1
        );
    }
}

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "test fixtures"
)]
use ai::transport::ApiKey;
use ai::{
    Consent, CostEstimate, Error, ExecutionHooks, Money, Operation, ProviderConfig, Request,
    SampleRange, Scope, Stage, clips::*,
};
use captions::LanguageTag;
use serde_json::{Value, json};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use time::{SourceRange, TICKS_PER_SECOND, Tick};
use transport::{HttpRequest, HttpResponse, HttpTransport, UreqTransport};
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
            max_segments: 4,
            min_segment_duration: Tick(TICKS_PER_SECOND),
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
    if anthropic {
        ai::clips::AnthropicClipSuggester::provider_config("claude-sonnet-5-5").unwrap()
    } else {
        ai::clips::OpenAiClipSuggester::provider_config("gpt-6.1-sol").unwrap()
    }
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
    json!({"segments":[{"start_segment_id":a,"end_segment_id":b,"role":"payoff"}],"title":"Time saving","hook_text":"Try this","reason":"Complete useful thought","score":score})
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
    fn send(&self, r: HttpRequest) -> Result<HttpResponse, transport::Error> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        self.requests.lock().unwrap().push(r);
        if let Some(h) = &self.cancel {
            h.0.store(true, Ordering::Relaxed);
        }
        if self.fail {
            return Err(transport::Error::Network);
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
            r.clips[0].segments[0].range,
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
        for mut v in [proposal("s0", "s1", -0.1), proposal("s0", "s1", 1.1)] {
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
                    .any(|s| s.range.start() == c.segments[0].range.start())
            );
            assert!(
                w.segments
                    .iter()
                    .any(|s| s.range.end() == c.segments[0].range.end())
            );
            assert!((tick(4)..=tick(15)).contains(&c.segments[0].range.duration()));
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
#[ignore = "requires VIBECUT_LIVE_AI=1 and OPENAI_API_KEY; one bounded gpt-6-luna request"]
fn live_clip_providers() {
    assert_eq!(std::env::var("VIBECUT_LIVE_AI").as_deref(), Ok("1"));
    let cfg = ai::clips::OpenAiClipSuggester::provider_config("gpt-6-luna").unwrap();
    let p = OpenAiClipSuggester::new(
        cfg,
        ApiKey::new(std::env::var("OPENAI_API_KEY").unwrap()).unwrap(),
        Arc::new(UreqTransport::new()),
    )
    .unwrap();
    let mut w = if let Some(path) = std::env::var_os("VIBECUT_CLIP_SMOKE_INPUT") {
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
    } else {
        input("en", 3)
    };
    w.settings.count = 1;
    assert_eq!(
        w.windows().unwrap().len(),
        1,
        "live smoke must stay one bounded invocation"
    );
    let r = run(&p, &w).unwrap();
    assert!(!r.clips.is_empty());
    if let Some(dir) = std::env::var_os("VIBECUT_CLIP_SMOKE_OUTPUT_DIR") {
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("openai.json"),
            serde_json::to_vec_pretty(&r).unwrap(),
        )
        .unwrap();
    }
    eprintln!(
        "gpt-6-luna: {} validated clips, {} source pieces",
        r.clips.len(),
        r.clips[0].segments.len()
    );
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
        baseline_hits += usize::from(
            baseline.clips[0].proposal.segments[0]
                .start_segment_id
                .as_str()
                == "s1",
        );
        w.settings.keywords = vec![keyword.into()];
        let ranked = run(&p, &w).unwrap();
        keyword_hits += usize::from(
            ranked.clips[0].proposal.segments[0]
                .start_segment_id
                .as_str()
                == "s1",
        );
        assert_eq!(ranked.clips[0].segments[0].range.duration(), tick(4));
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
fn dense_long_local_input_returns_one_bounded_clip() {
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
        result.clips[0].segments[0].range,
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
        fn send(&self, _: HttpRequest) -> Result<HttpResponse, transport::Error> {
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

fn assembled(pieces: &[(&str, &str, &str)], score: f64) -> Value {
    let mut p = proposal("s0", "s0", score);
    p["segments"] = pieces
        .iter()
        .map(|(a, b, role)| json!({"start_segment_id":a,"end_segment_id":b,"role":role}))
        .collect();
    p
}

#[test]
fn assembled_play_order_retains_source_order_and_serializes_without_enclosing_range() {
    for a in [false, true] {
        let w = input("en", 8);
        let (p, _) = provider(
            a,
            response(
                a,
                json!([assembled(
                    &[
                        ("s6", "s6", "hook"),
                        ("s0", "s0", "context"),
                        ("s3", "s3", "payoff")
                    ],
                    0.9
                )]),
            ),
        );
        let r = run(p.as_ref(), &w).unwrap();
        let c = &r.clips[0];
        assert_eq!(c.duration, tick(12));
        assert_eq!(
            c.segments
                .iter()
                .map(|s| s.source_order)
                .collect::<Vec<_>>(),
            [2, 0, 1]
        );
        assert_eq!(
            c.segments
                .iter()
                .map(|s| s.range.start())
                .collect::<Vec<_>>(),
            [tick(30), tick(0), tick(15)]
        );
        assert_eq!(c.proposal.segments[0].role, ClipRole::Hook);
        let json = serde_json::to_value(c).unwrap();
        assert!(json.get("range").is_none());
        assert_eq!(json["proposal"]["hook_text"], "Try this");
        assert_eq!(json["segments"][0]["source_order"], 2);
    }
}

#[test]
fn invalid_assemblies_drop_individually_with_typed_reasons() {
    for a in [false, true] {
        let mut w = input("en", 8);
        w.settings.max_duration = tick(12);
        let cases = [
            (assembled(&[], 0.9), ClipDropReason::SegmentCount),
            (
                assembled(&[("s0", "s0", "payoff"), ("s1", "s1", "hook")], 0.9),
                ClipDropReason::HookOrder,
            ),
            (
                assembled(&[("s0", "s0", "hook"), ("s1", "s1", "hook")], 0.9),
                ClipDropReason::HookOrder,
            ),
            (
                assembled(&[("s0", "s1", "hook"), ("s1", "s2", "payoff")], 0.9),
                ClipDropReason::SegmentOverlap,
            ),
            (
                assembled(&[("s0", "s0", "hook"), ("s0", "s0", "payoff")], 0.9),
                ClipDropReason::SegmentOverlap,
            ),
            (
                assembled(&[("missing", "s1", "payoff")], 0.9),
                ClipDropReason::UnknownId,
            ),
            (
                assembled(&[("s2", "s1", "payoff")], 0.9),
                ClipDropReason::ReversedRange,
            ),
            (
                assembled(
                    &[
                        ("s0", "s0", "payoff"),
                        ("s1", "s1", "context"),
                        ("s2", "s2", "payoff"),
                        ("s3", "s3", "payoff"),
                        ("s4", "s4", "payoff"),
                    ],
                    0.9,
                ),
                ClipDropReason::SegmentCount,
            ),
            (
                assembled(&[("s0", "s0", "hook"), ("s2", "s3", "payoff")], 0.9),
                ClipDropReason::Duration,
            ),
        ];
        for (bad, reason) in cases {
            let (p, _) = provider(a, response(a, json!([bad, proposal("s7", "s7", 0.8)])));
            let r = run(p.as_ref(), &w).unwrap();
            assert_eq!(r.clips.len(), 1, "{reason:?}");
            assert_eq!(r.dropped.len(), 1, "{reason:?}");
            assert_eq!(r.dropped[0].reason, reason);
        }
        for role in ["unknown", "Hook", ""] {
            let (p, _) = provider(
                a,
                response(a, json!([assembled(&[("s0", "s0", role)], 0.5)])),
            );
            assert!(matches!(run(p.as_ref(), &w), Err(Error::Schema)));
        }
        let mut old = proposal("s0", "s1", 0.5);
        old.as_object_mut().unwrap().remove("segments");
        old["start_segment_id"] = json!("s0");
        old["end_segment_id"] = json!("s1");
        let (p, _) = provider(a, response(a, json!([old])));
        assert!(matches!(run(p.as_ref(), &w), Err(Error::Schema)));
    }
}

#[test]
fn adjacency_merges_only_touching_consecutive_play_pieces_with_first_role() {
    for a in [false, true] {
        let mut w = input("en", 4);
        w.segments[1].range = SourceRange::new(tick(4), tick(8)).unwrap();
        let cases = [
            (vec![("s0", "s0", "hook"), ("s1", "s1", "payoff")], 1),
            (vec![("s1", "s1", "hook"), ("s0", "s0", "payoff")], 2),
            (
                vec![
                    ("s0", "s0", "hook"),
                    ("s3", "s3", "context"),
                    ("s1", "s1", "payoff"),
                ],
                3,
            ),
            (vec![("s1", "s1", "hook"), ("s2", "s2", "payoff")], 2),
        ];
        for (pieces, count) in cases {
            let (p, _) = provider(a, response(a, json!([assembled(&pieces, 0.9)])));
            let r = run(p.as_ref(), &w).unwrap();
            assert_eq!(r.clips[0].segments.len(), count);
            assert_eq!(r.clips[0].proposal.segments.len(), count);
            assert_eq!(r.clips[0].segments[0].role, ClipRole::Hook);
            if count == 1 {
                assert_eq!(r.clips[0].duration, tick(8));
                assert_eq!(
                    r.clips[0].segments[0].repair,
                    Some(ClipRepairReason::AdjacentMerged)
                );
                assert_eq!(
                    r.clips[0].proposal.segments[0].end_segment_id.as_str(),
                    "s1"
                );
            }
        }
    }
}

#[test]
fn canonical_piece_minimum_and_summed_duration_bounds_are_inclusive() {
    for a in [false, true] {
        let mut w = input("en", 4);
        w.settings.min_duration = tick(8);
        w.settings.max_duration = tick(8);
        w.settings.min_segment_duration = tick(4);
        let (p, _) = provider(
            a,
            response(
                a,
                json!([assembled(
                    &[("s3", "s3", "hook"), ("s0", "s0", "payoff")],
                    0.9
                )]),
            ),
        );
        assert_eq!(run(p.as_ref(), &w).unwrap().clips[0].duration, tick(8));
        w.settings.min_segment_duration = tick(4 + 1);
        let (p, _) = provider(
            a,
            response(
                a,
                json!([assembled(
                    &[("s3", "s3", "hook"), ("s0", "s0", "payoff")],
                    0.9
                )]),
            ),
        );
        assert_eq!(
            run(p.as_ref(), &w).unwrap().dropped[0].reason,
            ClipDropReason::SegmentDuration
        );
        w.settings.min_segment_duration = tick(1);
        let (p, _) = provider(a, response(a, json!([proposal("s0", "s0", 0.9)])));
        assert_eq!(
            run(p.as_ref(), &w).unwrap().dropped[0].reason,
            ClipDropReason::Duration
        );
        // Touching sub-minimum pieces are judged after canonical adjacency repair.
        w.segments[0].range = SourceRange::new(tick(0), tick(4)).unwrap();
        w.segments[1].range = SourceRange::new(tick(4), tick(8)).unwrap();
        w.settings.min_segment_duration = tick(8);
        let (p, _) = provider(
            a,
            response(
                a,
                json!([assembled(
                    &[("s0", "s0", "hook"), ("s1", "s1", "context")],
                    0.9
                )]),
            ),
        );
        assert_eq!(run(p.as_ref(), &w).unwrap().clips[0].segments.len(), 1);
    }
}

#[test]
fn multisource_dedupe_uses_summed_overlap_excluding_gaps_and_play_order() {
    for a in [false, true] {
        let mut w = input("en", 8);
        w.settings.overlap_tolerance = 0.2;
        let (p, _) = provider(
            a,
            response(
                a,
                json!([
                    assembled(&[("s7", "s7", "hook"), ("s0", "s0", "payoff")], 0.9),
                    assembled(&[("s0", "s0", "hook"), ("s7", "s7", "payoff")], 0.8),
                    assembled(&[("s2", "s2", "hook"), ("s5", "s5", "payoff")], 0.7),
                    assembled(&[("s0", "s0", "hook"), ("s3", "s3", "payoff")], 0.6)
                ]),
            ),
        );
        let r = run(p.as_ref(), &w).unwrap();
        assert_eq!(r.clips.len(), 2); // enclosing envelopes overlap, actual source does not
        assert_eq!(
            r.dropped
                .iter()
                .filter(|d| d.reason == ClipDropReason::Duplicate)
                .count(),
            1
        );
        assert_eq!(
            r.dropped
                .iter()
                .filter(|d| d.reason == ClipDropReason::Overlap)
                .count(),
            1
        );
        // Same text plus >= 0.5 summed IoU is a near-duplicate, even with different pieces.
        w.settings.overlap_tolerance = 1.0;
        let (p, _) = provider(
            a,
            response(
                a,
                json!([
                    assembled(
                        &[
                            ("s0", "s0", "hook"),
                            ("s3", "s3", "context"),
                            ("s7", "s7", "payoff")
                        ],
                        0.9
                    ),
                    assembled(&[("s3", "s3", "hook"), ("s7", "s7", "payoff")], 0.7)
                ]),
            ),
        );
        let r = run(p.as_ref(), &w).unwrap();
        assert_eq!(r.clips.len(), 1);
        assert_eq!(r.dropped[0].reason, ClipDropReason::Duplicate);
    }
}

#[test]
fn settings_default_maximum_and_hostile_limits() {
    let w = input("en", 3);
    let mut json = serde_json::to_value(&w).unwrap();
    json["settings"]
        .as_object_mut()
        .unwrap()
        .remove("max_segments");
    let parsed: ClipInput = serde_json::from_value(json).unwrap();
    assert_eq!(parsed.settings.max_segments, 4);
    for n in [0, 33, usize::MAX] {
        let mut bad = w.clone();
        bad.settings.max_segments = n;
        assert!(bad.validate().is_err());
    }
    for minimum in [Tick::ZERO, Tick(-1), tick(16)] {
        let mut bad = w.clone();
        bad.settings.min_segment_duration = minimum;
        assert!(bad.validate().is_err());
    }
}

#[test]
fn fixed_multilingual_assembly_contract_baseline() {
    let mut accepted = 0;
    for lang in ["en", "vi", "zh"] {
        let w = input(lang, 8);
        let (p, _) = provider(
            false,
            response(
                false,
                json!([assembled(
                    &[
                        ("s6", "s6", "hook"),
                        ("s0", "s0", "context"),
                        ("s3", "s3", "payoff")
                    ],
                    0.9
                )]),
            ),
        );
        let r = run(p.as_ref(), &w).unwrap();
        let c = &r.clips[0];
        accepted += usize::from(
            c.duration == tick(12) && c.segments.iter().map(|s| s.source_order).eq([2, 0, 1]),
        );
    }
    assert_eq!(accepted, 3);
    eprintln!(
        "fixed EN/VI/ZH assembly contract acceptance: {accepted}/3; single-range contract baseline 0/3 (cannot represent reordered disjoint pieces)"
    );
}

#[test]
fn heuristic_respects_piece_minimum_when_it_exceeds_total_minimum() {
    let mut w = input("vi", 3);
    w.settings.min_segment_duration = tick(9);
    let p = HeuristicClipSuggester::new().unwrap();
    let r = run(&p, &w).unwrap();
    assert!(!r.clips.is_empty());
    for c in r.clips {
        assert_eq!(c.segments.len(), 1);
        assert_eq!(c.segments[0].role, ClipRole::Payoff);
        assert!(c.duration >= tick(9));
    }
}

#[test]
fn luna_model_selection_disclosure_and_nested_schema_match_the_wire() {
    let w = input("en", 3);
    let t = Arc::new(Transport {
        response: Mutex::new(Some(response(false, json!([proposal("s0", "s0", 0.9)])))),
        calls: AtomicUsize::new(0),
        requests: Mutex::new(vec![]),
        cancel: None,
        fail: false,
    });
    let cfg = ai::clips::OpenAiClipSuggester::provider_config("gpt-6-luna").unwrap();
    let p =
        OpenAiClipSuggester::new(cfg, ApiKey::new("synthetic-key").unwrap(), t.clone()).unwrap();
    let s = scope(&w, &p);
    assert_eq!(
        p.disclosure(&s).unwrap().unwrap().model.as_str(),
        "gpt-6-luna"
    );
    assert_eq!(run(&p, &w).unwrap().clips.len(), 1);
    let requests = t.requests.lock().unwrap();
    let wire: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(wire["model"], "gpt-6-luna");
    let item = &wire["text"]["format"]["schema"]["properties"]["clips"]["items"];
    assert!(item["properties"].get("start_segment_id").is_none());
    assert_eq!(
        item["properties"]["segments"]["items"]["properties"]["role"]["enum"],
        json!(["hook", "context", "payoff"])
    );
    assert!(
        wire["instructions"]
            .as_str()
            .unwrap()
            .contains("PLAY order")
    );
}

#[test]
fn invalid_window_observation_cannot_replace_valid_metadata_or_score() {
    struct CrossWindow {
        calls: AtomicUsize,
        invalid_first: bool,
    }
    impl HttpTransport for CrossWindow {
        fn send(&self, _: HttpRequest) -> Result<HttpResponse, transport::Error> {
            let call = self.calls.fetch_add(1, Ordering::Relaxed);
            let invalid = if self.invalid_first {
                call == 0
            } else {
                call == 1
            };
            let id = if self.invalid_first { "s130" } else { "s0" };
            let mut p = proposal(id, id, if invalid { 1.0 } else { 0.5 });
            p["title"] = json!(if invalid {
                "invalid window"
            } else {
                "valid window"
            });
            Ok(response(false, json!([p])))
        }
    }
    for invalid_first in [false, true] {
        let t = Arc::new(CrossWindow {
            calls: AtomicUsize::new(0),
            invalid_first,
        });
        let p = remote(false, t.clone());
        let w = input("en", 150);
        assert_eq!(w.windows().unwrap().len(), 2);
        let r = run(p.as_ref(), &w).unwrap();
        assert_eq!(r.clips.len(), 1);
        assert_eq!(r.clips[0].proposal.score, 0.5);
        assert_eq!(r.clips[0].proposal.title.as_str(), "valid window");
    }
}

#[test]
fn configured_piece_limit_is_inclusive_and_total_sums_near_tick_limit() {
    let mut w = input("en", 64);
    w.settings.max_segments = 32;
    w.settings.max_duration = tick(128);
    let pieces: Vec<_> = (0..32)
        .map(|i| (format!("s{}", i * 2), format!("s{}", i * 2)))
        .collect();
    let refs: Vec<_> = pieces
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str(), "payoff"))
        .collect();
    let (p, _) = provider(false, response(false, json!([assembled(&refs, 0.9)])));
    let r = run(p.as_ref(), &w).unwrap();
    assert_eq!(r.clips[0].segments.len(), 32);
    assert_eq!(r.clips[0].duration, tick(128));
    let mut w = input("en", 2);
    w.segments[0].range = SourceRange::new(Tick::ZERO, Tick(i64::MAX / 2)).unwrap();
    w.segments[1].range = SourceRange::new(Tick(i64::MAX / 2 + 1), Tick(i64::MAX)).unwrap();
    w.settings.max_duration = Tick(i64::MAX);
    let (p, _) = provider(
        false,
        response(
            false,
            json!([assembled(
                &[("s1", "s1", "hook"), ("s0", "s0", "payoff")],
                0.9
            )]),
        ),
    );
    let r = run(p.as_ref(), &w).unwrap();
    assert_eq!(r.clips[0].duration, Tick(i64::MAX - 1));
}

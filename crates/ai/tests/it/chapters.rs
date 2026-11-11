#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "test fixtures"
)]
use ai::{
    Capabilities, Consent, CostEstimate, Error, ExecutionHooks, Money, Operation, ProviderConfig,
    Request, SampleRange, Scope, Stage, Translation, TranslationWindow, Translator,
    chapters::*,
    transport::{ApiKey, HttpRequest, HttpResponse, HttpTransport, UreqTransport},
};
use captions::{LanguageTag, NfcText};
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
fn source_text(language: &str) -> &'static str {
    match language {
        "vi" => "Bí quyết học tập mỗi ngày.",
        "zh" | "zh-Hans" | "zh-Hant" => "每天学习的好习惯。",
        _ => "A habit for learning every day.",
    }
}
fn input(language: &str, n: usize) -> ChapterInput {
    ChapterInput {
        segments: (0..n)
            .map(|i| ChapterSegment {
                id: format!("s{i}").into(),
                text: source_text(language).into(),
                range: SourceRange::new(tick(i as i64 * 5), tick(i as i64 * 5 + 4)).unwrap(),
            })
            .collect(),
        settings: ChapterSettings {
            language: LanguageTag::parse(language).unwrap(),
            max_chapters: 5,
            duration: tick(n as i64 * 5),
        },
    }
}
fn config(anthropic: bool) -> ProviderConfig {
    if anthropic {
        ai::chapters::AnthropicChapterGenerator::provider_config("claude-sonnet-5-5").unwrap()
    } else {
        ai::chapters::OpenAiChapterGenerator::provider_config("gpt-6-luna").unwrap()
    }
}
fn scope(w: &ChapterInput, p: &dyn ChapterGenerator) -> Scope {
    Scope {
        provider: p.config().clone(),
        operation: Operation::GenerateChapters,
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
fn proposal(chapters: Value) -> Value {
    json!({
        "title": "Bí quyết học tập",
        "description": "Một cuộc trò chuyện về thói quen.",
        "chapters": chapters
    })
}
fn envelope(anthropic: bool, body: &Value) -> HttpResponse {
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
fn provider(anthropic: bool, body: HttpResponse) -> (Box<dyn ChapterGenerator>, Arc<Transport>) {
    let t = Arc::new(Transport {
        response: Mutex::new(Some(body)),
        calls: AtomicUsize::new(0),
        requests: Mutex::new(vec![]),
        cancel: None,
        fail: false,
    });
    (remote(anthropic, t.clone()), t)
}
fn remote(anthropic: bool, t: Arc<dyn HttpTransport>) -> Box<dyn ChapterGenerator> {
    let key = ApiKey::new("synthetic-key").unwrap();
    if anthropic {
        Box::new(AnthropicChapterGenerator::new(config(true), key, t).unwrap())
    } else {
        Box::new(OpenAiChapterGenerator::new(config(false), key, t).unwrap())
    }
}
fn run(p: &dyn ChapterGenerator, w: &ChapterInput) -> Result<ChapterDocument, Error> {
    let scope = scope(w, p);
    let consent = Consent::grant(scope.clone(), Money::new("USD", 1_000_000).unwrap()).unwrap();
    generate_chapters(
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
fn offline_fixtures_generate_unicode_documents_per_language() {
    for (language, file, title, chapter_starts) in [
        (
            "vi",
            "chương-vi.json",
            "Bí quyết học tập",
            vec![tick(0), tick(10)],
        ),
        (
            "en",
            "chapter-en.json",
            "Learning habits",
            vec![tick(5), tick(15)],
        ),
        (
            "zh",
            "章节-zh.json",
            "学习习惯",
            vec![tick(0), tick(10), tick(20)],
        ),
    ] {
        let body = std::fs::read(format!(
            "{}/tests/fixtures/chapters/{file}",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();
        let (p, t) = provider(
            false,
            HttpResponse {
                status: 200,
                headers: vec![],
                body,
            },
        );
        let document = run(p.as_ref(), &input(language, 5)).unwrap();
        assert_eq!(document.schema_version(), SCHEMA_VERSION);
        assert_eq!(document.language().as_str(), language);
        assert_eq!(document.title().as_str(), title);
        assert!(!document.description().as_str().trim().is_empty());
        assert_eq!(
            document
                .chapters()
                .iter()
                .map(|chapter| chapter.start)
                .collect::<Vec<_>>(),
            chapter_starts
        );
        assert_eq!(t.calls.load(Ordering::Relaxed), 1);
    }
}

#[test]
fn chapter_starts_snap_to_cue_boundaries_increase_and_stay_inside_duration() {
    for anthropic in [false, true] {
        let (p, _) = provider(
            anthropic,
            envelope(
                anthropic,
                &proposal(json!([
                    {"start_segment_id":"s0","title":"Mở đầu"},
                    {"start_segment_id":"s2","title":"Giữa"},
                    {"start_segment_id":"s4","title":"Kết"}
                ])),
            ),
        );
        let document = run(p.as_ref(), &input("vi", 5)).unwrap();
        assert_eq!(
            document
                .chapters()
                .iter()
                .map(|chapter| chapter.start)
                .collect::<Vec<_>>(),
            vec![tick(0), tick(10), tick(20)]
        );
        assert!(
            document
                .chapters()
                .windows(2)
                .all(|pair| pair[0].start < pair[1].start)
        );
        assert!(
            document
                .chapters()
                .iter()
                .all(|chapter| chapter.start < document.duration())
        );
    }
    for chapters in [
        json!([{"start_segment_id":"s9","title":"Unknown"}]),
        json!([
            {"start_segment_id":"s2","title":"First"},
            {"start_segment_id":"s2","title":"Duplicate"}
        ]),
        json!([
            {"start_segment_id":"s2","title":"Later"},
            {"start_segment_id":"s1","title":"Earlier"}
        ]),
        json!([{"start_segment_id":"s0","title":"   "}]),
        json!([
            {"start_segment_id":"s0","title":"One"},
            {"start_segment_id":"s1","title":"Two"},
            {"start_segment_id":"s2","title":"Three"},
            {"start_segment_id":"s3","title":"Four"},
            {"start_segment_id":"s4","title":"Five"},
            {"start_segment_id":"s0","title":"Six"}
        ]),
    ] {
        let (p, _) = provider(false, envelope(false, &proposal(chapters)));
        assert!(matches!(
            run(p.as_ref(), &input("en", 5)),
            Err(Error::Schema)
        ));
    }
}

#[test]
fn empty_and_unsupported_transcripts_refuse_with_typed_reason() {
    let mut empty = input("en", 0);
    empty.settings.duration = tick(10);
    assert_eq!(
        empty.validate().unwrap_err(),
        ChapterRefusal::EmptyTranscript
    );
    assert_eq!(
        input("fr", 3).validate().unwrap_err(),
        ChapterRefusal::UnsupportedLanguage
    );
    let mut reversed = input("en", 3);
    reversed.segments[1].range = SourceRange::new(tick(0), tick(1)).unwrap();
    assert_eq!(
        reversed.validate().unwrap_err(),
        ChapterRefusal::InvalidSegments
    );
    let mut duplicate = input("en", 3);
    duplicate.segments[1].id = duplicate.segments[0].id.clone();
    assert_eq!(
        duplicate.validate().unwrap_err(),
        ChapterRefusal::InvalidSegments
    );
    let mut outside = input("en", 3);
    outside.segments[2].range = SourceRange::new(tick(20), tick(21)).unwrap();
    assert_eq!(
        outside.validate().unwrap_err(),
        ChapterRefusal::InvalidSegments
    );
    let mut blank = input("en", 3);
    blank.segments[0].text = "   ".into();
    assert_eq!(
        blank.validate().unwrap_err(),
        ChapterRefusal::InvalidSegments
    );
    let mut dense = input("en", MAX_SEGMENTS + 1);
    dense.settings.duration = tick(1_000_000);
    assert_eq!(dense.validate().unwrap_err(), ChapterRefusal::Payload);
    let mut oversized = input("en", 1);
    oversized.segments[0].text = "x".repeat(MAX_INPUT_BYTES + 1).into();
    assert_eq!(oversized.validate().unwrap_err(), ChapterRefusal::Payload);
    for (max_chapters, duration) in [(0, tick(10)), (MAX_CHAPTERS + 1, tick(10)), (1, Tick::ZERO)] {
        let mut settings = input("en", 1);
        settings.settings.max_chapters = max_chapters;
        settings.settings.duration = duration;
        assert_eq!(
            settings.validate().unwrap_err(),
            ChapterRefusal::InvalidSettings
        );
    }
    let (p, t) = provider(false, envelope(false, &proposal(json!([]))));
    let empty_scope = Scope {
        provider: p.config().clone(),
        operation: Operation::GenerateChapters,
        payload_fingerprint: "synthetic".into(),
        mode: MODE.into(),
        texts: Vec::new(),
        source_language: empty.settings.language.clone(),
        target_language: None,
        source_samples: SampleRange::new(0, 1).unwrap(),
        sample_rate: 1,
        max_bytes: MAX_WIRE,
    };
    assert!(matches!(
        generate_chapters(
            p.as_ref(),
            &empty,
            &Request {
                scope: &empty_scope,
                consent: None
            },
            &Hooks::default()
        ),
        Err(Error::Chapters(ChapterRefusal::EmptyTranscript))
    ));
    assert_eq!(t.calls.load(Ordering::Relaxed), 0);
}

#[test]
fn document_schema_round_trips_and_rejects_wrong_version() {
    let document = ChapterDocument::new(
        LanguageTag::parse("vi").unwrap(),
        tick(25),
        "Hành trình học tập".into(),
        "Một cuộc trò chuyện về thói quen.".into(),
        vec![
            Chapter {
                start: Tick::ZERO,
                segment_id: "s0".into(),
                title: "Mở đầu".into(),
            },
            Chapter {
                start: tick(10),
                segment_id: "s2".into(),
                title: "Thói quen".into(),
            },
        ],
        vec![ChapterTranslation {
            language: LanguageTag::parse("en").unwrap(),
            title: "A learning journey".into(),
            description: "A conversation about habits.".into(),
            chapters: vec!["Opening".into(), "Habits".into()],
        }],
    )
    .unwrap();
    let json = document.to_json().unwrap();
    assert_eq!(ChapterDocument::from_json(&json).unwrap(), document);
    let text = document.to_text().unwrap();
    assert!(text.contains("00:00:10 Thói quen"));
    assert!(text.contains("[en]"));
    assert!(text.contains("00:00:10 Habits"));

    let mut value: Value = serde_json::from_slice(&json).unwrap();
    value["schema_version"] = json!(SCHEMA_VERSION + 1);
    let invalid = serde_json::to_vec(&value).unwrap();
    assert!(matches!(
        ChapterDocument::from_json(&invalid),
        Err(Error::Chapters(ChapterRefusal::SchemaVersion))
    ));
    let mut value: Value = serde_json::from_slice(&json).unwrap();
    let first_start = value["chapters"][0]["start"].clone();
    value["chapters"][1]["start"] = first_start;
    let invalid = serde_json::to_vec(&value).unwrap();
    assert!(matches!(
        ChapterDocument::from_json(&invalid),
        Err(Error::Chapters(ChapterRefusal::InvalidDocument))
    ));
    let mut value: Value = serde_json::from_slice(&json).unwrap();
    value["extra"] = json!(true);
    let invalid = serde_json::to_vec(&value).unwrap();
    assert!(matches!(
        ChapterDocument::from_json(&invalid),
        Err(Error::Schema)
    ));
}

#[test]
fn consent_settings_language_cost_and_wire_ceiling_precede_io() {
    for anthropic in [false, true] {
        let (p, t) = provider(anthropic, envelope(anthropic, &proposal(json!([]))));
        let w = input("en", 3);
        let scope = scope(&w, p.as_ref());
        let consent = Consent::grant(scope.clone(), Money::new("USD", 1_000_000).unwrap()).unwrap();
        assert!(matches!(
            generate_chapters(
                p.as_ref(),
                &w,
                &Request {
                    scope: &scope,
                    consent: None
                },
                &Hooks::default()
            ),
            Err(Error::ConsentRequired)
        ));
        let mut changed = w.clone();
        changed.segments[1].text = "Changed text.".into();
        assert!(matches!(
            generate_chapters(
                p.as_ref(),
                &changed,
                &Request {
                    scope: &scope,
                    consent: Some(&consent)
                },
                &Hooks::default()
            ),
            Err(Error::ConsentRequired)
        ));
        let low = Consent::grant(scope.clone(), Money::new("USD", 0).unwrap()).unwrap();
        assert!(matches!(
            generate_chapters(
                p.as_ref(),
                &w,
                &Request {
                    scope: &scope,
                    consent: Some(&low)
                },
                &Hooks::default()
            ),
            Err(Error::Cost)
        ));
        let mut tiny = scope.clone();
        tiny.max_bytes = w.consent_text().unwrap().as_str().len() as u64 - 1;
        let tiny_consent =
            Consent::grant(tiny.clone(), Money::new("USD", 1_000_000).unwrap()).unwrap();
        assert!(matches!(
            generate_chapters(
                p.as_ref(),
                &w,
                &Request {
                    scope: &tiny,
                    consent: Some(&tiny_consent)
                },
                &Hooks::default()
            ),
            Err(Error::Payload)
        ));
        assert_eq!(t.calls.load(Ordering::Relaxed), 0);
    }
}

#[test]
fn provider_schema_network_and_cancellation_failures_are_typed() {
    for anthropic in [false, true] {
        for (status, expected) in [
            (401, "auth"),
            (429, "rate"),
            (500, "server"),
            (413, "payload"),
        ] {
            let (p, _) = provider(
                anthropic,
                HttpResponse {
                    status,
                    headers: vec![],
                    body: vec![],
                },
            );
            let error = run(p.as_ref(), &input("en", 3)).unwrap_err();
            assert!(matches!(
                (expected, error),
                ("auth", Error::Auth)
                    | ("rate", Error::RateLimit { .. })
                    | ("server", Error::Server)
                    | ("payload", Error::Payload)
            ));
        }
        let mut body = proposal(json!([{"start_segment_id":"s0","title":"One"}]));
        body["chapters"][0]["start_time"] = json!(0);
        let (p, _) = provider(anthropic, envelope(anthropic, &body));
        assert!(matches!(
            run(p.as_ref(), &input("en", 3)),
            Err(Error::Schema)
        ));

        let t = Arc::new(Transport {
            response: Mutex::new(None),
            calls: AtomicUsize::new(0),
            requests: Mutex::new(vec![]),
            cancel: None,
            fail: true,
        });
        assert!(matches!(
            run(remote(anthropic, t.clone()).as_ref(), &input("en", 3)),
            Err(Error::Network)
        ));
        assert_eq!(t.calls.load(Ordering::Relaxed), 1);

        let hooks = Arc::new(Hooks::default());
        let t = Arc::new(Transport {
            response: Mutex::new(Some(envelope(
                anthropic,
                &proposal(json!([{"start_segment_id":"s0","title":"One"}])),
            ))),
            calls: AtomicUsize::new(0),
            requests: Mutex::new(vec![]),
            cancel: Some(hooks.clone()),
            fail: false,
        });
        let p = remote(anthropic, t.clone());
        let w = input("en", 3);
        let scope = scope(&w, p.as_ref());
        let consent = Consent::grant(scope.clone(), Money::new("USD", 1_000_000).unwrap()).unwrap();
        assert!(matches!(
            generate_chapters(
                p.as_ref(),
                &w,
                &Request {
                    scope: &scope,
                    consent: Some(&consent)
                },
                hooks.as_ref()
            ),
            Err(Error::Cancelled)
        ));
        assert_eq!(t.calls.load(Ordering::Relaxed), 1);
    }
}

#[test]
fn adapter_wire_identity_and_disclosure_name_the_selected_provider() {
    for anthropic in [false, true] {
        let (p, t) = provider(
            anthropic,
            envelope(
                anthropic,
                &proposal(json!([{"start_segment_id":"s0","title":"One"}])),
            ),
        );
        let w = input("vi", 3);
        let scope = scope(&w, p.as_ref());
        let consent = Consent::grant(scope.clone(), Money::new("USD", 1_000_000).unwrap()).unwrap();
        generate_chapters(
            p.as_ref(),
            &w,
            &Request {
                scope: &scope,
                consent: Some(&consent),
            },
            &Hooks::default(),
        )
        .unwrap();
        let requests = t.requests.lock().unwrap();
        let wire: Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(wire["model"], p.config().model().as_str());
        if anthropic {
            assert_eq!(requests[0].url, ai::anthropic::ENDPOINT);
            assert_eq!(wire["output_config"]["format"]["type"], "json_schema");
            assert_eq!(wire["thinking"]["type"], "between_tools");
        } else {
            assert_eq!(requests[0].url, ai::openai::TRANSLATION_ENDPOINT);
            assert_eq!(wire["store"], false);
            assert_eq!(wire["text"]["format"]["strict"], true);
        }
        assert!(!format!("{:?}", requests[0]).contains("synthetic-key"));
        drop(requests);
        let disclosure = if anthropic {
            ai::chapters::AnthropicChapterGenerator::new(
                config(true),
                ApiKey::new("synthetic-key").unwrap(),
                Arc::new(Transport {
                    response: Mutex::new(None),
                    calls: AtomicUsize::new(0),
                    requests: Mutex::new(vec![]),
                    cancel: None,
                    fail: false,
                }),
            )
            .unwrap()
            .disclosure(&scope)
        } else {
            ai::chapters::OpenAiChapterGenerator::new(
                config(false),
                ApiKey::new("synthetic-key").unwrap(),
                Arc::new(Transport {
                    response: Mutex::new(None),
                    calls: AtomicUsize::new(0),
                    requests: Mutex::new(vec![]),
                    cancel: None,
                    fail: false,
                }),
            )
            .unwrap()
            .disclosure(&scope)
        }
        .unwrap()
        .unwrap();
        assert_eq!(
            disclosure.upload_bytes,
            u64::try_from(w.consent_text().unwrap().as_str().len()).unwrap()
        );
        assert!(matches!(disclosure.estimate, CostEstimate::UpperBound(_)));
        assert!(disclosure.link("policy").is_some());
    }
}

struct EchoTranslator(ProviderConfig);
impl Translator for EchoTranslator {
    fn config(&self) -> &ProviderConfig {
        &self.0
    }
    fn capabilities(&self, _: &Scope) -> Capabilities {
        Capabilities {
            model: self.0.model().clone(),
            mode: ai::openai::TRANSLATION_MODE.into(),
            operation: Operation::Translate,
            languages: ["vi", "en"]
                .into_iter()
                .filter_map(|s| LanguageTag::parse(s).ok())
                .collect(),
            target_languages: ["en"]
                .into_iter()
                .filter_map(|s| LanguageTag::parse(s).ok())
                .collect(),
            timing: Default::default(),
            diarization: ai::Support::Unsupported,
            live_audio_input: ai::Support::Unsupported,
            file_result_stream: ai::Support::Unsupported,
            max_upload_bytes: Some(ai::MAX_WINDOW_BYTES as u64),
            max_duration_micros: None,
            evidence: ai::Evidence::Advertised,
            revision: "echo".into(),
        }
    }
    fn estimate(&self, _: &Scope) -> CostEstimate {
        CostEstimate::Free
    }
    fn translate(
        &self,
        window: &TranslationWindow,
        _: &Request<'_>,
        _: &dyn ExecutionHooks,
    ) -> Result<Translation, Error> {
        Ok(Translation {
            source_language: window.source_language.clone(),
            target_language: window.target_language.clone(),
            texts: window
                .segments
                .iter()
                .map(|segment| {
                    (
                        segment.id.clone(),
                        format!("EN {}", segment.text.as_str()).into(),
                    )
                })
                .collect(),
            number_warnings: Vec::new(),
        })
    }
}

#[test]
fn optional_translation_reuses_the_shared_window_without_moving_chapters() {
    let document = ChapterDocument::new(
        LanguageTag::parse("vi").unwrap(),
        tick(25),
        "Hành trình".into(),
        "Một cuộc trò chuyện.".into(),
        vec![
            Chapter {
                start: Tick::ZERO,
                segment_id: "s0".into(),
                title: "Mở đầu".into(),
            },
            Chapter {
                start: tick(10),
                segment_id: "s2".into(),
                title: "Thói quen".into(),
            },
        ],
        Vec::new(),
    )
    .unwrap();
    let source = document.language().clone();
    let target = LanguageTag::parse("en").unwrap();
    let style = NfcText::from("trung tính");
    let window = TranslationWindow {
        source_language: source.clone(),
        target_language: target.clone(),
        segments: vec![
            ai::TranslationSegment {
                id: "title".into(),
                text: document.title().clone(),
            },
            ai::TranslationSegment {
                id: "description".into(),
                text: document.description().clone(),
            },
            ai::TranslationSegment {
                id: "chapter-0".into(),
                text: document.chapters()[0].title.clone(),
            },
            ai::TranslationSegment {
                id: "chapter-1".into(),
                text: document.chapters()[1].title.clone(),
            },
        ],
        before: Vec::new(),
        after: Vec::new(),
        glossary: Vec::new(),
        style: style.clone(),
    };
    let config = ai::openai::OpenAiTranslator::provider_config("gpt-6.1-sol").unwrap();
    let scope = Scope {
        provider: config.clone(),
        operation: Operation::Translate,
        payload_fingerprint: "synthetic".into(),
        mode: ai::openai::TRANSLATION_MODE.into(),
        texts: vec![window.consent_text().unwrap()],
        source_language: source.clone(),
        target_language: Some(target.clone()),
        source_samples: SampleRange::new(0, 1).unwrap(),
        sample_rate: 1,
        max_bytes: ai::MAX_WINDOW_BYTES as u64,
    };
    let consent = Consent::grant(scope.clone(), Money::new("USD", 1_000_000).unwrap()).unwrap();
    let translated = translate_chapters(
        &EchoTranslator(config.clone()),
        &document,
        target.clone(),
        &style,
        &Request {
            scope: &scope,
            consent: Some(&consent),
        },
        &Hooks::default(),
    )
    .unwrap();
    assert_eq!(translated.chapters(), document.chapters());
    assert_eq!(translated.translations().len(), 1);
    assert_eq!(translated.translations()[0].title.as_str(), "EN Hành trình");
    assert_eq!(
        translated.translations()[0].chapters,
        vec![NfcText::from("EN Mở đầu"), NfcText::from("EN Thói quen")]
    );
    assert_eq!(document.translations().len(), 0);
    assert!(matches!(
        translate_chapters(
            &EchoTranslator(config.clone()),
            &document,
            source,
            &style,
            &Request {
                scope: &scope,
                consent: Some(&consent)
            },
            &Hooks::default(),
        ),
        Err(Error::Chapters(ChapterRefusal::InvalidTranslation))
    ));
}

#[test]
#[ignore = "requires VIBECUT_LIVE_AI=1 and OPENAI_API_KEY; one bounded gpt-6-luna request"]
fn live_chapter_generation() {
    assert_eq!(std::env::var("VIBECUT_LIVE_AI").as_deref(), Ok("1"));
    let p = OpenAiChapterGenerator::new(
        config(false),
        ApiKey::new(std::env::var("OPENAI_API_KEY").unwrap()).unwrap(),
        Arc::new(UreqTransport::new()),
    )
    .unwrap();
    let input = if let Some(path) = std::env::var_os("VIBECUT_CHAPTERS_SMOKE_INPUT") {
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
    } else {
        input("vi", 5)
    };
    let document = run(&p, &input).unwrap();
    document.validate().unwrap();
    assert!(!document.chapters().is_empty());
    if let Some(dir) = std::env::var_os("VIBECUT_CHAPTERS_SMOKE_OUTPUT_DIR") {
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("chapters.json"), document.to_json().unwrap()).unwrap();
        std::fs::write(dir.join("chapters.txt"), document.to_text().unwrap()).unwrap();
    }
    eprintln!(
        "gpt-6-luna: {} chapter(s), title {:?}",
        document.chapters().len(),
        document.title().as_str()
    );
}

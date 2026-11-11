#![allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "integration fixtures"
)]
use ai::{
    openai::*,
    transport::{ApiKey, HttpRequest, HttpResponse, HttpTransport},
    *,
};
use captions::{Cue, CueText, CueTrack, LanguageTag};
use engine::{
    ByteBudget, Command, Error as EngineError, Session,
    jobs::*,
    translation::{self, TranslationRun, TranslationTarget},
};
use project::{
    CaptionParams, Item, ItemId, ItemParams, Project, ProjectId, Track, TrackId, TrackKind,
};
use serde_json::json;
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use time::{SourceRange, Tick};
fn lang(s: &str) -> LanguageTag {
    LanguageTag::parse(s).unwrap()
}
fn session() -> Session {
    let mut data = Project::empty(ProjectId(1), "".into()).unwrap().into_data();
    data.assets.push(
        media::SourceAsset::try_from(media::AssetData {
            ephemeral: false,
            id: media::AssetId(1),
            path: std::env::temp_dir().join("translation synthetic source"),
            content_hash: media::ContentHash([0; 32]),
            probe: media::SourceProbe {
                format: "fixture".into(),
                duration: None,
                streams: vec![],
            },
        })
        .unwrap(),
    );
    let cues = (0..4)
        .map(|i| Cue {
            id: format!("cue-{i}").into(),
            range: SourceRange::new(Tick(i * 10), Tick(i * 10 + 10)).unwrap(),
            source_lang: lang("vi"),
            texts: [
                (
                    lang("vi"),
                    CueText {
                        text: format!("nguồn {i}").into(),
                        edited: false,
                        translation_ref: None,
                    },
                ),
                (
                    lang("en"),
                    CueText {
                        text: "user edit".into(),
                        edited: i == 1,
                        translation_ref: None,
                    },
                ),
            ]
            .into(),
            word_ranges: Vec::new(),
            style: None,
        })
        .collect();
    data.scene_mut().timeline.insert_track(Track {
        id: TrackId(1),
        name: "captions".into(),
        kind: TrackKind::Text,
        muted: false,
        hidden: false,
        locked: false,
        items: vec![Item {
            hidden: false,
            blend_mode: Default::default(),
            retime: Default::default(),
            masks: Vec::new(),
            effects: Vec::new(),
            id: ItemId(0),
            source_asset: Some(media::AssetId(1)),
            source_range: SourceRange::new(Tick(0), Tick(40)).unwrap(),
            start: Tick(0),
            animations: Default::default(),
            content: ItemParams::Caption(CaptionParams {
                cues: CueTrack {
                    styles: Vec::new(),
                    cues,
                },
            }),
        }],
    });
    Session::new(Project::new(data).unwrap(), 10).unwrap()
}
fn cues(s: &Session) -> &[Cue] {
    let ItemParams::Caption(p) = &s.project().data().scene().timeline.overlays[0].items[0].content
    else {
        panic!("fixture")
    };
    &p.cues.cues
}
struct Transport {
    calls: AtomicUsize,
    fail_count: usize,
}
impl HttpTransport for Transport {
    fn send(&self, request: HttpRequest) -> Result<HttpResponse, Error> {
        let call = self.calls.fetch_add(1, Ordering::Relaxed);
        let wire: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
        let w: TranslationWindow = serde_json::from_str(wire["input"].as_str().unwrap()).unwrap();
        let entries = if call < self.fail_count {
            vec![]
        } else {
            w.segments
                .iter()
                .map(|s| json!({"id":s.id,"text":format!("translated {}",s.text.as_str())}))
                .collect()
        };
        let result = json!({"translations":entries});
        Ok(HttpResponse{status:200,headers:vec![],body:serde_json::to_vec(&json!({"status":"completed","output":[{"type":"message","role":"assistant","status":"completed","content":[{"type":"output_text","text":result.to_string()}]}]})).unwrap()})
    }
}
fn try_run(
    session: &Session,
    w: TranslationWindow,
    fail_count: usize,
    max_attempts: u8,
    budget: u64,
) -> Result<(Arc<TranslationRun>, Arc<Transport>), EngineError> {
    let config = OpenAiTranslator::provider_config("gpt-6.1-sol").unwrap();
    let scope =
        Scope::translation(config.clone(), TRANSLATION_MODE.into(), &w, "test".into()).unwrap();
    let c = Consent::grant(scope.clone(), Money::new("USD", 1_000_000).unwrap()).unwrap();
    let transport = Arc::new(Transport {
        calls: AtomicUsize::new(0),
        fail_count,
    });
    let p = Arc::new(
        OpenAiTranslator::new(config, ApiKey::new("test").unwrap(), transport.clone()).unwrap(),
    );
    Ok((
        TranslationRun::new(
            TranslationTarget {
                session,
                item: ItemId(0),
            },
            p,
            w,
            scope,
            Some(c),
            max_attempts,
            Some(Money::new("USD", budget).unwrap()),
        )?,
        transport,
    ))
}
fn run(
    session: &Session,
    w: TranslationWindow,
    fail_count: usize,
    max_attempts: u8,
    budget: u64,
) -> (Arc<TranslationRun>, Arc<Transport>) {
    try_run(session, w, fail_count, max_attempts, budget).unwrap()
}
fn runner() -> JobRunner {
    JobRunner::new(
        RunnerConfig {
            concurrency: [1, 1, 1],
            queue: 2,
            retained: 2,
            network_attempts: 3,
            backoff: Duration::from_millis(1),
            progress_interval: Duration::from_millis(10),
        },
        ByteBudget::new(16 * 1024 * 1024).unwrap(),
        Arc::new(SystemClock::default()),
    )
    .unwrap()
}
fn pump(r: &mut JobRunner, id: JobId) -> JobSnapshot {
    let end = Instant::now() + Duration::from_secs(10);
    loop {
        r.tick().unwrap();
        let s = r.snapshot(id).unwrap();
        if matches!(
            s.state,
            JobState::Completed | JobState::Failed | JobState::Cancelled
        ) {
            return s;
        }
        assert!(Instant::now() < end);
        std::thread::sleep(Duration::from_millis(1));
    }
}
fn selected(s: &Session, force: bool) -> TranslationWindow {
    translation::window(
        s,
        ItemId(0),
        &["cue-1".into(), "cue-2".into()],
        lang("en"),
        vec![],
        "concise".into(),
        force,
    )
    .unwrap()
}
#[test]
fn selection_context_and_explicit_json_force_preserve_user_edits_and_unselected_cues() {
    let mut s = session();
    let w = selected(&s, false);
    assert_eq!(w.segments.len(), 1);
    assert_eq!(w.segments[0].id.as_str(), "cue-2");
    assert_eq!(w.before.len(), 2);
    assert_eq!(w.after.len(), 1);
    assert_eq!(selected(&s, true).segments.len(), 2);
    for force in [false, true] {
        s.apply(Command::Retranslate {
            item: ItemId(0),
            target_language: lang("en"),
            texts: [
                ("cue-1".into(), "changed".into()),
                ("cue-2".into(), "changed".into()),
            ]
            .into_iter()
            .collect(),
            translation_ref: "openai/test".into(),
            force,
        })
        .unwrap();
        assert_eq!(
            cues(&s)[1].texts[&lang("en")].text.as_str(),
            if force { "changed" } else { "user edit" }
        );
        assert_eq!(cues(&s)[0].texts[&lang("en")].text.as_str(), "user edit");
        assert_eq!(cues(&s)[3].texts[&lang("en")].text.as_str(), "user edit");
    }
    let before = s.project().clone();
    assert!(
        s.apply(Command::Retranslate {
            item: ItemId(0),
            target_language: lang("en"),
            texts: [
                ("cue-1".into(), "changed".into()),
                ("missing".into(), "bad".into()),
            ]
            .into_iter()
            .collect(),
            translation_ref: "test".into(),
            force: true,
        })
        .is_err()
    );
    assert!(Arc::ptr_eq(s.project(), &before));
}
#[test]
fn engine_repairs_schema_with_same_provider_then_applies_one_revision_bound_command() {
    let mut s = session();
    let (run, t) = run(&s, selected(&s, false), 1, 2, 1_000_000);
    let mut r = runner();
    let (id, _) = run.submit(&mut r, &s).unwrap();
    let done = pump(&mut r, id);
    assert_eq!(done.state, JobState::Completed);
    assert_eq!(done.attempts, 2);
    assert_eq!(t.calls.load(Ordering::Relaxed), 2);
    let output = r.take_output(id).unwrap();
    let translated: Translation = serde_json::from_slice(output.bytes()).unwrap();
    output
        .apply(
            &mut s,
            Command::Retranslate {
                item: ItemId(0),
                target_language: translated.target_language,
                texts: translated.texts,
                translation_ref: "openai/test".into(),
                force: false,
            },
        )
        .unwrap();
    assert_eq!(cues(&s)[1].texts[&lang("en")].text.as_str(), "user edit");
    assert!(
        cues(&s)[2].texts[&lang("en")]
            .text
            .as_str()
            .starts_with("translated")
    );
    assert_eq!(s.history_len(), (1, 0));
    s.undo().unwrap();
    assert_eq!(cues(&s)[2].texts[&lang("en")].text.as_str(), "user edit");
}
#[test]
fn maximum_model_ids_submit_translation_with_exact_consent_and_distinct_cache_keys() {
    struct SelectedTransport {
        model: String,
        inner: Transport,
    }
    impl HttpTransport for SelectedTransport {
        fn send(&self, request: HttpRequest) -> Result<HttpResponse, Error> {
            let wire: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
            assert_eq!(wire["model"], self.model);
            self.inner.send(request)
        }
    }
    let mut keys = Vec::new();
    let mut previous_acknowledgement = None;
    for suffix in ['a', 'b'] {
        let model = format!("{}{suffix}", "x".repeat(255));
        let mut s = session();
        let window = selected(&s, false);
        let config = OpenAiTranslator::provider_config(&model).unwrap();
        let scope = Scope::translation(
            config.clone(),
            TRANSLATION_MODE.into(),
            &window,
            "fixture".into(),
        )
        .unwrap();
        let transport = Arc::new(SelectedTransport {
            model,
            inner: Transport {
                calls: AtomicUsize::new(0),
                fail_count: 0,
            },
        });
        let provider = Arc::new(
            OpenAiTranslator::new(
                config.clone(),
                ApiKey::new("fixture").unwrap(),
                transport.clone(),
            )
            .unwrap(),
        );
        assert_eq!(provider.estimate(&scope), CostEstimate::Unknown);
        let target = || TranslationTarget {
            session: &s,
            item: ItemId(0),
        };
        let ordinary =
            Consent::grant(scope.clone(), Money::new("USD", 1_000_000).unwrap()).unwrap();
        assert!(matches!(
            TranslationRun::new(
                target(),
                provider.clone(),
                window.clone(),
                scope.clone(),
                Some(ordinary),
                1,
                None,
            ),
            Err(EngineError::Ai(Error::Cost))
        ));
        assert_eq!(transport.inner.calls.load(Ordering::Relaxed), 0);
        if let Some(previous) = &previous_acknowledgement {
            assert!(matches!(
                Consent::grant_unknown_price(scope.clone(), previous),
                Err(Error::ConsentRequired)
            ));
        }
        let acknowledgement = UnknownPriceAcknowledgement::grant(config, Operation::Translate);
        let token = Consent::grant_unknown_price(scope.clone(), &acknowledgement).unwrap();
        previous_acknowledgement = Some(acknowledgement);
        let run =
            TranslationRun::new(target(), provider, window, scope, Some(token), 1, None).unwrap();
        let mut runner = runner();
        let (id, _) = run.submit(&mut runner, &s).unwrap();
        keys.push(runner.snapshot(id).unwrap().input);
        let done = pump(&mut runner, id);
        assert_eq!(done.state, JobState::Completed, "{:?}", done.error);
        assert_eq!(transport.inner.calls.load(Ordering::Relaxed), 1);
        let output = runner.take_output(id).unwrap();
        let translated: Translation = serde_json::from_slice(output.bytes()).unwrap();
        output
            .apply(
                &mut s,
                Command::Retranslate {
                    item: ItemId(0),
                    target_language: translated.target_language,
                    texts: translated.texts,
                    translation_ref: "fixture".into(),
                    force: false,
                },
            )
            .unwrap();
        assert!(
            cues(&s)[2].texts[&lang("en")]
                .text
                .as_str()
                .starts_with("translated")
        );
    }
    assert_ne!(
        keys[0], keys[1],
        "the complete model ID must isolate translation jobs"
    );
}
#[test]
fn retries_cannot_reset_operation_budget_and_late_results_refuse_changed_session() {
    let mut s = session();
    let (run, t) = run(&s, selected(&s, false), 10, 1, 1_000_000);
    let mut r = runner();
    let (id, _) = run.submit(&mut r, &s).unwrap();
    let done = pump(&mut r, id);
    assert_eq!(done.state, JobState::Failed);
    assert_eq!(t.calls.load(Ordering::Relaxed), 1);
    assert!(r.take_output(id).is_none());
    let (run, _) = self::run(&s, selected(&s, false), 0, 1, 1_000_000);
    let (id, _) = run.submit(&mut r, &s).unwrap();
    s.apply(Command::AddTrack {
        placement: engine::TrackPlacement::Auto,
        name: "".into(),
        kind: TrackKind::Video,
    })
    .unwrap();
    assert_eq!(pump(&mut r, id).state, JobState::Completed);
    let output = r.take_output(id).unwrap();
    let result: Translation = serde_json::from_slice(output.bytes()).unwrap();
    assert!(matches!(
        output.apply(
            &mut s,
            Command::Retranslate {
                item: ItemId(0),
                target_language: result.target_language,
                texts: result.texts,
                translation_ref: "test".into(),
                force: false
            }
        ),
        Err(engine::EditError::Session(
            engine::SessionError::StaleResult
        ))
    ));
}

#[test]
fn operation_budget_covers_all_attempts_before_any_translation_starts() {
    let s = session();
    assert!(matches!(
        try_run(&s, selected(&s, false), 0, 2, 344064),
        Err(EngineError::Ai(Error::Cost))
    ));
    assert!(matches!(
        try_run(&s, selected(&s, false), 0, 0, 1_000_000),
        Err(EngineError::Control(engine::control::ControlError::Limit))
    ));
}

#[test]
fn consent_preparation_cannot_rebind_old_source_to_a_new_session_revision() {
    let mut s = session();
    let mut stale = selected(&s, false);
    stale.segments.first_mut().unwrap().text = "old source".into();
    assert!(matches!(
        try_run(&s, stale, 0, 1, 1_000_000),
        Err(EngineError::Session(engine::SessionError::StaleResult))
    ));
    let (run, transport) = run(&s, selected(&s, false), 0, 1, 1_000_000);
    s.apply(Command::AddTrack {
        placement: engine::TrackPlacement::Auto,
        name: "edit during consent".into(),
        kind: TrackKind::Video,
    })
    .unwrap();
    assert!(matches!(
        run.submit(&mut runner(), &s),
        Err(EngineError::Session(engine::SessionError::StaleResult))
    ));
    assert_eq!(transport.calls.load(Ordering::Relaxed), 0);
}

#[test]
fn retranslate_during_trim_retains_unselected_hidden_cues() {
    let mut s = session();
    s.begin_gesture().unwrap();
    s.apply(Command::TrimEnd {
        item: ItemId(0),
        at: Tick(20),
    })
    .unwrap();
    assert_eq!(cues(&s).len(), 2);
    s.apply(Command::Retranslate {
        item: ItemId(0),
        target_language: lang("en"),
        texts: [("cue-0".into(), "new translation".into())].into(),
        translation_ref: "review".into(),
        force: false,
    })
    .unwrap();
    s.apply(Command::TrimEnd {
        item: ItemId(0),
        at: Tick(40),
    })
    .unwrap();
    assert_eq!(
        cues(&s).len(),
        4,
        "reversing trim must restore unrelated source cues"
    );
    assert_eq!(
        cues(&s)[0].texts[&lang("en")].text.as_str(),
        "new translation"
    );
}

fn translate_cue(id: &str, force: bool) -> Command {
    Command::Retranslate {
        item: ItemId(0),
        target_language: lang("en"),
        texts: [(id.into(), "selected translation".into())].into(),
        translation_ref: "retained-translation".into(),
        force,
    }
}

#[test]
fn translation_preserves_partial_and_hidden_cues_across_every_trim_retention_lifetime() {
    for mode in ["gesture", "group", "batch", "merged"] {
        for (start, edge, cue_index) in
            [(false, 15, 1), (false, 25, 2), (true, 15, 1), (true, 25, 2)]
        {
            for force in [false, true] {
                let mut s = session();
                let original = s.project().clone();
                let original_cues = cues(&s).to_vec();
                let mut expected = original_cues.clone();
                let id = expected[cue_index].id.as_str().to_owned();
                expected[cue_index].set_text(
                    lang("en"),
                    CueText {
                        text: "selected translation".into(),
                        edited: false,
                        translation_ref: Some("retained-translation".into()),
                    },
                    force,
                );
                let trim = |at| {
                    if start {
                        Command::TrimStart {
                            item: ItemId(0),
                            at: Tick(at),
                        }
                    } else {
                        Command::TrimEnd {
                            item: ItemId(0),
                            at: Tick(at),
                        }
                    }
                };
                let commands = vec![
                    trim(edge),
                    translate_cue(&id, force),
                    trim(if start { edge + 3 } else { edge - 3 }),
                    trim(if start { 0 } else { 40 }),
                ];
                match mode {
                    "gesture" => {
                        s.begin_gesture().unwrap();
                        for command in commands {
                            s.apply(command).unwrap();
                        }
                        s.commit_gesture().unwrap();
                    }
                    "group" => s
                        .transaction(|s| -> Result<(), engine::EditError> {
                            for command in commands {
                                s.apply(command)?;
                            }
                            Ok(())
                        })
                        .unwrap(),
                    "batch" => {
                        s.apply(Command::Batch { commands }).unwrap();
                    }
                    "merged" => {
                        for command in commands {
                            s.apply_merged(command, Some("trim-and-translate")).unwrap();
                        }
                        s.end_merge();
                    }
                    _ => panic!("unknown fixture mode"),
                }
                assert_eq!(
                    cues(&s),
                    expected,
                    "{mode} start={start} edge={edge} force={force}"
                );
                let item = &s.project().data().scene().timeline.overlays[0].items[0];
                assert_eq!(item.start, Tick(0));
                assert_eq!(
                    item.source_range,
                    SourceRange::new(Tick(0), Tick(40)).unwrap()
                );
                if expected != original_cues {
                    assert_eq!(s.history_len(), (1, 0));
                }
                if s.can_undo() {
                    s.undo().unwrap();
                    assert_eq!(s.project(), &original);
                    s.redo().unwrap();
                    assert_eq!(cues(&s), expected);
                }
            }
        }
    }
}

#[test]
fn translation_trim_cancel_and_failed_commands_restore_retained_content() {
    for force in [false, true] {
        let mut s = session();
        let original = s.project().clone();
        s.begin_gesture().unwrap();
        s.apply(Command::TrimEnd {
            item: ItemId(0),
            at: Tick(15),
        })
        .unwrap();
        s.apply(translate_cue("cue-1", force)).unwrap();
        s.cancel_gesture().unwrap();
        assert_eq!(s.project(), &original);
        assert_eq!(s.history_len(), (0, 0));

        s.begin_gesture().unwrap();
        s.apply(Command::TrimEnd {
            item: ItemId(0),
            at: Tick(15),
        })
        .unwrap();
        let preview = s.project().clone();
        assert!(
            s.apply(translate_cue("cue-3", force)).is_err(),
            "hidden IDs are not selected visible cues"
        );
        assert!(Arc::ptr_eq(s.project(), &preview));
        assert!(
            s.apply(Command::Batch {
                commands: vec![
                    translate_cue("cue-0", force),
                    Command::Delete {
                        items: vec![ItemId(999)]
                    }
                ]
            })
            .is_err()
        );
        assert!(Arc::ptr_eq(s.project(), &preview));
        s.apply(Command::TrimEnd {
            item: ItemId(0),
            at: Tick(40),
        })
        .unwrap();
        assert_eq!(
            s.project(),
            &original,
            "failed batch must also roll back retained translations"
        );
        s.cancel_gesture().unwrap();

        let result: Result<(), EngineError> = s.transaction(|s| {
            s.apply(Command::TrimStart {
                item: ItemId(0),
                at: Tick(15),
            })?;
            s.apply(translate_cue("cue-1", force))?;
            Err(EngineError::Edit(engine::EditError::Invalid(
                "test rollback",
            )))
        });
        assert!(result.is_err());
        assert_eq!(s.project(), &original);
    }
}

#[test]
fn an_edited_partial_cue_can_be_hidden_then_restored_after_another_translation() {
    for start in [false, true] {
        for force in [false, true] {
            let mut s = session();
            let mut expected = cues(&s).to_vec();
            for (index, allow) in [(1, force), (if start { 3 } else { 0 }, false)] {
                expected[index].set_text(
                    lang("en"),
                    CueText {
                        text: "selected translation".into(),
                        edited: false,
                        translation_ref: Some("retained-translation".into()),
                    },
                    allow,
                );
            }
            let trim = |at| {
                if start {
                    Command::TrimStart {
                        item: ItemId(0),
                        at: Tick(at),
                    }
                } else {
                    Command::TrimEnd {
                        item: ItemId(0),
                        at: Tick(at),
                    }
                }
            };
            s.begin_gesture().unwrap();
            s.apply(trim(15)).unwrap();
            s.apply(translate_cue("cue-1", force)).unwrap();
            assert_eq!(
                cues(&s)
                    .iter()
                    .find(|c| c.id.as_str() == "cue-1")
                    .unwrap()
                    .range
                    .duration(),
                Tick(5)
            );
            s.apply(trim(if start { 25 } else { 5 })).unwrap();
            assert!(!cues(&s).iter().any(|c| c.id.as_str() == "cue-1"));
            s.apply(translate_cue(if start { "cue-3" } else { "cue-0" }, false))
                .unwrap();
            s.apply(trim(if start { 0 } else { 40 })).unwrap();
            assert_eq!(cues(&s), expected);
            s.commit_gesture().unwrap();
            assert_eq!(cues(&s), expected);
            s.undo().unwrap();
            assert_eq!(cues(&s), cues(&session()));
            s.redo().unwrap();
            assert_eq!(cues(&s), expected);
        }
    }
}

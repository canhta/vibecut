#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "public-interface conformance assertions"
)]
use engine::control::{
    self, Caller, ControlResult, ErrorCode, Payload, Policy, Request, Response, ResponseValue,
};
use engine::jobs::{
    JobBytes, JobKind, JobRunner, JobSpec, JobState, ResourceClass, RunnerConfig, SystemClock,
};
use engine::{ByteBudget, Command, EditError, Effect, Fixture, Session, registry};
use project::{
    Color, Item, ItemId, ItemParams, OverlayParams, Project, ProjectId, Track, TrackId, TrackKind,
};
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use time::{FrameRate, SourceRange, Tick};

struct Allow;
impl Policy for Allow {
    fn authorize(&self, _: &Command, _: &Project) -> engine::Result<()> {
        Ok(())
    }
}
struct Deny;
impl Policy for Deny {
    fn authorize(&self, _: &Command, _: &Project) -> engine::Result<()> {
        Err(EditError::Permission)
    }
}
fn caller(policy: &dyn Policy) -> Caller<'_> {
    Caller {
        principal: "test",
        external: true,
        policy,
        fonts: None,
    }
}
fn runner() -> JobRunner {
    JobRunner::new(
        RunnerConfig {
            concurrency: [1, 1, 1],
            queue: 16,
            retained: 16,
            network_attempts: 1,
            backoff: Duration::from_millis(1),
            progress_interval: Duration::from_millis(10),
        },
        ByteBudget::new(512 << 20).unwrap(),
        Arc::new(SystemClock::default()),
    )
    .unwrap()
}
fn session() -> Session {
    Session::new(
        Project::empty(ProjectId(u64::MAX), "Test".into()).unwrap(),
        10,
    )
    .unwrap()
}
fn request(s: &Session, id: &str, params: Value, key: &str) -> Request {
    Request {
        api_version: control::API_VERSION,
        registry_hash: Some(control::registry_hash().into()),
        session_id: s.session_id().into(),
        request_id: "request".into(),
        idempotency_key: Some(key.into()),
        base_rev: Some(s.revision()),
        command: id.into(),
        params,
    }
}
fn request_at(
    s: &Session,
    request_id: &str,
    id: &str,
    params: Value,
    key: &str,
    base: &control::Revision,
) -> Request {
    Request {
        api_version: control::API_VERSION,
        registry_hash: Some(control::registry_hash().into()),
        session_id: s.session_id().into(),
        request_id: request_id.into(),
        idempotency_key: Some(key.into()),
        base_rev: Some(base.clone()),
        command: id.into(),
        params,
    }
}
fn run(s: &mut Session, r: &mut JobRunner, id: &str, params: Value, key: &str) -> Response {
    let req = request(s, id, params, key);
    s.control_request(req, caller(&Allow), r).unwrap()
}
fn ok(response: Response) -> ControlResult {
    match response.value {
        ResponseValue::Result(v) => v,
        ResponseValue::Error(e) => panic!("unexpected error {e:?}"),
    }
}
fn error(response: Response, code: ErrorCode) {
    assert!(
        matches!(response.value,ResponseValue::Error(c) if c==code),
        "{response:?}"
    );
}
fn add(s: &mut Session, r: &mut JobRunner, key: &str) {
    ok(run(
        s,
        r,
        "timeline.add_track",
        json!({"name":"Added","kind":"video"}),
        key,
    ));
}

#[test]
fn export_control_admits_typed_settings_refuses_old_range_and_preserves_revision() {
    let mut data = session().project().data().clone();
    data.canvas.width = 64;
    data.canvas.height = 64;
    data.scene_mut().timeline.insert_track(Track {
        id: TrackId(1),
        name: "Picture".into(),
        kind: TrackKind::Video,
        muted: false,
        hidden: false,
        locked: false,
        items: vec![Item {
            hidden: false,
            id: ItemId(1),
            start: Tick::ZERO,
            source_asset: None,
            source_range: SourceRange::new(Tick::ZERO, Tick(time::TICKS_PER_SECOND)).unwrap(),
            content: ItemParams::Overlay(OverlayParams {
                color: Color([20, 80, 120, 255]),
            }),
            blend_mode: Default::default(),
            retime: Default::default(),
            masks: vec![],
            effects: vec![],
            animations: Default::default(),
        }],
    });
    let mut session = Session::new(Project::new(data).unwrap(), 10).unwrap();
    let mut runner = runner();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("typed.mp4");
    let before = project::encode(session.project(), false).unwrap();
    let revision = session.revision();
    let settings = render::OutputOptions {
        resolution: Some([64, 64]),
        range: render::RangeChoice::InOut {
            start: Tick::ZERO,
            end: Tick(time::TICKS_PER_SECOND / 2),
        },
        ..Default::default()
    };
    let response = ok(run(
        &mut session,
        &mut runner,
        "export",
        json!({"settings":settings,"path":path,"overwrite":false}),
        "typed-export",
    ));
    let job = match response {
        ControlResult::JobAccepted { job, paths, .. } => {
            assert_eq!(paths, vec![path.clone()]);
            job
        }
        other => panic!("{other:?}"),
    };
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    loop {
        runner.tick().unwrap();
        let snapshot = runner.snapshot(job).unwrap();
        if matches!(
            snapshot.state,
            JobState::Completed | JobState::Failed | JobState::Cancelled
        ) {
            assert_eq!(snapshot.state, JobState::Completed, "{:?}", snapshot.error);
            break;
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(path.exists());
    assert_eq!(session.revision(), revision);
    assert_eq!(project::encode(session.project(), false).unwrap(), before);
    let refused = run(
        &mut session,
        &mut runner,
        "export",
        json!({"range":{"start":0,"end":1},"path":path,"overwrite":true}),
        "old-export",
    );
    assert!(matches!(refused.value, ResponseValue::Error(_)));
    runner.shutdown();
}
fn seed() -> Session {
    let mut data = session().project().data().clone();
    data.canvas.width = 64;
    data.canvas.height = 64;
    data.canvas.fps = FrameRate::FPS_30;
    let duration = Tick(time::TICKS_PER_SECOND);
    let mut animations = project::Animations::default();
    let mut param = project::Param::new(project::ParamValue::Scalar(1.0)).unwrap();
    param
        .channel_mut(None)
        .unwrap()
        .put(project::Keyframe::new(
            Tick::ZERO,
            project::ParamValue::Scalar(1.0),
        ))
        .unwrap();
    animations
        .set(project::AnimationPath::new("opacity").unwrap(), param)
        .unwrap();
    for (id, path) in [
        (0, "vibecut-control-caption-source.mp4"),
        (98, "vibecut-control-unused-source.mp4"),
    ] {
        data.assets.push(
            testkit::source::asset(id, std::env::temp_dir().join(path))
                .format("mp4")
                .duration(duration)
                .content_hash(media::ContentHash::from_bytes(b"fixture"))
                .stream(
                    testkit::source::video()
                        .time_base(media::TimeBase::new(1, 30000).unwrap())
                        .duration(duration)
                        .start(Tick::ZERO)
                        .size(64, 64)
                        .build(),
                )
                .build(),
        );
    }
    data.scene_mut().timeline.overlays = vec![
        Track {
            id: TrackId(1),
            name: "Overlay".into(),
            kind: TrackKind::Video,
            muted: false,
            hidden: false,
            locked: false,
            items: vec![
                Item {
                    hidden: false,
                    blend_mode: Default::default(),
                    retime: Default::default(),
                    masks: vec![project::Mask::new("mask1", project::MaskShape::Rectangle)],
                    effects: vec![project::EffectInstance {
                        id: "blur1".into(),
                        enabled: true,
                        params: Default::default(),
                    }],
                    id: ItemId(1),
                    animations,
                    source_asset: None,
                    source_range: SourceRange::new(Tick::ZERO, duration).unwrap(),
                    start: Tick::ZERO,
                    content: ItemParams::Overlay(OverlayParams {
                        color: Color([0, 64, 128, 255]),
                    }),
                },
                Item {
                    hidden: false,
                    blend_mode: Default::default(),
                    retime: Default::default(),
                    masks: Vec::new(),
                    effects: Vec::new(),
                    id: ItemId(5),
                    animations: Default::default(),
                    source_asset: Some(media::AssetId(0)),
                    source_range: SourceRange::new(Tick::ZERO, duration).unwrap(),
                    start: duration,
                    content: ItemParams::Video(project::VideoParams {
                        fades: Default::default(),
                        source_audio_enabled: false,
                        volume: 0.0,
                        muted: false,
                    }),
                },
            ],
        },
        Track {
            id: TrackId(3),
            name: "Empty".into(),
            kind: TrackKind::Video,
            muted: false,
            hidden: false,
            locked: false,
            items: vec![],
        },
        Track {
            id: TrackId(4),
            name: "Captions".into(),
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
                id: ItemId(4),
                animations: Default::default(),
                source_asset: Some(media::AssetId(0)),
                source_range: SourceRange::new(Tick::ZERO, duration).unwrap(),
                start: Tick::ZERO,
                content: ItemParams::Caption(project::CaptionParams {
                    cues: captions::CueTrack {
                        cues: vec![captions::Cue {
                            id: "cue-1".into(),
                            range: SourceRange::new(Tick::ZERO, duration).unwrap(),
                            source_lang: captions::LanguageTag::parse("en").unwrap(),
                            texts: [(
                                captions::LanguageTag::parse("en").unwrap(),
                                captions::CueText {
                                    text: "Hello".into(),
                                    edited: false,
                                    translation_ref: None,
                                },
                            )]
                            .into_iter()
                            .collect(),
                            word_ranges: Vec::new(),
                            style: None,
                        }],
                    },
                }),
            }],
        },
    ];
    data.scene_mut().timeline.overlays.reverse();
    data.scene_mut().bookmarks.push(project::Bookmark {
        id: project::BookmarkId(1),
        start: Tick::ZERO,
        duration: Tick::ZERO,
        note: "Mark".into(),
        color: Color([255, 255, 0, 255]),
    });
    Session::new(Project::new(data).unwrap(), 10).unwrap()
}
/// The seed plus one audible clip (track 5, item 5) for audio registry examples.
fn with_audio(seed: &Session) -> Session {
    let mut data = seed.project().data().clone();
    for track in data.scene_mut().timeline.iter_mut() {
        track.items.retain(|item| item.id != ItemId(5));
    }
    let duration = Tick(time::TICKS_PER_SECOND);
    data.assets.push(
        testkit::source::asset(
            97,
            std::env::temp_dir().join("vibecut-control-audio-source.wav"),
        )
        .format("wav")
        .duration(duration)
        .content_hash(media::ContentHash::from_bytes(b"audio fixture"))
        .stream(
            testkit::source::audio()
                .codec("pcm_f32le")
                .time_base(media::TimeBase::new(1, 48000).unwrap())
                .duration(duration)
                .build(),
        )
        .build(),
    );
    data.scene_mut().timeline.insert_track(Track {
        id: TrackId(5),
        name: "Audio".into(),
        kind: TrackKind::Audio,
        muted: false,
        hidden: false,
        locked: false,
        items: vec![Item {
            hidden: false,
            retime: Default::default(),
            blend_mode: Default::default(),
            masks: Vec::new(),
            effects: Vec::new(),
            id: ItemId(5),
            animations: Default::default(),
            source_asset: Some(media::AssetId(97)),
            source_range: SourceRange::new(Tick::ZERO, duration).unwrap(),
            start: Tick::ZERO,
            content: ItemParams::Audio(project::AudioParams {
                fades: Default::default(),
                volume: 0.0,
                muted: false,
                source: Default::default(),
            }),
        }],
    });
    Session::new(Project::new(data).unwrap(), 10).unwrap()
}
/// The seed whose item 5 is a video with an audible source asset.
fn with_source_audio(seed: &Session) -> Session {
    let mut data = seed.project().data().clone();
    let duration = Tick(time::TICKS_PER_SECOND);
    let position = data
        .assets
        .iter()
        .position(|asset| asset.id() == media::AssetId(0))
        .unwrap();
    data.assets[position] = testkit::source::asset(
        0,
        std::env::temp_dir().join("vibecut-control-caption-source.mp4"),
    )
    .format("mp4")
    .duration(duration)
    .content_hash(media::ContentHash::from_bytes(b"fixture"))
    .stream(
        testkit::source::video()
            .time_base(media::TimeBase::new(1, 30000).unwrap())
            .duration(duration)
            .start(Tick::ZERO)
            .size(64, 64)
            .build(),
    )
    .stream(
        testkit::source::audio()
            .index(1)
            .codec("aac")
            .time_base(media::TimeBase::new(1, 48000).unwrap())
            .duration(duration)
            .start(Tick::ZERO)
            .build(),
    )
    .build();
    for track in data.scene_mut().timeline.iter_mut() {
        for item in &mut track.items {
            if item.id == ItemId(5)
                && let ItemParams::Video(params) = &mut item.content
            {
                params.source_audio_enabled = true;
            }
        }
    }
    Session::new(Project::new(data).unwrap(), 10).unwrap()
}
fn replace_paths(v: &mut Value, root: &std::path::Path) {
    match v {
        Value::String(s) if s.starts_with("$output/") => {
            *s = root
                .join(s.trim_start_matches("$output/"))
                .to_str()
                .unwrap()
                .to_owned()
        }
        Value::Object(o) => {
            for v in o.values_mut() {
                replace_paths(v, root)
            }
        }
        Value::Array(a) => {
            for v in a {
                replace_paths(v, root)
            }
        }
        _ => {}
    }
}
fn key() -> engine::cache::CacheKey {
    engine::cache::CacheKey::new(engine::cache::KeyInputs {
        kind: engine::cache::ArtifactKind::Preview,
        source: [0; 32],
        provider: "fixture",
        model: "test",
        version: "1",
        settings: &[],
        transcript: [0; 32],
        cues: [0; 32],
        clip_range: [0; 32],
        translation: [0; 32],
    })
    .unwrap()
}
fn submit(r: &mut JobRunner) -> engine::jobs::JobId {
    r.submit(
        JobSpec {
            kind: JobKind::Import,
            class: ResourceClass::Cpu,
            input: key(),
            bytes: JobBytes::default(),
        },
        |_| Ok(vec![]),
    )
    .unwrap()
    .0
}
fn fake_fonts_list(_: Option<&str>) -> engine::Result<Vec<engine::control::FontEntry>> {
    Ok(vec![engine::control::FontEntry {
        family: "Inter".into(),
        origin: engine::control::FontOrigin::Bundled,
        styles: vec!["Regular".into()],
        state: engine::control::FontState::Available,
        license: Some("OFL-1.1".into()),
        source: None,
        download_bytes: None,
    }])
}
fn fake_fonts_load(family: &str) -> engine::Result<engine::control::FontLoad> {
    Ok(engine::control::FontLoad {
        family: family.into(),
        state: engine::control::FontLoadState::Loaded,
        origin: Some(engine::control::FontOrigin::Bundled),
        effective_family: family.into(),
        styles: vec!["Regular".into()],
    })
}
fn fake_fonts_download(
    runner: &mut JobRunner,
    _: &str,
    accepted: bool,
) -> engine::Result<engine::jobs::JobId> {
    if !accepted {
        return Err(engine::EditError::ConsentRequired);
    }
    Ok(submit(runner))
}
/// Synthetic capability for registry dispatch; real-store coverage lives in `libraries`.
fn font_access() -> engine::control::FontAccess<'static> {
    engine::control::FontAccess::new(&fake_fonts_list, &fake_fonts_load, &fake_fonts_download)
}
fn finish(r: &mut JobRunner, id: engine::jobs::JobId) {
    for _ in 0..10000 {
        r.tick().unwrap();
        if r.snapshot(id).is_some_and(|s| {
            matches!(
                s.state,
                JobState::Completed | JobState::Failed | JobState::Cancelled
            )
        }) {
            return;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    panic!("job deadline");
}

#[test]
fn registry_examples_validate_and_dispatch_without_an_id_table() {
    let root = tempfile::tempdir().unwrap();
    let mut exercised = std::collections::BTreeSet::new();
    for spec in registry() {
        let mut s = seed();
        let mut r = runner();
        let mut params = spec.example.params.clone();
        replace_paths(&mut params, root.path());
        let validator = jsonschema::options()
            .offline()
            .build(&spec.params_schema)
            .unwrap();
        assert!(
            validator.is_valid(&params),
            "{} schema rejects {}: {:?}",
            spec.id,
            params,
            validator.iter_errors(&params).collect::<Vec<_>>()
        );
        let typed: Command =
            serde_json::from_value(json!({"command":spec.id,"params":params})).unwrap();
        assert_eq!(typed.id(), spec.id);
        match spec.example.fixture {
            Fixture::TransitionCut => s = crate::session::transition_session(false),
            Fixture::Transition => s = crate::session::transition_session(true),
            Fixture::Audio => s = with_audio(&s),
            Fixture::SourceAudio => s = with_source_audio(&s),
            Fixture::History => {
                s.apply(Command::RenameProject {
                    name: "History".into(),
                })
                .unwrap();
            }
            Fixture::Redo => {
                s.apply(Command::RenameProject {
                    name: "History".into(),
                })
                .unwrap();
                s.undo().unwrap();
            }
            Fixture::Gesture => {
                s.begin_gesture().unwrap();
            }
            Fixture::Job => {
                submit(&mut r);
            }
            Fixture::Library => {
                let mut library = engine::library::ProjectLibrary::open(
                    &root.path().join(format!("library-{}", spec.id)),
                    ByteBudget::new(64 << 20).unwrap(),
                )
                .unwrap();
                library
                    .execute(
                        project::ProjectId(1),
                        Command::CreateProject {
                            id: project::ProjectId(1),
                            name: "Example".into(),
                        },
                    )
                    .unwrap();
                library.execute(project::ProjectId(1), typed).unwrap();
                exercised.insert(spec.id);
                continue;
            }
            Fixture::Create => {
                assert!(Session::create(typed, 10).is_ok());
                exercised.insert(spec.id);
                continue;
            }
            Fixture::Save => {
                let mut autosave = engine::recovery::Autosave::new(
                    &root.path().join("recovery"),
                    s,
                    ByteBudget::new(512 << 20).unwrap(),
                    Arc::new(SystemClock::default()),
                    engine::recovery::Schedule::default(),
                )
                .unwrap();
                autosave.save_as(root.path().join("saved.vibecut")).unwrap();
                autosave.flush().unwrap();
                autosave.execute(spec.id, params).unwrap();
                autosave.flush().unwrap();
                exercised.insert(spec.id);
                continue;
            }
            _ => {}
        }
        let req = request(&s, spec.id, params, "example");
        let fonts = font_access();
        let result = s
            .control_request(
                req,
                Caller {
                    principal: "ui",
                    external: false,
                    policy: &Allow,
                    fonts: Some(&fonts),
                },
                &mut r,
            )
            .unwrap();
        let value = ok(result);
        if let ControlResult::JobAccepted { job, .. } = value {
            finish(&mut r, job);
            let snap = r.snapshot(job).unwrap();
            assert_eq!(
                snap.state,
                JobState::Completed,
                "{}: {:?}",
                spec.id,
                snap.error
            );
        }
        exercised.insert(spec.id);
    }
    assert_eq!(exercised, registry().iter().map(|s| s.id).collect());
}

#[test]
fn invalid_shape_version_unknown_and_nested_extra_refuse_before_effects() {
    let mut s = seed();
    let mut r = runner();
    let initial = s.project().clone();
    let rev = s.revision();
    let history = s.history_len();
    let mut cases = vec![
        ("absent", json!({}), ErrorCode::UnknownCommand),
        (
            "timeline.add_track",
            json!({"name":"x","kind":"wrong"}),
            ErrorCode::InvalidParams,
        ),
        (
            "timeline.add_track",
            json!({"name":1,"kind":"overlay"}),
            ErrorCode::InvalidParams,
        ),
        (
            "timeline.add_track",
            json!({"name":"x","kind":"video","extra":"secret"}),
            ErrorCode::InvalidParams,
        ),
        ("undo", json!({"extra":true}), ErrorCode::InvalidParams),
    ];
    let mut insert = registry()
        .iter()
        .find(|s| s.id == "timeline.insert")
        .unwrap()
        .example
        .params
        .clone();
    insert["item"]["source_range"]["extra"] = true.into();
    cases.push(("timeline.insert", insert, ErrorCode::InvalidParams));
    for (id, params, code) in cases {
        error(run(&mut s, &mut r, id, params, "invalid"), code);
        assert_eq!(s.project(), &initial);
        assert_eq!(s.revision(), rev);
        assert_eq!(s.history_len(), history);
    }
    for version in [control::API_VERSION - 1, control::API_VERSION + 1] {
        let mut bad = request(
            &s,
            "timeline.add_track",
            json!({"name":"x","kind":"video"}),
            "bad-version",
        );
        bad.api_version = version;
        error(
            s.control_request(bad, caller(&Allow), &mut r).unwrap(),
            ErrorCode::SchemaMismatch,
        );
    }
    let mut bad = request(
        &s,
        "timeline.add_track",
        json!({"name":"x","kind":"video"}),
        "bad-hash",
    );
    bad.registry_hash = Some("old".into());
    error(
        s.control_request(bad, caller(&Allow), &mut r).unwrap(),
        ErrorCode::SchemaMismatch,
    );
    assert_eq!(s.project(), &initial);
    assert_eq!(s.revision(), rev);
    assert_eq!(r.counts().0, 0);
}

#[test]
fn loss_replay_conflict_principal_and_current_authorization() {
    let mut s = session();
    let mut r = runner();
    let req = request(
        &s,
        "timeline.add_track",
        json!({"name":"once","kind":"video"}),
        "once",
    );
    let first = s
        .control_request(req.clone(), caller(&Allow), &mut r)
        .unwrap();
    add(&mut s, &mut r, "second");
    let replay = s
        .control_request(req.clone(), caller(&Allow), &mut r)
        .unwrap();
    assert_eq!(
        serde_json::to_value(first).unwrap(),
        serde_json::to_value(replay).unwrap()
    );
    assert_eq!(s.history_len(), (2, 0));
    error(
        s.control_request(req.clone(), caller(&Deny), &mut r)
            .unwrap(),
        ErrorCode::Permission,
    );
    let mut conflict = req.clone();
    conflict.params["name"] = "different".into();
    error(
        s.control_request(conflict, caller(&Allow), &mut r).unwrap(),
        ErrorCode::IdempotencyConflict,
    );
    error(
        s.control_request(
            req,
            Caller {
                principal: "other",
                external: true,
                policy: &Allow,
                fonts: None,
            },
            &mut r,
        )
        .unwrap(),
        ErrorCode::StaleRevision,
    );
    assert_eq!(s.history_len(), (2, 0));
}

#[test]
fn ledger_exhaustion_has_no_eviction_and_no_effect() {
    let mut s = session();
    let mut r = runner();
    let req = request(&s, "project.rename", json!({"name":"Test"}), "first");
    let first = s
        .control_request(req.clone(), caller(&Allow), &mut r)
        .unwrap();
    for index in 1..control::MAX_RECEIPTS {
        ok(run(
            &mut s,
            &mut r,
            "project.rename",
            json!({"name":"Test"}),
            &index.to_string(),
        ));
    }
    let before = s.revision();
    error(
        run(
            &mut s,
            &mut r,
            "timeline.add_track",
            json!({"name":"never","kind":"video"}),
            "full",
        ),
        ErrorCode::ReceiptCapacity,
    );
    assert!(s.project().data().scene().timeline.overlays.is_empty());
    assert_eq!(s.revision(), before);
    assert_eq!(s.history_len(), (0, 0));
    let replay = s.control_request(req, caller(&Allow), &mut r).unwrap();
    assert_eq!(
        serde_json::to_value(first).unwrap(),
        serde_json::to_value(replay).unwrap()
    );
}

#[test]
fn stale_batch_and_external_gestures_cannot_bypass_admission() {
    let mut s = session();
    let mut r = runner();
    let old = request(
        &s,
        "batch",
        json!({"commands":[{"command":"timeline.add_track","params":{"name":"stale","kind":"video"}}]}),
        "batch",
    );
    add(&mut s, &mut r, "ui");
    let before = s.project().clone();
    error(
        s.control_request(old, caller(&Allow), &mut r).unwrap(),
        ErrorCode::StaleRevision,
    );
    assert_eq!(s.project(), &before);
    for id in [
        "begin_gesture",
        "commit_gesture",
        "cancel_gesture",
        "timeline.preview",
        "timeline.commit_preview",
        "timeline.discard_preview",
    ] {
        error(
            run(
                &mut s,
                &mut r,
                id,
                if id == "timeline.preview" {
                    json!({"commands":[]})
                } else {
                    json!({})
                },
                id,
            ),
            ErrorCode::Permission,
        );
    }
    s.begin_gesture().unwrap();
    error(
        run(&mut s, &mut r, "inspect", json!({}), "inspect-preview"),
        ErrorCode::TransactionActive,
    );
    error(
        run(
            &mut s,
            &mut r,
            "timeline.add_track",
            json!({"name":"blocked","kind":"video"}),
            "gesture-edit",
        ),
        ErrorCode::TransactionActive,
    );
    s.cancel_gesture().unwrap();
    let malformed = json!({"commands":[{"command":"timeline.add_track","params":{"name":"early","kind":"video"}},{"command":"export","params":{"settings":render::OutputOptions::default(),"path":"/never","overwrite":false}}]});
    assert!(matches!(
        run(&mut s, &mut r, "batch", malformed, "mixed").value,
        ResponseValue::Error(_)
    ));
    assert_eq!(s.project(), &before);
    assert_eq!(r.counts().0, 0);
}

#[test]
fn revision_and_job_target_share_ui_save_preview_undo_and_rollback_boundaries() {
    let mut s = session();
    let mut r = runner();
    let first = s.revision();
    let old = s.job_target();
    add(&mut s, &mut r, "edit");
    assert_ne!(s.revision(), first);
    assert!(matches!(
        s.apply_job_result(
            &old,
            Command::RenameProject {
                name: "stale".into()
            }
        ),
        Err(EditError::StaleResult)
    ));
    let edit = s.revision();
    s.undo().unwrap();
    assert_ne!(s.revision(), first);
    assert_ne!(s.revision(), edit);
    s.redo().unwrap();
    let before = s.revision();
    s.saved().unwrap();
    assert_ne!(s.revision(), before);
    let before = s.revision();
    s.transaction(|_| Ok(())).unwrap();
    assert_ne!(s.revision(), before);
    let before = s.revision();
    let result: engine::Result<()> = s.transaction(|s| {
        s.apply(Command::RenameProject {
            name: "preview".into(),
        })?;
        Err(EditError::InvalidParams)
    });
    assert!(result.is_err());
    assert_ne!(s.revision(), before);
    let before = s.revision();
    s.begin_gesture().unwrap();
    s.cancel_gesture().unwrap();
    assert_ne!(s.revision(), before);
    let req = request(
        &s,
        "project.rename",
        json!({"name":"restart"}),
        "old-session",
    );
    let mut next = session();
    error(
        next.control_request(req, caller(&Allow), &mut r).unwrap(),
        ErrorCode::SessionGone,
    );
}

#[test]
fn strict_frame_parser_refuses_duplicate_truncated_hostile_and_unsafe_numbers() {
    for bytes in [
        b"{\"x\":1,\"x\":2}".as_slice(),
        b"{\"x\":{\"a\":1,\"a\":2}}",
        b"{",
        b"[",
        b"\xff",
        b"9007199254740992",
        b"-9007199254740992",
        b"18446744073709551615",
        b"-9223372036854775808",
        b"1e999",
        b"0 trailing",
    ] {
        assert!(control::parse_frame(bytes).is_err(), "{bytes:?}");
    }
    for n in [-control::MAX_SAFE_INTEGER, control::MAX_SAFE_INTEGER] {
        assert_eq!(
            control::parse_frame(n.to_string().as_bytes())
                .unwrap()
                .as_i64(),
            Some(n)
        );
    }
    assert!(control::parse_frame(&vec![b' '; control::MAX_FRAME_BYTES + 1]).is_err());
    assert!(
        control::parse_frame(format!("{}0{}", "[".repeat(34), "]".repeat(34)).as_bytes()).is_err()
    );
    assert!(control::parse_frame(format!("\"{}\"", "a".repeat((4 << 20) + 1)).as_bytes()).is_err());
    let mut s = session();
    let rev = s.revision();
    assert!(
        s.execute(
            "add_bookmark",
            json!({"start":control::MAX_SAFE_INTEGER+1,"duration":0,"note":"x","color":[0,0,0,0]})
        )
        .is_err()
    );
    assert_eq!(s.revision(), rev);
}

#[test]
fn snapshot_barrier_slow_listener_and_preview_never_claim_committed_change() {
    let mut s = session();
    let mut r = runner();
    let mut snapshot = s.snapshot_and_subscribe(&r).unwrap();
    add(&mut s, &mut r, "first");
    let events = s.events(&mut snapshot.cursor).unwrap();
    assert!(
        events
            .iter()
            .any(|e| matches!(e.payload, Payload::ProjectChanged { .. }))
    );
    assert!(s.events(&mut snapshot.cursor).unwrap().is_empty());
    let mut slow = s.snapshot_and_subscribe(&r).unwrap();
    for index in 0..control::MAX_EVENTS {
        add(&mut s, &mut r, &format!("pressure-{index}"));
    }
    assert!(matches!(
        s.events(&mut slow.cursor),
        Err(EditError::ResyncRequired)
    ));
    let mut fresh = s.snapshot_and_subscribe(&r).unwrap();
    s.begin_gesture().unwrap();
    s.apply(Command::RenameProject {
        name: "preview".into(),
    })
    .unwrap();
    assert!(s.snapshot_and_subscribe(&r).is_err());
    assert!(
        s.events(&mut fresh.cursor)
            .unwrap()
            .iter()
            .all(|e| !matches!(e.payload, Payload::ProjectChanged { .. }))
    );
    s.cancel_gesture().unwrap();
}

#[test]
fn job_acceptance_precedes_events_cancel_and_result_eviction_never_resubmit() {
    let mut s = seed();
    let mut r = runner();
    let mut snapshot = s.snapshot_and_subscribe(&r).unwrap();
    let req = request(&s, "preview", json!({"frames":[0]}), "preview");
    let accepted = s
        .control_request(req.clone(), caller(&Allow), &mut r)
        .unwrap();
    let job = match ok(accepted) {
        ControlResult::JobAccepted { job, .. } => job,
        _ => panic!("expected job"),
    };
    s.observe_jobs(&r);
    let events = s.events(&mut snapshot.cursor).unwrap();
    let accepted = events
        .iter()
        .position(|e| matches!(e.payload, Payload::CommandResult { .. }))
        .unwrap();
    let changed = events
        .iter()
        .position(|e| matches!(e.payload, Payload::JobChanged { .. }))
        .unwrap();
    assert!(accepted < changed);
    finish(&mut r, job);
    let state = r.snapshot(job).unwrap();
    assert_eq!(state.state, JobState::Completed, "{:?}", state.error);
    let preview = match ok(run(
        &mut s,
        &mut r,
        "job_status",
        json!({"job":job.0}),
        "poll",
    )) {
        ControlResult::Job { snapshot } => snapshot.result.unwrap(),
        _ => panic!("expected preview"),
    };
    let artifact = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/fixtures/control/preview.jpg");
    std::fs::create_dir_all(artifact.parent().unwrap()).unwrap();
    std::fs::write(&artifact, &preview.frames[0].jpeg).unwrap();
    let decoded = image::load_from_memory(&preview.frames[0].jpeg).unwrap();
    assert_eq!(decoded.width(), preview.frames[0].width);
    assert_eq!(decoded.height(), preview.frames[0].height);
    println!(
        "preview_jpeg_bytes={} preview_result_json_bytes={}",
        preview.frames[0].jpeg.len(),
        serde_json::to_vec(&preview).unwrap().len()
    );
    assert_eq!(preview.frames.len(), 1);
    assert!(preview.frames[0].width <= 640 && preview.frames[0].height <= 640);
    assert_eq!(preview.frames[0].clips, vec![ItemId(1), ItemId(4)]);
    assert_eq!(&preview.frames[0].jpeg[..2], &[255, 216]);
    s.observe_jobs(&r);
    let terminal = s.events(&mut snapshot.cursor).unwrap();
    assert!(terminal.iter().any(|e|matches!(&e.payload,Payload::JobChanged{snapshot} if snapshot.state==JobState::Completed)));
    s.observe_jobs(&r);
    assert!(s.events(&mut snapshot.cursor).unwrap().is_empty());
    for _ in 0..20 {
        let id = submit(&mut r);
        finish(&mut r, id);
    }
    error(
        run(
            &mut s,
            &mut r,
            "job_status",
            json!({"job":job.0}),
            "evicted",
        ),
        ErrorCode::JobNotRetained,
    );
    let count = r.counts();
    assert!(
        matches!(ok(s.control_request(req,caller(&Allow),&mut r).unwrap()),ControlResult::JobAccepted{job:id,..} if id==job)
    );
    assert_eq!(r.counts(), count);
    let cancel = submit(&mut r);
    ok(run(
        &mut s,
        &mut r,
        "job_cancel",
        json!({"job":cancel.0}),
        "cancel",
    ));
    finish(&mut r, cancel);
    assert_eq!(r.snapshot(cancel).unwrap().state, JobState::Cancelled);
}

#[test]
fn all_schema_shapes_agree_on_required_fields_wrong_types_and_closed_objects() {
    for spec in registry() {
        let schema = jsonschema::options()
            .offline()
            .build(&spec.params_schema)
            .unwrap();
        let params = &spec.example.params;
        assert!(schema.is_valid(params), "{}", spec.id);
        let mut extra = params.clone();
        extra
            .as_object_mut()
            .unwrap()
            .insert("untrusted-secret".into(), json!(true));
        assert!(!schema.is_valid(&extra), "{}", spec.id);
        assert!(
            serde_json::from_value::<Command>(json!({"command":spec.id,"params":extra})).is_err()
        );
        for field in spec.params_schema["required"]
            .as_array()
            .into_iter()
            .flatten()
        {
            let mut missing = params.clone();
            missing
                .as_object_mut()
                .unwrap()
                .remove(field.as_str().unwrap());
            assert!(!schema.is_valid(&missing), "{} missing {field}", spec.id);
            assert!(
                serde_json::from_value::<Command>(json!({"command":spec.id,"params":missing}))
                    .is_err(),
                "{} missing {field}",
                spec.id
            );
        }
    }
}

#[test]
fn decimal_project_identity_is_one_encoding_and_old_files_refuse() {
    for id in [
        0,
        control::MAX_SAFE_INTEGER as u64,
        (control::MAX_SAFE_INTEGER as u64) + 1,
        u64::MAX,
    ] {
        let id = ProjectId(id);
        let wire = serde_json::to_string(&id).unwrap();
        assert_eq!(wire, format!("\"{}\"", id.0));
        assert_eq!(serde_json::from_str::<ProjectId>(&wire).unwrap(), id);
    }
    for bad in ["0", "\"01\"", "\"+1\"", "\"18446744073709551616\""] {
        assert!(serde_json::from_str::<ProjectId>(bad).is_err());
    }
    let bytes = project::encode(session().project(), false).unwrap();
    let mut value: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["project"]["id"], u64::MAX.to_string());
    value["schema_version"] = json!(2);
    assert!(matches!(
        project::decode(&serde_json::to_vec(&value).unwrap()),
        Err(project::Error::TooOld { .. })
    ));
}

#[test]
fn policy_refusal_and_nested_policy_are_safe_before_io() {
    let mut s = session();
    let mut r = runner();
    let mut requests = vec![request(
        &s,
        "export",
        json!({"settings":render::OutputOptions::default(),"path":"/secret/private","overwrite":true}),
        "denied",
    )];
    requests.push(request(&s,"batch",json!({"commands":[{"command":"timeline.add_track","params":{"name":"private text","kind":"video"}}]}),"batch-denied"));
    let before = s.project().clone();
    for req in requests {
        let result = s.control_request(req, caller(&Deny), &mut r).unwrap();
        error(result.clone(), ErrorCode::Permission);
        let diagnostic = serde_json::to_string(&result).unwrap();
        assert!(!diagnostic.contains("secret") && !diagnostic.contains("private text"));
    }
    assert_eq!(s.project(), &before);
    assert_eq!(r.counts().0, 0);
}

#[test]
fn deterministic_catalog_and_schema_document_receipts() {
    let first = serde_json::to_vec(registry()).unwrap();
    let second = serde_json::to_vec(registry()).unwrap();
    assert_eq!(first, second);
    assert_eq!(control::registry_hash().len(), 64);
    for spec in registry() {
        assert_eq!(
            spec.params_schema["$schema"],
            "https://json-schema.org/draft/2020-12/schema"
        );
        assert!(
            jsonschema::options()
                .offline()
                .build(&spec.output_schema)
                .is_ok()
        );
    }
    println!(
        "catalog_bytes={} registry_hash={}",
        first.len(),
        control::registry_hash()
    );
    println!(
        "representative_project_bytes={}",
        control::bounded_json(seed().project().data(), control::MAX_RESPONSE_BYTES)
            .unwrap()
            .len()
    );
    assert!(first.len() < control::MAX_RESPONSE_BYTES);
    let mut job_control: Vec<_> = registry()
        .iter()
        .filter(|s| s.effect == Effect::JobControl)
        .map(|s| s.id)
        .collect();
    job_control.sort_unstable();
    assert_eq!(job_control, ["fonts.download", "job_cancel", "preview"]);
    // Each fact rides on its declaration line: a renamed id keeps its rule, and a
    // command without a grant class (or a publisher without a path) fails here.
    use engine::{Availability, GrantClass, Lifecycle};
    let ids = |pick: &dyn Fn(&engine::CommandSpec) -> bool| {
        let mut ids: Vec<_> = registry()
            .iter()
            .filter(|s| pick(s))
            .map(|s| s.id)
            .collect();
        ids.sort_unstable();
        ids
    };
    for (rule, expected) in [
        (
            Availability::CaptionItem,
            &["burn_subtitles", "export_subtitles"][..],
        ),
        (
            Availability::NoTransaction,
            &["begin_gesture", "timeline.set_ripple"],
        ),
        (
            Availability::GestureActive,
            &[
                "cancel_gesture",
                "commit_gesture",
                "timeline.commit_preview",
                "timeline.discard_preview",
            ],
        ),
        (Availability::PlayableTimeline, &["preview.snapshot"]),
        (Availability::CanUndo, &["undo"]),
        (Availability::CanRedo, &["redo"]),
        (Availability::Assets, &["media.remove"]),
        (Availability::RetimableItem, &["timeline.set_speed"]),
        (Availability::RemovableTrack, &["track.remove"]),
        (
            Availability::VisualItem,
            &[
                "effects.add",
                "effects.remove",
                "effects.reorder",
                "effects.toggle",
                "effects.update_param",
                "masks.add",
                "masks.remove",
                "masks.reorder",
                "masks.update",
            ],
        ),
        (
            Availability::UnlockedItem,
            &[
                "audio.set_fade",
                "delete",
                "keyframes.add",
                "keyframes.delete_selection",
                "keyframes.remove",
                "keyframes.retime",
                "keyframes.scale_time",
                "keyframes.set_extrapolation",
                "keyframes.set_handle",
                "keyframes.set_param",
                "keyframes.toggle_at",
                "move_item",
                "move_items",
                "retranslate",
                "ripple_delete",
                "set_item_param",
                "split_at",
                "timeline.duplicate",
                "timeline.move_items",
                "timeline.set_hidden",
                "timeline.set_item_param",
                "timeline.set_muted",
                "timeline.split_left",
                "timeline.split_right",
                "timeline.trim",
                "trim_end",
                "trim_start",
            ],
        ),
        (
            Availability::VideoItem,
            &["timeline.extract_audio", "timeline.recover_audio"],
        ),
    ] {
        assert_eq!(ids(&|s| s.availability == rule), expected, "{rule:?}");
    }
    assert_eq!(
        ids(&|s| s.grant == Some(GrantClass::Human)),
        [
            "begin_gesture",
            "cancel_gesture",
            "captions.import",
            "commit_gesture",
            "fonts.download",
            "import.add_asset",
            "project.create",
            "project.delete",
            "project.duplicate",
            "project.thumbnail",
            "sounds.add_to_timeline",
            "timeline.commit_preview",
            "timeline.discard_preview",
            "timeline.preview",
        ]
    );
    assert_eq!(
        ids(&|s| s.grant == Some(GrantClass::Output)),
        [
            "burn_subtitles",
            "export",
            "export_subtitles",
            "preview.snapshot"
        ]
    );
    assert_eq!(ids(&|s| s.grant.is_none()), Vec::<&str>::new());
    assert_eq!(
        ids(&|s| s.curated),
        [
            "batch",
            "export",
            "inspect",
            "job_cancel",
            "job_status",
            "preview",
            "redo",
            "undo"
        ]
    );
    assert_eq!(
        ids(&|s| s.lifecycle.is_some()),
        [
            "project.create",
            "project.delete",
            "project.duplicate",
            "project.save",
            "project.thumbnail",
        ]
    );
    assert_eq!(engine::lifecycle("project.save"), Some(Lifecycle::Save));
    for spec in registry()
        .iter()
        .filter(|s| s.effect == engine::Effect::FilePublish)
    {
        let command: Command =
            serde_json::from_value(json!({"command":spec.id,"params":spec.example.params.clone()}))
                .unwrap();
        assert!(
            command.published_path().is_some(),
            "{} has no output path",
            spec.id
        );
    }
}

#[test]
fn concurrent_duplicate_requests_execute_once_under_the_session_boundary() {
    let s = session();
    let req = request(
        &s,
        "timeline.add_track",
        json!({"name":"Concurrent","kind":"video"}),
        "concurrent",
    );
    let state = std::sync::Mutex::new((s, runner()));
    let values = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..8)
            .map(|_| {
                scope.spawn(|| {
                    let mut guard = state.lock().unwrap();
                    let (session, runner) = &mut *guard;
                    serde_json::to_value(
                        session
                            .control_request(req.clone(), caller(&Allow), runner)
                            .unwrap(),
                    )
                    .unwrap()
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert!(values.windows(2).all(|pair| pair[0] == pair[1]));
    let guard = state.lock().unwrap();
    assert_eq!(guard.0.history_len(), (1, 0));
    assert_eq!(guard.0.project().data().scene().timeline.overlays.len(), 1);
}

#[test]
fn every_child_failure_restores_content_history_revision_and_ids() {
    for failed in 0..4 {
        let mut s = session();
        let mut r = runner();
        let before = s.project().clone();
        let revision = s.revision();
        let mut listener = s.snapshot_and_subscribe(&r).unwrap();
        let children: Vec<_> = (0..4)
            .map(|index| {
                if index == failed {
                    json!({"command":"track.remove","params":{"track":1000}})
                } else {
                    json!({"command":"timeline.add_track","params":{"name":"Candidate","kind":"video"}})
                }
            })
            .collect();
        assert!(matches!(
            run(
                &mut s,
                &mut r,
                "batch",
                json!({"commands":children}),
                "failed"
            )
            .value,
            ResponseValue::Error(_)
        ));
        assert_eq!(s.project(), &before);
        assert_eq!(s.revision(), revision);
        assert_eq!(s.history_len(), (0, 0));
        assert!(
            s.events(&mut listener.cursor)
                .unwrap()
                .iter()
                .all(|e| !matches!(e.payload, Payload::ProjectChanged { .. }))
        );
        let result = ok(run(
            &mut s,
            &mut r,
            "timeline.add_track",
            json!({"name":"After","kind":"video"}),
            "after",
        ));
        assert!(
            matches!(result,ControlResult::Edit{outcome} if outcome.created==vec![engine::CreatedId::Track(TrackId(1))])
        );
    }
}

#[test]
fn external_gesture_ownership_is_also_enforced_inside_generic_batch() {
    let mut s = session();
    let mut r = runner();
    let before = s.revision();
    for id in [
        "begin_gesture",
        "commit_gesture",
        "cancel_gesture",
        "timeline.preview",
        "timeline.commit_preview",
        "timeline.discard_preview",
    ] {
        assert!(matches!(
            run(
                &mut s,
                &mut r,
                "batch",
                json!({"commands":[{"command":id,"params":if id == "timeline.preview" {json!({"commands":[]})} else {json!({})}}]}),
                id
            )
            .value,
            ResponseValue::Error(_)
        ));
    }
    assert_eq!(s.revision(), before);
    assert_eq!(s.history_len(), (0, 0));
    let mut snapshot = s.snapshot_and_subscribe(&r).unwrap();
    s.begin_gesture().unwrap();
    s.apply(Command::RenameProject {
        name: "preview".into(),
    })
    .unwrap();
    s.cancel_gesture().unwrap();
    assert!(
        s.events(&mut snapshot.cursor)
            .unwrap()
            .iter()
            .all(|e| !matches!(e.payload, Payload::ProjectChanged { .. }))
    );
}

#[test]
fn parser_exact_collection_node_depth_and_text_limits() {
    let array = json!(vec![0; 100_000]);
    assert!(control::parse_frame(&serde_json::to_vec(&array).unwrap()).is_ok());
    let array = json!(vec![0; 100_001]);
    assert!(control::parse_frame(&serde_json::to_vec(&array).unwrap()).is_err());
    let value = json!([vec![0; 99_998], vec![0; 99_999]]);
    assert!(control::parse_frame(&serde_json::to_vec(&value).unwrap()).is_ok());
    let value = json!([vec![0; 99_999], vec![0; 99_999]]);
    assert!(control::parse_frame(&serde_json::to_vec(&value).unwrap()).is_err());
    assert!(
        control::parse_frame(format!("{}0{}", "[".repeat(32), "]".repeat(32)).as_bytes()).is_ok()
    );
    assert!(
        control::parse_frame(format!("{}0{}", "[".repeat(33), "]".repeat(33)).as_bytes()).is_err()
    );
    assert!(control::parse_frame(format!("\"{}\"", "a".repeat(4 << 20)).as_bytes()).is_ok());
}

#[test]
fn malformed_envelope_and_batch_depth_have_no_effect() {
    let mut s = session();
    let mut r = runner();
    let initial = s.project().clone();
    let revision = s.revision();
    let req = request(
        &s,
        "timeline.add_track",
        json!({"name":"x","kind":"video"}),
        "shape",
    );
    let mut value = serde_json::to_value(req).unwrap();
    value["caller"] = "self-authorized".into();
    assert!(
        s.control_json(&serde_json::to_vec(&value).unwrap(), caller(&Allow), &mut r)
            .is_err()
    );
    let command = json!({"command":"timeline.add_track","params":{"name":"x","kind":"video"}});
    for count in [1024, 1025] {
        let batch = json!({"commands":vec![command.clone();count]});
        let result = run(&mut s, &mut r, "batch", batch, &format!("count-{count}"));
        // Root batch counts as one command: 1024 children exceed the total budget.
        error(result, ErrorCode::Limit);
    }
    let mut nested = command;
    for _ in 0..17 {
        nested = json!({"command":"batch","params":{"commands":[nested]}});
    }
    let req = request(&s, "batch", json!({"commands":[nested]}), "deep");
    assert!(s.control_request(req, caller(&Allow), &mut r).is_err());
    assert_eq!(s.project(), &initial);
    assert_eq!(s.revision(), revision);
    assert_eq!(r.counts().0, 0);
}

#[test]
fn checked_in_registry_digest_and_command_reference_match_the_registry() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    assert_eq!(
        std::fs::read_to_string(root.join("catalog.digest"))
            .unwrap()
            .replace("\r\n", "\n"),
        format!("{}\n", control::registry_hash()),
        "regenerate with cargo xtask catalog"
    );
    assert_eq!(
        std::fs::read_to_string(root.join("../../docs/product/agent-control/commands.md"))
            .unwrap()
            .replace("\r\n", "\n"),
        control::reference_markdown().unwrap(),
        "regenerate with cargo xtask catalog"
    );
}

#[test]
fn labelled_preview_preserves_production_picture_outside_the_label_band() {
    let mut s = seed();
    let created = s
        .apply(Command::CreateScene {
            name: "Inactive".into(),
        })
        .unwrap();
    let engine::CreatedId::Scene(scene) = created.created[0] else {
        panic!("scene");
    };
    s.apply(Command::SwitchScene { scene }).unwrap();
    s.apply(Command::AddTrack {
        name: "Inactive overlay".into(),
        kind: TrackKind::Video,
        placement: Default::default(),
    })
    .unwrap();
    s.apply(Command::SwitchScene {
        scene: project::SceneId(0),
    })
    .unwrap();
    let mut r = runner();
    let project = s.project().clone();
    let (reference, _) = r
        .submit(
            JobSpec {
                kind: JobKind::Preview,
                class: ResourceClass::Cpu,
                input: key(),
                bytes: JobBytes {
                    result: 640 * 640 * 4,
                    ..Default::default()
                },
            },
            move |context| {
                let mut settings = render::RenderSettings::from_project(&project);
                settings.width = 640;
                settings.height = 640;
                let mut renderer = render::Renderer::new(&project, settings)?;
                let mut sources = render::FileSources::default();
                Ok(renderer
                    .render_frame(Tick::ZERO, &mut sources, context)?
                    .video_rgba8(13, context)?
                    .bytes)
            },
        )
        .unwrap();
    finish(&mut r, reference);
    let baseline = r.output_bytes(reference).unwrap().to_vec();
    let job = match ok(run(
        &mut s,
        &mut r,
        "preview",
        json!({"frames":[0]}),
        "quality",
    )) {
        ControlResult::JobAccepted { job, .. } => job,
        _ => panic!("preview job"),
    };
    finish(&mut r, job);
    let result = match ok(run(
        &mut s,
        &mut r,
        "job_status",
        json!({"job":job.0}),
        "picture",
    )) {
        ControlResult::Job { snapshot } => snapshot.result.unwrap(),
        _ => panic!("preview result"),
    };
    let decoded = image::load_from_memory(&result.frames[0].jpeg)
        .unwrap()
        .to_rgb8();
    let mut error = 0u64;
    let mut channels = 0u64;
    let mut white = 0usize;
    let mut retained = 0usize;
    for (index, pixel) in baseline
        .as_chunks::<4>()
        .0
        .iter()
        .enumerate()
        .skip(96 * 640)
    {
        let actual = decoded.as_raw().get(index * 3..index * 3 + 3).unwrap();
        for (expected, actual) in pixel.iter().take(3).zip(actual) {
            error += u64::from(expected.abs_diff(*actual));
            channels += 1;
        }
        if pixel.iter().take(3).all(|v| *v >= 240) {
            white += 1;
            if actual.iter().all(|v| *v >= 200) {
                retained += 1;
            }
        }
    }
    let mae = error as f64 / channels as f64;
    println!("preview_body_rgb_mae={mae:.6} caption_white_pixels={white} retained={retained}");
    assert!(mae < 2.0, "preview changed production picture: {mae}");
    assert!(white > 10, "caption fixture must be visible");
    assert!(retained * 10 >= white * 9, "preview obscured caption");
}

#[test]
fn preview_four_frame_limit_is_executable_and_five_frames_start_no_job() {
    let mut s = seed();
    let mut r = runner();
    error(
        run(
            &mut s,
            &mut r,
            "preview",
            json!({"frames":[0,1,2,3,4]}),
            "too-many",
        ),
        ErrorCode::Limit,
    );
    assert_eq!(r.counts().0, 0);
    let revision = s.revision();
    let job = match ok(run(
        &mut s,
        &mut r,
        "preview",
        json!({"frames":[0,1,2,3]}),
        "four",
    )) {
        ControlResult::JobAccepted { job, .. } => job,
        _ => panic!("preview job"),
    };
    finish(&mut r, job);
    let result = match ok(run(
        &mut s,
        &mut r,
        "job_status",
        json!({"job":job.0}),
        "four-result",
    )) {
        ControlResult::Job { snapshot } => snapshot.result.unwrap(),
        _ => panic!("preview result"),
    };
    assert_eq!(result.frames.len(), 4);
    assert_eq!(result.revision, revision);
    for (frame, actual) in result.frames.iter().enumerate() {
        assert_eq!(actual.frame, frame as i64);
        assert_eq!(
            actual.output_tick,
            FrameRate::FPS_30.tick_of(frame as i64).unwrap()
        );
    }
    println!(
        "four_frame_preview_json_bytes={}",
        serde_json::to_vec(&result).unwrap().len()
    );
}

#[test]
fn diagnostic_debug_redacts_request_content_output_paths_and_frame_bytes() {
    let s = session();
    let request = request(
        &s,
        "export",
        json!({"path":"/private-secret-path","text":"private-secret-text"}),
        "private-secret-key",
    );
    assert!(!format!("{request:?}").contains("private-secret"));
    let result = ControlResult::JobAccepted {
        job: engine::jobs::JobId(0),
        revision: s.revision(),
        paths: vec!["/private-secret-path".into()],
    };
    assert!(!format!("{result:?}").contains("private-secret"));
    let frame = control::PreviewFrame {
        frame: 0,
        output_tick: Tick::ZERO,
        clips: vec![],
        width: 1,
        height: 1,
        jpeg: b"private-secret-frame".to_vec(),
    };
    assert!(!format!("{frame:?}").contains("private-secret"));
}

fn assert_integer_candidate_refusal(s: &mut Session, r: &mut JobRunner, id: &str, params: Value) {
    let before = s.project().clone();
    let history = s.history_len();
    let revision = s.revision();
    let metadata = s.metadata().clone();
    let mut listener = s.snapshot_and_subscribe(r).unwrap();
    let req = request(s, id, params, "integer-candidate");
    let response = s.control_request(req.clone(), caller(&Allow), r).unwrap();
    error(response.clone(), ErrorCode::InvalidParams);
    assert_eq!(s.project(), &before);
    assert_eq!(s.history_len(), history);
    assert_eq!(s.revision(), revision);
    assert_eq!(s.metadata(), &metadata);
    assert!(
        s.events(&mut listener.cursor)
            .unwrap()
            .iter()
            .all(|e| !matches!(
                e.payload,
                Payload::ProjectChanged { .. } | Payload::RevisionChanged
            ))
    );
    assert!(s.take_change().is_none());
    ok(run(s, r, "inspect", json!({}), "inspect-after-refusal"));
    assert!(
        jsonschema::options()
            .offline()
            .build(&control::contract_schema()["response"])
            .unwrap()
            .is_valid(&serde_json::to_value(&response).unwrap())
    );
    s.apply(Command::RenameProject {
        name: "Later revision".into(),
    })
    .unwrap();
    let after = s.project().clone();
    let replay = s.control_request(req, caller(&Allow), r).unwrap();
    assert_eq!(
        serde_json::to_value(replay).unwrap(),
        serde_json::to_value(response).unwrap()
    );
    assert_eq!(s.project(), &after);
}

#[test]
fn integer_candidates_at_the_safe_limit_refuse_atomically_through_every_entry() {
    // A generated id at the safe limit refuses the next allocation.
    let mut data = session().project().data().clone();
    data.scene_mut().timeline.insert_track(Track {
        id: TrackId(control::MAX_SAFE_INTEGER as u64),
        name: "Boundary".into(),
        kind: TrackKind::Video,
        muted: false,
        hidden: false,
        locked: false,
        items: vec![],
    });
    let mut generated = Session::new(Project::new(data).unwrap(), 10).unwrap();
    let mut r = runner();
    ok(run(&mut generated, &mut r, "inspect", json!({}), "before"));
    assert_integer_candidate_refusal(
        &mut generated,
        &mut r,
        "timeline.add_track",
        json!({"name":"Next","kind":"video"}),
    );

    // Derived source ticks refuse through one command and through a batch.
    let mut data = session().project().data().clone();
    data.scene_mut().timeline.insert_track(Track {
        id: TrackId(1),
        name: "Boundary".into(),
        kind: TrackKind::Video,
        muted: false,
        hidden: false,
        locked: false,
        items: vec![Item {
            hidden: false,
            blend_mode: Default::default(),
            retime: Default::default(),
            masks: Vec::new(),
            effects: Vec::new(),
            id: ItemId(1),
            animations: Default::default(),
            source_asset: None,
            source_range: SourceRange::new(
                Tick(control::MAX_SAFE_INTEGER - 10),
                Tick(control::MAX_SAFE_INTEGER),
            )
            .unwrap(),
            start: Tick::ZERO,
            content: ItemParams::Overlay(OverlayParams {
                color: Color([0, 0, 0, 255]),
            }),
        }],
    });
    for batch in [false, true] {
        let mut s = Session::new(Project::new(data.clone()).unwrap(), 10).unwrap();
        let mut r = runner();
        ok(run(&mut s, &mut r, "inspect", json!({}), "before"));
        if batch {
            assert_integer_candidate_refusal(
                &mut s,
                &mut r,
                "batch",
                json!({"commands":[{"command":"project.rename","params":{"name":"Must roll back"}},{"command":"timeline.add_track","params":{"name":"Must not consume ID","kind":"video"}},{"command":"trim_end","params":{"item":1,"at":20}}]}),
            );
            let result = ok(run(
                &mut s,
                &mut r,
                "timeline.add_track",
                json!({"name":"After refusal","kind":"video"}),
                "after-batch",
            ));
            assert!(
                matches!(result,ControlResult::Edit{outcome} if outcome.created==vec![engine::CreatedId::Track(TrackId(2))])
            );
        } else {
            assert_integer_candidate_refusal(&mut s, &mut r, "trim_end", json!({"item":1,"at":20}));
        }
    }

    // The last safe created id still matches the output schema; the next id refuses.
    let mut data = session().project().data().clone();
    data.scene_mut().timeline.insert_track(Track {
        id: TrackId(control::MAX_SAFE_INTEGER as u64 - 1),
        name: "Boundary".into(),
        kind: TrackKind::Video,
        muted: false,
        hidden: false,
        locked: false,
        items: vec![],
    });
    let mut last_safe = Session::new(Project::new(data).unwrap(), 10).unwrap();
    let mut r = runner();
    let result = ok(run(
        &mut last_safe,
        &mut r,
        "timeline.add_track",
        json!({"name":"Last safe","kind":"video"}),
        "last-safe",
    ));
    assert!(
        matches!(&result,ControlResult::Edit{outcome} if outcome.created==vec![engine::CreatedId::Track(TrackId(control::MAX_SAFE_INTEGER as u64))])
    );
    let schema = &registry()
        .iter()
        .find(|spec| spec.id == "timeline.add_track")
        .unwrap()
        .output_schema;
    assert!(
        jsonschema::options()
            .offline()
            .build(schema)
            .unwrap()
            .is_valid(&serde_json::to_value(result).unwrap())
    );
    last_safe.take_change();
    assert_integer_candidate_refusal(
        &mut last_safe,
        &mut r,
        "timeline.add_track",
        json!({"name":"Unsafe","kind":"video"}),
    );

    // Derived keyframe ticks refuse before commit.
    let mut data = seed().project().data().clone();
    let path = project::AnimationPath::new("opacity").unwrap();
    data.scene_mut()
        .timeline
        .iter_mut()
        .flat_map(|track| &mut track.items)
        .find(|item| item.id == ItemId(1))
        .unwrap()
        .animations
        .get_mut(&path)
        .unwrap()
        .channel_mut(None)
        .unwrap()
        .put(project::Keyframe::new(
            Tick(control::MAX_SAFE_INTEGER / 2 + 1),
            project::ParamValue::Scalar(0.5),
        ))
        .unwrap();
    let mut keyframes = Session::new(Project::new(data).unwrap(), 10).unwrap();
    let mut r = runner();
    ok(run(&mut keyframes, &mut r, "inspect", json!({}), "before"));
    assert_integer_candidate_refusal(
        &mut keyframes,
        &mut r,
        "keyframes.scale_time",
        json!({"item":1,"numerator":2,"denominator":1}),
    );

    // History destinations and legacy JSON share the same candidate validation.
    let mut data = session().project().data().clone();
    data.scene_mut().timeline.insert_track(Track {
        id: TrackId(1),
        name: "Boundary".into(),
        kind: TrackKind::Video,
        muted: false,
        hidden: false,
        locked: false,
        items: vec![Item {
            hidden: false,
            blend_mode: Default::default(),
            retime: Default::default(),
            masks: Vec::new(),
            effects: Vec::new(),
            id: ItemId(1),
            animations: Default::default(),
            source_asset: None,
            source_range: SourceRange::new(
                Tick(control::MAX_SAFE_INTEGER - 10),
                Tick(control::MAX_SAFE_INTEGER),
            )
            .unwrap(),
            start: Tick::ZERO,
            content: ItemParams::Overlay(OverlayParams {
                color: Color([0, 0, 0, 255]),
            }),
        }],
    });
    let mut history = Session::new(Project::new(data).unwrap(), 10).unwrap();
    let mut r = runner();
    let before = history.project().clone();
    let revision = history.revision();
    assert!(matches!(
        history.execute("trim_end", json!({"item":1,"at":20})),
        Err(EditError::InvalidParams)
    ));
    assert_eq!(history.project(), &before);
    assert_eq!(history.revision(), revision);
    history
        .apply(Command::TrimEnd {
            item: ItemId(1),
            at: Tick(20),
        })
        .unwrap();
    history.undo().unwrap();
    history.take_change();
    assert_eq!(history.project(), &before);
    assert_eq!(history.history_len(), (0, 1));
    assert_integer_candidate_refusal(&mut history, &mut r, "redo", json!({}));
}

#[test]
fn lifecycle_save_uses_receipts_and_retries_after_publication_failure() {
    let mut s = session();
    let initial = s.revision();
    let mut published = 0;
    let response = s
        .control_lifecycle_save(
            request_at(&s, "save-1", "project.save", json!({}), "save", &initial),
            caller(&Allow),
            |project, _metadata| {
                assert_eq!(project.data().name.as_str(), "Test");
                published += 1;
                Ok(())
            },
        )
        .unwrap();
    assert_eq!(published, 1);
    let ControlResult::Edit { outcome } = ok(response) else {
        panic!("save result");
    };
    assert!(!outcome.changed);
    let saved = s.revision();
    assert_ne!(saved, initial);

    let mut replayed = 0;
    let replay = s
        .control_lifecycle_save(
            request_at(
                &s,
                "save-replay",
                "project.save",
                json!({}),
                "save",
                &initial,
            ),
            caller(&Allow),
            |_, _| {
                replayed += 1;
                Ok(())
            },
        )
        .unwrap();
    assert_eq!(replayed, 0);
    let ControlResult::Edit { outcome } = ok(replay.clone()) else {
        panic!("replay result");
    };
    assert!(!outcome.changed);
    assert_eq!(replay.rev_after, saved);

    let conflict = s
        .control_lifecycle_save(
            request_at(
                &s,
                "save-conflict",
                "project.save",
                json!({}),
                "save",
                &s.revision(),
            ),
            caller(&Allow),
            |_, _| panic!("conflicting save must not publish"),
        )
        .unwrap();
    error(conflict, ErrorCode::IdempotencyConflict);

    let before_failure = s.revision();
    let mut failure_publishes = 0;
    let failure = s
        .control_lifecycle_save(
            request_at(
                &s,
                "save-fail",
                "project.save",
                json!({}),
                "fail",
                &before_failure,
            ),
            caller(&Allow),
            |_, _| {
                failure_publishes += 1;
                Err(EditError::Io(std::io::Error::other("disk")))
            },
        )
        .unwrap();
    assert_eq!(failure_publishes, 1);
    error(failure, ErrorCode::Failed);
    assert_eq!(s.revision(), before_failure);

    let mut replay_publishes = 0;
    let replay_failure = s
        .control_lifecycle_save(
            request_at(
                &s,
                "save-fail-replay",
                "project.save",
                json!({}),
                "fail",
                &before_failure,
            ),
            caller(&Allow),
            |_, _| {
                replay_publishes += 1;
                Ok(())
            },
        )
        .unwrap();
    assert_eq!(replay_publishes, 0);
    error(replay_failure, ErrorCode::Failed);
    assert_eq!(s.revision(), before_failure);

    let conflict = run(
        &mut s,
        &mut runner(),
        "timeline.add_track",
        json!({"name":"Conflict","kind":"video"}),
        "fail",
    );
    error(conflict, ErrorCode::IdempotencyConflict);

    let retry = s
        .control_lifecycle_save(
            request_at(
                &s,
                "save-retry",
                "project.save",
                json!({}),
                "fresh",
                &before_failure,
            ),
            caller(&Allow),
            |_, _| Ok(()),
        )
        .unwrap();
    assert!(matches!(ok(retry), ControlResult::Edit { .. }));
    assert_ne!(s.revision(), before_failure);

    let invalid = s.control_lifecycle_save(
        request_at(&s, "", "project.save", json!({}), "fresh", &s.revision()),
        caller(&Allow),
        |_, _| panic!("invalid request must not publish"),
    );
    assert!(matches!(invalid, Err(EditError::Limit)));
}

#[test]
fn lent_font_operations_run_only_after_admission_and_never_on_replay() {
    use std::cell::Cell;
    let calls = Cell::new(0u32);
    let fail = Cell::new(false);
    let list = |_: Option<&str>| {
        calls.set(calls.get() + 1);
        Ok(Vec::new())
    };
    let load = |_: &str| -> engine::Result<engine::control::FontLoad> {
        calls.set(calls.get() + 1);
        Err(engine::FontError::Catalog.into())
    };
    let download = |_: &mut JobRunner, _: &str, _: bool| {
        calls.set(calls.get() + 1);
        if fail.get() {
            Err(engine::FontError::NotDownloadable.into())
        } else {
            Ok(engine::jobs::JobId(1))
        }
    };
    let access = engine::control::FontAccess::new(&list, &load, &download);
    let lent = |policy| Caller {
        fonts: Some(&access),
        ..caller(policy)
    };
    let mut s = session();
    let mut r = runner();
    let params = json!({"family":"Lobster","accept_license":true});

    // Denied by policy: no operation runs.
    let req = request(&s, "fonts.download", params.clone(), "denied");
    error(
        s.control_request(req, lent(&Deny), &mut r).unwrap(),
        ErrorCode::Permission,
    );
    assert_eq!(calls.get(), 0);

    // An operation failure is a typed refusal, and the first accepted call runs it once.
    fail.set(true);
    let req = request(&s, "fonts.download", params.clone(), "failing");
    error(
        s.control_request(req, lent(&Allow), &mut r).unwrap(),
        ErrorCode::Refused,
    );
    assert_eq!(calls.get(), 1);
    fail.set(false);
    let req = request(&s, "fonts.download", params.clone(), "once");
    ok(s.control_request(req, lent(&Allow), &mut r).unwrap());
    assert_eq!(calls.get(), 2);
    // Replaying the same idempotency key returns the receipt without a second invocation.
    let req = request(&s, "fonts.download", params, "once");
    ok(s.control_request(req, lent(&Allow), &mut r).unwrap());
    assert_eq!(calls.get(), 2);
}

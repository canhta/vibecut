#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
use engine::control::{self, Caller, ControlResult, ErrorCode, Policy, Request, ResponseValue};
use engine::{ByteBudget, Command, EditError, Session};
use project::{Item, ItemId, ItemParams, OverlayParams, Project, ProjectId, Track, TrackId};
use std::{path::PathBuf, sync::Arc};
use time::{FrameRate, SourceRange, TICKS_PER_SECOND, Tick};

struct Hooks;
struct Lease;
impl render::Reservation for Lease {}
impl render::ExecutionHooks for Hooks {
    fn cancelled(&self) -> bool {
        false
    }
    fn reserve(
        &self,
        _: render::MemoryKind,
        _: u64,
    ) -> render::Result<Box<dyn render::Reservation>> {
        Ok(Box::new(Lease))
    }
}

struct Allow;
impl Policy for Allow {
    fn authorize(&self, _: &Command, _: &Project) -> engine::Result<()> {
        Ok(())
    }
}

fn project(name: &str, width: u32, height: u32, duration: Tick) -> Project {
    let mut data = Project::empty(ProjectId(1), name.into())
        .unwrap()
        .into_data();
    data.canvas.width = width;
    data.canvas.height = height;
    data.canvas.fps = FrameRate::FPS_30;
    data.scene_mut().timeline.insert_track(Track {
        id: TrackId(1),
        name: "Overlay".into(),
        kind: project::TrackKind::Video,
        muted: false,
        hidden: false,
        locked: false,
        items: vec![Item {
            hidden: false,
            blend_mode: Default::default(),
            retime: Default::default(),
            masks: Vec::new(),
            effects: Vec::new(),
            animations: Default::default(),
            id: ItemId(1),
            source_asset: None,
            source_range: SourceRange::new(Tick::ZERO, duration).unwrap(),
            start: Tick::ZERO,
            content: ItemParams::Overlay(OverlayParams {
                color: project::Color([30, 160, 90, 255]),
            }),
        }],
    });
    Project::new(data).unwrap()
}

fn with_source(project: Project, path: PathBuf) -> Project {
    let mut data = project.into_data();
    data.assets.push(
        media::SourceAsset::try_from(media::AssetData {
            ephemeral: false,
            id: media::AssetId(7),
            path,
            content_hash: media::ContentHash::from_bytes(b"source"),
            probe: media::SourceProbe {
                format: "mp4".into(),
                duration: Some(Tick(TICKS_PER_SECOND)),
                streams: Vec::new(),
            },
        })
        .unwrap(),
    );
    Project::new(data).unwrap()
}

fn runner() -> engine::jobs::JobRunner {
    engine::jobs::JobRunner::new(
        engine::jobs::RunnerConfig {
            concurrency: [1, 1, 1],
            queue: 4,
            retained: 4,
            network_attempts: 1,
            backoff: std::time::Duration::from_millis(1),
            progress_interval: std::time::Duration::from_millis(10),
        },
        ByteBudget::new(512 << 20).unwrap(),
        Arc::new(engine::jobs::SystemClock::default()),
    )
    .unwrap()
}

fn request(session: &Session, id: &str, params: serde_json::Value, key: &str) -> Request {
    Request {
        api_version: control::API_VERSION,
        registry_hash: Some(control::registry_hash().into()),
        session_id: session.session_id().into(),
        request_id: "snapshot-request".into(),
        idempotency_key: Some(key.into()),
        base_rev: Some(session.revision()),
        command: id.into(),
        params,
    }
}

fn run(
    session: &mut Session,
    runner: &mut engine::jobs::JobRunner,
    id: &str,
    params: serde_json::Value,
    key: &str,
) -> ResponseValue {
    let request = request(session, id, params, key);
    session
        .control_request(
            request,
            Caller {
                principal: "test",
                external: false,
                policy: &Allow,
                fonts: None,
                media: None,
            },
            runner,
        )
        .unwrap()
        .value
}

fn reference(project: &Project, time: Tick) -> Vec<u8> {
    let settings = render::RenderSettings::from_project(project);
    let mut renderer = render::Renderer::new(project, settings).unwrap();
    renderer
        .render_frame(time, &mut render::FileSources::default(), &Hooks)
        .unwrap()
        .rgba8(13, &Hooks)
        .unwrap()
        .bytes
}

#[test]
fn snapshot_png_matches_canvas_render_at_the_playhead() {
    let frame = TICKS_PER_SECOND / 30;
    for (duration, requested, captured, label) in [
        (
            TICKS_PER_SECOND,
            TICKS_PER_SECOND / 2,
            TICKS_PER_SECOND / 2,
            "00:00:00:15",
        ),
        (1, 0, 0, "00:00:00:00"),
        (
            TICKS_PER_SECOND,
            TICKS_PER_SECOND - 1,
            TICKS_PER_SECOND - 1,
            "00:00:00:29",
        ),
        (
            TICKS_PER_SECOND,
            TICKS_PER_SECOND,
            TICKS_PER_SECOND - frame,
            "00:00:00:29",
        ),
        (frame + frame / 2, frame, frame, "00:00:00:01"),
        (frame + frame / 2, i64::MAX, frame, "00:00:00:01"),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("frame.png");
        let project = project("Snapshot", 64, 48, Tick(duration));
        let before = project::encode(&project, true).unwrap();

        let receipt = engine::snapshot::save(&project, Tick(requested), &out, &[]).unwrap();

        assert_eq!(receipt.path, out);
        assert_eq!(
            receipt.time,
            Tick(captured),
            "duration={duration}, time={requested}"
        );
        assert_eq!((receipt.width, receipt.height), (64, 48));
        assert_eq!(receipt.timecode, label);
        let decoded = image::open(&out).unwrap().to_rgba8();
        assert_eq!(decoded.dimensions(), (64, 48));
        assert_eq!(decoded.as_raw(), &reference(&project, Tick(captured)));
        assert_eq!(project::encode(&project, true).unwrap(), before);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}

#[test]
fn snapshot_default_name_sanitizes_reserved_characters_and_falls_back() {
    let one_second = Tick(TICKS_PER_SECOND);
    for (name, expected) in [
        ("CON", "_CON"),
        ("con.txt", "_con.txt"),
        ("NUL", "_NUL"),
        ("Com1", "_Com1"),
        ("LPT1.backup", "_LPT1.backup"),
        (" final.  ", "final"),
        ("...", "snapshot"),
        ("Tie\u{0302}\u{0301}ng Vie\u{0323}\u{0302}t", "Tiếng Việt"),
    ] {
        assert_eq!(
            engine::snapshot::default_name(&project(name, 64, 48, one_second), Tick::ZERO).unwrap(),
            format!("{expected}-00-00-00-00.png"),
            "project name: {name:?}",
        );
    }
    for (name, expected) in [
        ("con", "_con"),
        ("NUL.txt", "_NUL.txt"),
        ("com1.backup", "_com1.backup"),
        ("Lpt1", "_Lpt1"),
        (" report.  ", "report"),
        ("a:b?", "a_b_"),
        ("...", "Untitled Project"),
        ("  ", "Untitled Project"),
        ("Tie\u{0302}\u{0301}ng Vie\u{0323}\u{0302}t", "Tiếng Việt"),
    ] {
        assert_eq!(engine::sanitize_file_name(name), expected);
    }
    for name in ["CON", "PRN", "AUX", "NUL", "COM1", "COM9", "LPT1", "LPT9"] {
        for variant in [name.to_owned(), name.to_lowercase(), format!("{name}.txt")] {
            assert!(engine::sanitize_file_name(&variant).starts_with('_'));
        }
    }
    let long_name = "ế".repeat(100);
    let name = engine::snapshot::default_name(&project(&long_name, 64, 48, one_second), Tick::ZERO)
        .unwrap();
    assert!(name.len() <= 255);
    assert_eq!(engine::sanitize_file_name(&long_name).len(), 255);
    let export = engine::default_file_name(&long_name, "mp4");
    assert!(export.len() <= 255);
    assert!(export.ends_with(".mp4"));
    assert_eq!(engine::default_file_name("CON", "mp4"), "_CON.mp4");
    let oversized_grapheme = format!("x{}", "\u{0300}".repeat(300));
    assert_eq!(
        engine::sanitize_file_name(&oversized_grapheme),
        "Untitled Project"
    );

    assert_eq!(
        engine::snapshot::default_name(&project("My: Project?", 64, 48, one_second), Tick::ZERO)
            .unwrap(),
        "My- Project--00-00-00-00.png"
    );
    assert_eq!(
        engine::snapshot::default_name(&project("  ", 64, 48, one_second), Tick::ZERO).unwrap(),
        "snapshot-00-00-00-00.png"
    );
    assert_eq!(
        engine::snapshot::default_name(
            &project("D\u{1ef1} \u{00e1}n", 64, 48, one_second),
            Tick::ZERO
        )
        .unwrap(),
        "D\u{1ef1} \u{00e1}n-00-00-00-00.png"
    );
    let mut drop_frame = project("df", 64, 48, one_second).into_data();
    drop_frame.canvas.fps = FrameRate::FPS_29_97;
    let drop_frame = Project::new(drop_frame).unwrap();
    assert_eq!(
        engine::snapshot::default_name(&drop_frame, Tick::ZERO).unwrap(),
        "df-00-00-00;00.png"
    );
}

#[test]
fn snapshot_refuses_empty_project_with_project_is_empty() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("empty.png");
    let empty = Project::empty(ProjectId(2), "Empty".into()).unwrap();
    let error = engine::snapshot::save(&empty, Tick::ZERO, &out, &[]).unwrap_err();
    assert!(matches!(error, EditError::EmptyProject));
    assert_eq!(error.to_string(), "Project is empty");
    assert!(!out.exists());
    assert!(matches!(
        engine::snapshot::default_name(&empty, Tick::ZERO),
        Err(EditError::EmptyProject)
    ));

    let mut session = Session::new(empty, 10).unwrap();
    let mut runner = runner();
    let value = run(
        &mut session,
        &mut runner,
        "preview.snapshot",
        serde_json::json!({"time":0,"path":out}),
        "empty",
    );
    assert!(matches!(
        value,
        ResponseValue::Error(ErrorCode::EmptyProject)
    ));
}

#[test]
fn snapshot_publication_refuses_project_and_source_aliases() {
    let dir = tempfile::tempdir().unwrap();
    let project_path = dir.path().join("active.vibecut");
    std::fs::write(&project_path, b"project JSON").unwrap();
    let metadata = project::metadata_path(&project_path).unwrap();
    let recovery = dir.path().join("recovery");
    std::fs::create_dir(&recovery).unwrap();
    let source = dir.path().join("source.mp4");
    std::fs::write(&source, b"source bytes").unwrap();
    let project = with_source(
        project("Protected", 64, 48, Tick(TICKS_PER_SECOND)),
        source.clone(),
    );
    let project_alias = dir.path().join("project-alias.vibecut");
    std::fs::hard_link(&project_path, &project_alias).unwrap();
    // Recorded import hash is deliberately stale (b"source", not current bytes).
    let source_alias = dir.path().join("source-alias.mp4");
    std::fs::hard_link(&source, &source_alias).unwrap();
    #[cfg(any(unix, windows))]
    let source_symlink = {
        let path = dir.path().join("source-symlink.mp4");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&source, &path).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_file(&source, &path).unwrap();
        path
    };
    #[cfg(any(unix, windows))]
    let project_symlink = {
        let path = dir.path().join("project-symlink.vibecut");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&project_path, &path).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_file(&project_path, &path).unwrap();
        path
    };
    let protected = vec![project_path.clone(), metadata.clone(), recovery.clone()];
    let mut destinations = vec![
        project_path.clone(),
        dir.path().join(".").join("active.vibecut"),
        project_alias.clone(),
        metadata.clone(),
        recovery.join("journal.json"),
        source.clone(),
        source_alias.clone(),
    ];
    #[cfg(windows)]
    destinations.extend([
        dir.path().join("ACTIVE.VIBECUT.META.JSON"),
        recovery.join("JOURNAL.json"),
    ]);
    #[cfg(any(unix, windows))]
    destinations.extend([source_symlink, project_symlink]);

    for destination in destinations {
        let result = engine::snapshot::save(&project, Tick::ZERO, &destination, &protected);
        assert!(result.is_err(), "accepted alias {}", destination.display());
    }
    assert_eq!(std::fs::read(&project_path).unwrap(), b"project JSON");
    assert_eq!(std::fs::read(&source).unwrap(), b"source bytes");
    assert!(!metadata.exists());
    assert!(!recovery.join("journal.json").exists());

    let out = dir.path().join("snapshot.png");
    engine::snapshot::save(&project, Tick::ZERO, &out, &protected).unwrap();
    assert!(out.is_file());
}

#[test]
fn snapshot_refused_destination_leaves_no_partial_file() {
    let dir = tempfile::tempdir().unwrap();
    let project = project("Refused", 16, 16, Tick(TICKS_PER_SECOND));
    let directory = dir.path().join("destination");
    std::fs::create_dir(&directory).unwrap();
    assert!(engine::snapshot::save(&project, Tick::ZERO, &directory, &[]).is_err());
    assert!(directory.is_dir());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);

    let missing_parent = dir.path().join("missing").join("frame.png");
    assert!(engine::snapshot::save(&project, Tick::ZERO, &missing_parent, &[]).is_err());
    assert!(!dir.path().join("missing").exists());
}

#[test]
fn snapshot_control_request_publishes_and_is_non_mutating() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("frame.png");
    let project = project("Control", 32, 32, Tick(TICKS_PER_SECOND));
    let before = project::encode(&project, true).unwrap();
    let mut session = Session::new(project, 10).unwrap();
    let revision = session.revision();
    let mut runner = runner();
    let value = run(
        &mut session,
        &mut runner,
        "preview.snapshot",
        serde_json::json!({"time":0,"path":out}),
        "snapshot",
    );
    let ResponseValue::Result(ControlResult::Snapshot { snapshot }) = value else {
        panic!("expected snapshot result: {value:?}");
    };
    assert_eq!(snapshot.path, out);
    assert_eq!((snapshot.width, snapshot.height), (32, 32));
    assert_eq!(snapshot.timecode, "00:00:00:00");
    assert!(out.is_file());
    assert_eq!(session.revision(), revision);
    assert_eq!(project::encode(session.project(), true).unwrap(), before);
}

#[test]
fn snapshot_image_returns_the_same_png_without_publication() {
    let project = project("Copy", 16, 16, Tick(TICKS_PER_SECOND));
    let image = engine::snapshot::image(&project, Tick::ZERO).unwrap();
    assert_eq!((image.width, image.height), (16, 16));
    assert_eq!(&image.png[..8], b"\x89PNG\r\n\x1a\n");
    let decoded = image::load_from_memory(&image.png).unwrap().to_rgba8();
    assert_eq!(decoded.as_raw(), &reference(&project, Tick::ZERO));
}

#[test]
fn snapshot_command_is_registered_as_file_publication() {
    let spec = engine::registry()
        .iter()
        .find(|spec| spec.id == "preview.snapshot")
        .expect("snapshot command");
    assert_eq!(spec.effect, engine::Effect::FilePublish);
    assert!(matches!(
        serde_json::from_value::<Command>(
            serde_json::json!({"command":"preview.snapshot","params":{"time":0,"path":"out.png"}})
        )
        .unwrap(),
        Command::Snapshot { .. }
    ));
}

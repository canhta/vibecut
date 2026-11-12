#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "public-interface conformance assertions"
)]
use crate::support::control::{
    Allow, Deny, caller, error, finish, fixture_chapters, key, media_access, ok, request, run,
    runner, seed, session, submit, with_loudness, with_thumbnails, with_video, with_waveform,
    with_words, write_png, write_video, write_wav,
};
use engine::control::{
    Caller, ControlResult, ErrorCode, JobOutput, MediaAccess, Payload, ResponseValue,
};
use engine::jobs::{JobBytes, JobKind, JobRunner, JobSpec, JobState, ResourceClass};
use engine::{Command, Session};
use project::{Color, Item, ItemId, ItemParams, OverlayParams, Project, Track, TrackId, TrackKind};
use serde_json::{Value, json};
use std::{sync::Mutex, time::Duration};
use time::{FrameRate, SourceRange, Tick};

fn lut_cube() -> &'static str {
    "# original notice\nTITLE \"Imported look\"\nLUT_3D_SIZE 2\n0 0 0\n0 0 1\n1 0 0\n1 0 1\n0 1 0\n0 1 1\n1 1 0\n1 1 1\n"
}
fn lut_result(
    s: &mut Session,
    r: &mut JobRunner,
    media: &Mutex<MediaAccess>,
    path: &std::path::Path,
    expected: Option<project::LutId>,
    key: &str,
) -> project::LutRecord {
    let id = submit_media(
        s,
        r,
        media,
        "color.import_lut",
        json!({"path":path,"expected_hash":expected}),
        key,
    );
    finish(r, id);
    assert_eq!(r.snapshot(id).unwrap().attempts, 1);
    let view = media_status(s, r, media, id, &format!("{key}-status"));
    assert_eq!(view.state, JobState::Completed, "{view:?}");
    match view.output.unwrap() {
        JobOutput::Lut(record) => record,
        other => panic!("{other:?}"),
    }
}
#[test]
fn lut_import_applies_one_undo_and_reopens_shared_pixels() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("user look.cube");
    std::fs::write(&source, lut_cube()).unwrap();
    let media = Mutex::new(media_access(dir.path()));
    let mut r = runner();
    let mut s = seed();
    let before = s.project().clone();
    let history = s.history_len();
    let record = lut_result(&mut s, &mut r, &media, &source, None, "lut-import");
    assert_eq!(s.project(), &before);
    assert_eq!(s.history_len(), history);
    assert_eq!(
        std::fs::read(record.directory.join("original.cube")).unwrap(),
        lut_cube().as_bytes()
    );
    assert!(
        !std::fs::read(record.directory.join("canonical.cube"))
            .unwrap()
            .windows(6)
            .any(|w| w == b"notice")
    );
    s.apply_lut(ItemId(1), record.clone(), 50.).unwrap();
    assert_eq!(s.history_len().0, history.0 + 1);
    let applied = s.project().clone();
    s.apply(Command::Undo {}).unwrap();
    assert_eq!(s.project(), &before);
    s.apply(Command::Redo {}).unwrap();
    assert_eq!(s.project(), &applied);
    let revision = s.revision();
    assert!(!s.apply_lut(ItemId(1), record.clone(), 50.).unwrap().changed);
    assert_eq!(s.revision(), revision);
    let path = dir.path().join("project.vibecut");
    project::WriterLease::acquire(&path)
        .unwrap()
        .save(s.project())
        .unwrap();
    let loaded = project::load(&path).unwrap().project;
    assert_eq!(&loaded, s.project().as_ref());
    std::fs::rename(&source, dir.path().join("moved.cube")).unwrap();
    let budget = engine::ByteBudget::new(64 << 20).unwrap();
    let hooks = engine::ByteBudgetHooks::new(&budget, None);
    let item = loaded
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|t| &t.items)
        .find(|i| i.id == ItemId(1))
        .unwrap();
    let effect = item
        .effects
        .iter()
        .find(|e| e.kind == "color_grade")
        .unwrap();
    let resources =
        render::lut::LutResources::resolve(&loaded, std::iter::once(effect), &hooks).unwrap();
    let passes = render::effects::evaluate_effect(
        &item.animations,
        effect,
        Tick::ZERO,
        render::effects::EffectGeometry::new([2, 2], [1., 1.]).unwrap(),
        true,
        &resources,
    )
    .unwrap();
    let input = render::Image::new(2, 2, [0.1, 0.2, 0.3, 0.5], &hooks).unwrap();
    let below = render::Image::new(2, 2, [0.; 4], &hooks).unwrap();
    let output =
        render::effects::apply_effect(input, &passes, &below, &render::effects::CpuEffects, &hooks)
            .unwrap();
    assert_eq!(output.pixels()[0][3], 0.5);
    assert_ne!(&output.pixels()[0][..3], &[0.1, 0.2, 0.3]);
    drop((resources, passes, below, output));
    assert_eq!(budget.snapshot().unwrap().used, 0);
    // Strict preview and export enter the same real scene evaluator, with a
    // device-free overlay so the assertion cannot accidentally use a codec oracle.
    let mut data = loaded.into_data();
    data.assets.clear();
    for track in data.scene_mut().timeline.iter_mut() {
        track.items.retain(|i| i.id == ItemId(1));
    }
    data.scene_mut()
        .timeline
        .overlays
        .retain(|t| t.items.iter().any(|i| i.id == ItemId(1)));
    data.scene_mut().timeline.audio.clear();
    data.scene_mut().timeline.main.items.clear();
    let item = data
        .scene_mut()
        .timeline
        .overlays
        .iter_mut()
        .flat_map(|t| &mut t.items)
        .find(|i| i.id == ItemId(1))
        .unwrap();
    item.masks.clear();
    item.effects.retain(|e| e.kind == "color_grade");
    let project = Project::new(data).unwrap();
    let mut settings = render::RenderSettings::from_project(&project);
    settings.width = 2;
    settings.height = 2;
    let mut renderer = render::Renderer::new(&project, settings).unwrap();
    let mut sources = render::FileSources::default();
    let strict = renderer
        .render_frame(Tick::ZERO, &mut sources, &hooks)
        .unwrap();
    let preview = renderer
        .preview_frame(Tick::ZERO, &mut sources, &hooks)
        .unwrap();
    assert_eq!(strict.pixels(), preview.pixels());
    drop((strict, preview, renderer));
    assert_eq!(budget.snapshot().unwrap().used, 0);
    std::fs::remove_file(record.directory.join("canonical.cube")).unwrap();
    let mut renderer =
        render::Renderer::new(&project, render::RenderSettings::from_project(&project)).unwrap();
    assert!(matches!(
        renderer.render_frame(Tick::ZERO, &mut sources, &hooks),
        Err(render::Error::LutUnavailable {
            reason: render::lut::LutUnavailableReason::Missing,
            ..
        })
    ));
    assert_eq!(budget.snapshot().unwrap().used, 0);
}
#[test]
fn lut_deduplicates_preserves_first_provenance_and_repairs_matching_content() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("look.cube");
    std::fs::write(&source, lut_cube()).unwrap();
    let media = Mutex::new(media_access(dir.path()));
    let mut r = runner();
    let mut s = seed();
    let first = lut_result(&mut s, &mut r, &media, &source, None, "lut-first");
    s.apply_lut(ItemId(1), first.clone(), 100.).unwrap();
    std::fs::write(
        &source,
        lut_cube().replace("# original notice", "# different formatting"),
    )
    .unwrap();
    let second = lut_result(&mut s, &mut r, &media, &source, None, "lut-dedup");
    assert_eq!(first, second);
    for (index, metadata) in [
        json!({"name":"conflicting"}),
        json!({"licence":"different"}),
    ]
    .into_iter()
    .enumerate()
    {
        let mut params = metadata;
        params["path"] = json!(source);
        params["expected_hash"] = json!(first.id);
        let job = submit_media(
            &mut s,
            &mut r,
            &media,
            "color.import_lut",
            params,
            &format!("lut-conflict-{index}"),
        );
        finish(&mut r, job);
        assert_eq!(r.snapshot(job).unwrap().state, JobState::Failed);
        assert!(r.output_bytes(job).is_none());
        assert_eq!(
            std::fs::read(first.directory.join("original.cube")).unwrap(),
            lut_cube().as_bytes()
        );
    }
    for cache in media.lock().unwrap().cache_maintenance().unwrap() {
        let report = cache.run(engine::cache::CacheOperation::Clear, || false);
        assert!(report.cause.is_none());
    }
    assert!(first.directory.join("canonical.cube").is_file());
    std::fs::write(first.directory.join("canonical.cube"), b"broken").unwrap();
    let before = s.project().clone();
    let repaired = lut_result(
        &mut s,
        &mut r,
        &media,
        &source,
        Some(first.id),
        "lut-relink",
    );
    assert_eq!(repaired, first);
    assert_eq!(s.project(), &before);
    let budget = engine::ByteBudget::new(64 << 20).unwrap();
    let hooks = engine::ByteBudgetHooks::new(&budget, None);
    let effect = s
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|t| &t.items)
        .find(|i| i.id == ItemId(1))
        .unwrap()
        .effects
        .iter()
        .find(|e| e.kind == "color_grade")
        .unwrap();
    assert!(
        render::lut::LutResources::resolve(s.project(), std::iter::once(effect), &hooks).is_ok()
    );
    for (index, missing) in ["canonical.cube", "original.cube"].into_iter().enumerate() {
        std::fs::remove_file(first.directory.join(missing)).unwrap();
        let repaired = lut_result(
            &mut s,
            &mut r,
            &media,
            &source,
            Some(first.id),
            &format!("lut-missing-{index}"),
        );
        assert_eq!(repaired, first);
        assert!(first.directory.join(missing).is_file());
        assert_eq!(s.project(), &before);
    }
    std::fs::write(&source, lut_cube().replace("0 0 1", "0 0 0")).unwrap();
    let protected = std::fs::read(first.directory.join("canonical.cube")).unwrap();
    let id = submit_media(
        &mut s,
        &mut r,
        &media,
        "color.import_lut",
        json!({"path":source,"expected_hash":first.id}),
        "lut-mismatch",
    );
    finish(&mut r, id);
    assert_eq!(r.snapshot(id).unwrap().state, JobState::Failed);
    assert!(r.output_bytes(id).is_none());
    assert_eq!(s.project(), &before);
    assert_eq!(
        std::fs::read(first.directory.join("canonical.cube")).unwrap(),
        protected
    );
}
#[test]
fn lut_invalid_cancelled_and_locked_workflows_preserve_state() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("hostile.cube");
    std::fs::write(&source, b"LUT_3D_SIZE 999999999\n").unwrap();
    let media = Mutex::new(media_access(dir.path()));
    let mut r = runner();
    let mut s = seed();
    let before = s.project().clone();
    let id = submit_media(
        &mut s,
        &mut r,
        &media,
        "color.import_lut",
        json!({"path":source}),
        "lut-bad",
    );
    finish(&mut r, id);
    assert_eq!(r.snapshot(id).unwrap().state, JobState::Failed);
    let failed = r.snapshot(id).unwrap();
    let raw = failed.error.as_ref().unwrap();
    let Some(engine::ArtifactError::Lut(engine::luts::Error::Render(original))) = (raw.as_ref()
        as &(dyn std::error::Error + 'static))
        .downcast_ref::<engine::ArtifactError>()
    else {
        panic!("lost composed LUT parser producer")
    };
    assert_eq!(raw.code(), ErrorCode::Render);
    assert!(!raw.transient());
    assert!(!raw.job_policy().terminal_cancelled);
    assert_eq!(raw.to_string(), "render execution failed");
    assert!(std::ptr::eq(
        original,
        raw.source()
            .unwrap()
            .downcast_ref::<render::Error>()
            .unwrap()
    ));
    assert!(std::sync::Arc::ptr_eq(
        raw,
        r.snapshots()
            .iter()
            .find(|s| s.id == id)
            .unwrap()
            .error
            .as_ref()
            .unwrap()
    ));
    assert!(std::sync::Arc::ptr_eq(
        raw,
        r.drain_events()
            .iter()
            .find(|s| s.id == id)
            .unwrap()
            .error
            .as_ref()
            .unwrap()
    ));
    assert!(r.output_bytes(id).is_none());
    assert_eq!(s.project(), &before);
    std::fs::write(&source, lut_cube()).unwrap();
    let id = submit_media(
        &mut s,
        &mut r,
        &media,
        "color.import_lut",
        json!({"path":source}),
        "lut-cancel",
    );
    r.cancel_job(id).unwrap();
    finish(&mut r, id);
    assert_eq!(r.snapshot(id).unwrap().state, JobState::Cancelled);
    assert_eq!(s.project(), &before);
    let record = lut_result(&mut s, &mut r, &media, &source, None, "lut-after-cancel");
    s.apply_lut(ItemId(1), record.clone(), 100.).unwrap();
    let unchanged = s.project().clone();
    let parent = record.directory.parent().unwrap();
    let publication =
        project::WriterLease::acquire(&parent.join(format!("{}.bundle", record.id.hex()))).unwrap();
    std::fs::write(
        record.directory.join("canonical.cube"),
        b"protected damaged resource",
    )
    .unwrap();
    let id = submit_media(
        &mut s,
        &mut r,
        &media,
        "color.import_lut",
        json!({"path":source,"expected_hash":record.id}),
        "lut-cancel-at-publication",
    );
    let deadline = std::time::Instant::now() + Duration::from_secs(4);
    loop {
        r.tick().unwrap();
        r.drain_events();
        let staged = std::fs::read_dir(parent).unwrap().any(|entry| {
            let path = entry.unwrap().path();
            path != record.directory && path.is_dir() && path.join("provenance.json").is_file()
        });
        if staged {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "import did not reach publication gate"
        );
        std::thread::yield_now();
    }
    r.cancel_job(id).unwrap();
    finish(&mut r, id);
    assert_eq!(r.snapshot(id).unwrap().state, JobState::Cancelled);
    assert!(r.output_bytes(id).is_none());
    assert_eq!(s.project(), &unchanged);
    assert_eq!(
        std::fs::read(record.directory.join("canonical.cube")).unwrap(),
        b"protected damaged resource"
    );
    assert_eq!(
        std::fs::read_dir(parent)
            .unwrap()
            .filter(|e| e.as_ref().unwrap().file_type().unwrap().is_dir())
            .count(),
        1
    );
    drop(publication);
    assert_eq!(
        lut_result(
            &mut s,
            &mut r,
            &media,
            &source,
            Some(record.id),
            "lut-after-publication-cancel"
        ),
        record
    );
    let mut data = s.project().data().clone();
    data.scene_mut()
        .timeline
        .iter_mut()
        .find(|t| t.items.iter().any(|i| i.id == ItemId(1)))
        .unwrap()
        .locked = true;
    s = Session::new(Project::new(data).unwrap(), 10).unwrap();
    let before = s.project().clone();
    assert!(s.apply_lut(ItemId(1), record, 100.).is_err());
    assert_eq!(s.project(), &before);
    assert_eq!(s.history_len(), (0, 0));
}

#[test]
fn lut_foreign_bundle_refuses_and_releases_the_store_for_later_use() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("look.cube");
    std::fs::write(&source, lut_cube()).unwrap();
    let media = Mutex::new(media_access(dir.path()));
    let mut r = runner();
    let mut s = seed();
    let record = lut_result(&mut s, &mut r, &media, &source, None, "lut-owned");
    let sentinel = record.directory.join("foreign.txt");
    std::fs::hard_link(&source, &sentinel).unwrap();
    let project = s.project().clone();
    let original = std::fs::read(&source).unwrap();
    let id = submit_media(
        &mut s,
        &mut r,
        &media,
        "color.import_lut",
        json!({"path":source}),
        "lut-foreign",
    );
    finish(&mut r, id);
    assert_eq!(r.snapshot(id).unwrap().state, JobState::Failed);
    assert!(r.output_bytes(id).is_none());
    assert_eq!(std::fs::read(&source).unwrap(), original);
    assert_eq!(std::fs::read(&sentinel).unwrap(), original);
    assert_eq!(s.project(), &project);
    std::fs::remove_file(sentinel).unwrap();
    assert_eq!(
        lut_result(&mut s, &mut r, &media, &source, None, "lut-after-refusal"),
        record
    );
    assert!(
        !std::fs::read_dir(record.directory.parent().unwrap())
            .unwrap()
            .any(|e| e
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".lut-"))
    );
}

#[test]
fn lut_concurrent_same_content_has_one_complete_bundle_and_no_partial_output() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("look.cube");
    std::fs::write(&source, lut_cube()).unwrap();
    let media = Mutex::new(media_access(dir.path()));
    let mut s = seed();
    let mut r = JobRunner::new(
        engine::jobs::RunnerConfig {
            concurrency: [2, 1, 1],
            queue: 4,
            retained: 4,
            network_attempts: 1,
            backoff: Duration::from_millis(1),
            progress_interval: Duration::from_millis(10),
        },
        engine::ByteBudget::new(128 << 20).unwrap(),
        std::sync::Arc::new(engine::jobs::SystemClock::default()),
    )
    .unwrap();
    let a = submit_media(
        &mut s,
        &mut r,
        &media,
        "color.import_lut",
        json!({"path":source}),
        "lut-race-a",
    );
    let b = submit_media(
        &mut s,
        &mut r,
        &media,
        "color.import_lut",
        json!({"path":source}),
        "lut-race-b",
    );
    finish(&mut r, a);
    finish(&mut r, b);
    let first = media_status(&mut s, &mut r, &media, a, "lut-race-status-a");
    let second = media_status(&mut s, &mut r, &media, b, "lut-race-status-b");
    assert_eq!(first.state, JobState::Completed);
    assert_eq!(second.state, JobState::Completed);
    let (Some(JobOutput::Lut(first)), Some(JobOutput::Lut(second))) = (first.output, second.output)
    else {
        panic!("missing LUT race output");
    };
    assert_eq!(first, second);
    assert_eq!(std::fs::read_dir(&first.directory).unwrap().count(), 3);
    assert_eq!(
        std::fs::read_dir(first.directory.parent().unwrap())
            .unwrap()
            .filter(|e| e.as_ref().unwrap().file_type().unwrap().is_dir())
            .count(),
        1
    );
    assert!(s.project().data().luts.is_empty());
}

#[test]
fn lut_store_reopen_enforces_disk_quota_without_deleting_foreign_files() {
    let dir = tempfile::tempdir().unwrap();
    let sentinel = dir.path().join("foreign.bin");
    let file = std::fs::File::create(&sentinel).unwrap();
    file.set_len((256 << 20) + 1).unwrap();
    drop(file);
    assert!(matches!(
        engine::luts::LutStore::open(dir.path()),
        Err(engine::luts::Error::Edit(engine::EditError::Control(
            engine::control::ControlError::Limit
        )))
    ));
    let blocked = engine::luts::LutStore::open(&sentinel).err().unwrap();
    use engine::Failure;
    assert_eq!(blocked.code(), ErrorCode::Io);
    assert!(!blocked.transient());
    assert_eq!(blocked.to_string(), "job/cache IO failed");
    let engine::luts::Error::Io(original) = &blocked else {
        panic!("lost store IO producer")
    };
    assert_eq!(original.kind(), std::io::ErrorKind::AlreadyExists);
    assert!(std::ptr::eq(
        original,
        std::error::Error::source(&blocked)
            .unwrap()
            .downcast_ref::<std::io::Error>()
            .unwrap()
    ));
    // A store-only job keeps the actual lower producer, rather than the engine aggregate.
    let path = dir.path().to_owned();
    let mut jobs = runner();
    let (id, _) = jobs
        .submit(
            JobSpec {
                kind: JobKind::ImportLut,
                class: ResourceClass::Cpu,
                input: key(),
                bytes: JobBytes::default(),
            },
            move |_| engine::luts::LutStore::open(&path).map(|_| Vec::new()),
        )
        .unwrap();
    finish(&mut jobs, id);
    let snapshot = jobs.snapshot(id).unwrap();
    assert_eq!((snapshot.state, snapshot.attempts), (JobState::Failed, 1));
    let raw = snapshot.error.as_ref().unwrap();
    assert!(matches!(
        (raw.as_ref() as &(dyn std::error::Error + 'static)).downcast_ref::<engine::luts::Error>(),
        Some(engine::luts::Error::Edit(engine::EditError::Control(
            engine::control::ControlError::Limit
        )))
    ));
    assert_eq!(raw.code(), ErrorCode::Limit);
    assert!(!raw.transient());
    assert!(!raw.job_policy().terminal_cancelled);
    assert_eq!(raw.job_policy().retry_after_seconds, None);
    assert_eq!(raw.to_string(), "command limit exceeded");
    assert!(raw.source().is_none());
    assert!(std::sync::Arc::ptr_eq(
        raw,
        jobs.snapshots()
            .iter()
            .find(|s| s.id == id)
            .unwrap()
            .error
            .as_ref()
            .unwrap()
    ));
    assert!(std::sync::Arc::ptr_eq(
        raw,
        jobs.drain_events()
            .iter()
            .find(|s| s.id == id)
            .unwrap()
            .error
            .as_ref()
            .unwrap()
    ));
    let projection = seed().snapshot_and_subscribe(&jobs).unwrap();
    assert_eq!(
        projection
            .jobs
            .iter()
            .find(|job| job.id == id)
            .unwrap()
            .error,
        Some(ErrorCode::Limit)
    );
    assert!(jobs.take_output(id).is_none());
    assert_eq!(std::fs::metadata(&sentinel).unwrap().len(), (256 << 20) + 1);
    std::fs::remove_file(&sentinel).unwrap();
    let store = engine::luts::LutStore::open(dir.path()).unwrap();
    drop(store);
    let source = dir.path().join("look.cube");
    std::fs::write(&source, lut_cube()).unwrap();
    let media = Mutex::new(media_access(dir.path()));
    let root = dir.path().join("luts");
    for index in 0..4090 {
        std::fs::write(root.join(format!("foreign-{index}")), b"").unwrap();
    }
    let mut s = seed();
    let before = s.project().clone();
    let mut r = runner();
    let job = submit_media(
        &mut s,
        &mut r,
        &media,
        "color.import_lut",
        json!({"path":source}),
        "lut-entry-limit",
    );
    finish(&mut r, job);
    assert_eq!(r.snapshot(job).unwrap().state, JobState::Failed);
    assert!(r.output_bytes(job).is_none());
    assert_eq!(s.project(), &before);
    assert_eq!(std::fs::read_dir(root).unwrap().count(), 4091);
}

#[test]
fn lut_managed_duplicate_history_and_trash_keep_verified_copied_resources() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("look.cube");
    std::fs::write(&source, lut_cube()).unwrap();
    let media = Mutex::new(media_access(dir.path()));
    let mut r = runner();
    let mut s = seed();
    let record = lut_result(&mut s, &mut r, &media, &source, None, "lut-managed");
    let mut library = engine::library::ProjectLibrary::open(
        &dir.path().join("projects"),
        engine::ByteBudget::new(128 << 20).unwrap(),
    )
    .unwrap();
    library
        .execute(
            project::ProjectId(1),
            Command::CreateProject {
                id: project::ProjectId(1),
                name: "Managed".into(),
            },
        )
        .unwrap();
    let original = library.list().unwrap()[0].path.clone();
    let mut lease = project::WriterLease::acquire(&original).unwrap();
    let mut data = lease.load().unwrap().project.into_data();
    data.luts.push(record.clone());
    lease.save(&Project::new(data).unwrap()).unwrap();
    drop(lease);
    library
        .execute(
            project::ProjectId(1),
            Command::DuplicateProject {
                id: project::ProjectId(2),
            },
        )
        .unwrap();
    let copied_path = library
        .list()
        .unwrap()
        .into_iter()
        .find(|p| p.id == project::ProjectId(2))
        .unwrap()
        .path;
    let copied = project::load(&copied_path).unwrap().project.data().luts[0].clone();
    assert_ne!(copied.directory, record.directory);
    assert_eq!(copied.id, record.id);
    let canonical = std::fs::read(copied.directory.join("canonical.cube")).unwrap();
    assert_eq!(
        std::fs::read(copied.directory.join("original.cube")).unwrap(),
        lut_cube().as_bytes()
    );
    library.undo().unwrap();
    assert!(!copied_path.exists());
    library.redo().unwrap();
    assert_eq!(
        std::fs::read(copied.directory.join("canonical.cube")).unwrap(),
        canonical
    );
    std::fs::write(copied.directory.join("canonical.cube"), b"changed").unwrap();
    let history = library.history_len();
    assert!(library.undo().is_err());
    assert_eq!(library.history_len(), history);
    assert!(copied_path.exists());
    std::fs::write(copied.directory.join("canonical.cube"), &canonical).unwrap();
    library
        .execute(project::ProjectId(2), Command::DeleteProject {})
        .unwrap();
    assert!(!copied_path.exists());
    library
        .execute(project::ProjectId(2), Command::RestoreProject {})
        .unwrap();
    assert_eq!(
        std::fs::read(copied.directory.join("canonical.cube")).unwrap(),
        canonical
    );
    assert_eq!(
        std::fs::read(record.directory.join("canonical.cube")).unwrap(),
        canonical
    );
}

fn submit_media(
    s: &mut Session,
    r: &mut JobRunner,
    media: &Mutex<MediaAccess>,
    id: &str,
    params: Value,
    key: &str,
) -> engine::jobs::JobId {
    let req = request(s, id, params, key);
    let caller = Caller {
        principal: "test",
        external: true,
        policy: &Allow,
        fonts: None,
        sounds: None,
        music: None,
        media: Some(media),
        chapters: None,
        captions: None,
        vision: None,
    };
    match ok(s.control_request(req, caller, r).unwrap()) {
        ControlResult::JobAccepted { job, .. } => job,
        other => panic!("{other:?}"),
    }
}
fn media_status(
    s: &mut Session,
    r: &mut JobRunner,
    media: &Mutex<MediaAccess>,
    job: engine::jobs::JobId,
    key: &str,
) -> engine::control::JobView {
    let req = request(s, "job_status", json!({"job":job.0}), key);
    let caller = Caller {
        principal: "test",
        external: true,
        policy: &Allow,
        fonts: None,
        sounds: None,
        music: None,
        media: Some(media),
        chapters: None,
        captions: None,
        vision: None,
    };
    match ok(s.control_request(req, caller, r).unwrap()) {
        ControlResult::Job { snapshot } => snapshot,
        other => panic!("{other:?}"),
    }
}
fn media_state(
    s: &mut Session,
    r: &mut JobRunner,
    media: &Mutex<MediaAccess>,
    job: engine::jobs::JobId,
    key: &str,
) -> JobState {
    media_status(s, r, media, job, key).state
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
        ControlResult::Job { snapshot } => match snapshot.output {
            Some(JobOutput::Preview(result)) => result,
            other => panic!("preview output: {other:?}"),
        },
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
    r.cancel_job(job).unwrap();
    drop(r.take_output(job).unwrap());
    assert_eq!(r.snapshot(job).unwrap().state, JobState::Completed);
    error(
        run(
            &mut s,
            &mut r,
            "job_status",
            json!({"job":job.0}),
            "missing-output",
        ),
        ErrorCode::JobNotRetained,
    );
    r.cancel_job(job).unwrap();
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
fn chapters_generate_uses_the_lent_consent_and_exports_sidecars() {
    let root = tempfile::tempdir().unwrap();
    let mut s = with_words(&seed());
    let mut r = runner();
    let chapters = fixture_chapters();
    let allow = Allow;
    let before = serde_json::to_vec(s.project().data()).unwrap();

    // An agent transport without a lent chapter access cannot upload.
    let req = request(
        &s,
        "chapters.generate",
        json!({"language":"en","max_chapters":2}),
        "chapters-denied",
    );
    error(
        s.control_request(
            req,
            Caller {
                principal: "local-mcp",
                external: true,
                policy: &allow,
                fonts: None,
                sounds: None,
                music: None,
                media: None,
                chapters: None,
                captions: None,
                vision: None,
            },
            &mut r,
        )
        .unwrap(),
        ErrorCode::Permission,
    );

    let req = request(
        &s,
        "chapters.generate",
        json!({"language":"en","max_chapters":2}),
        "chapters",
    );
    let value = ok(s
        .control_request(
            req,
            Caller {
                principal: "ui",
                external: false,
                policy: &allow,
                fonts: None,
                sounds: None,
                music: None,
                media: None,
                chapters: Some(&chapters),
                captions: None,
                vision: None,
            },
            &mut r,
        )
        .unwrap());
    let ControlResult::JobAccepted { job, .. } = value else {
        panic!("{value:?}");
    };
    finish(&mut r, job);
    let req = request(&s, "job_status", json!({"job":job.0}), "chapters-status");
    let value = ok(s
        .control_request(
            req,
            Caller {
                principal: "ui",
                external: false,
                policy: &allow,
                fonts: None,
                sounds: None,
                music: None,
                media: None,
                chapters: Some(&chapters),
                captions: None,
                vision: None,
            },
            &mut r,
        )
        .unwrap());
    let ControlResult::Job { snapshot } = value else {
        panic!("{value:?}");
    };
    let Some(JobOutput::Chapters(document)) = snapshot.output.clone() else {
        panic!("chapters output: {snapshot:?}");
    };
    assert_eq!(document.language().as_str(), "en");
    assert_eq!(document.chapters().len(), 2);
    assert_eq!(document.chapters()[0].start, Tick::ZERO);
    assert_eq!(serde_json::to_vec(s.project().data()).unwrap(), before);

    let edited = ai::chapters::ChapterDocument::new(
        document.language().clone(),
        document.duration(),
        "Reviewed title".into(),
        document.description().clone(),
        document.chapters().to_vec(),
        Vec::new(),
    )
    .unwrap();
    let params = json!({
        "dir": root.path(),
        "document": serde_json::to_value(&edited).unwrap(),
    });
    let req = request(
        &s,
        "chapters.export_sidecars",
        params.clone(),
        "chapters-export",
    );
    let value = ok(s
        .control_request(
            req,
            Caller {
                principal: "ui",
                external: false,
                policy: &allow,
                fonts: None,
                sounds: None,
                music: None,
                media: None,
                chapters: None,
                captions: None,
                vision: None,
            },
            &mut r,
        )
        .unwrap());
    let ControlResult::ChapterSidecars { paths } = value else {
        panic!("{value:?}");
    };
    assert!(paths.text.exists());
    assert!(paths.json.exists());
    let json = std::fs::read(&paths.json).unwrap();
    assert_eq!(
        ai::chapters::ChapterDocument::from_json(&json)
            .unwrap()
            .title()
            .as_str(),
        "Reviewed title"
    );
    assert!(
        std::fs::read_to_string(&paths.text)
            .unwrap()
            .starts_with("Reviewed title")
    );
    assert_eq!(serde_json::to_vec(s.project().data()).unwrap(), before);

    let req = request(
        &s,
        "chapters.export_sidecars",
        params,
        "chapters-export-again",
    );
    error(
        s.control_request(
            req,
            Caller {
                principal: "ui",
                external: false,
                policy: &allow,
                fonts: None,
                sounds: None,
                music: None,
                media: None,
                chapters: None,
                captions: None,
                vision: None,
            },
            &mut r,
        )
        .unwrap(),
        ErrorCode::Refused,
    );
    r.shutdown();
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
        .submit::<engine::control::Error>(
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
        ControlResult::Job { snapshot } => match snapshot.output {
            Some(JobOutput::Preview(result)) => result,
            other => panic!("preview output: {other:?}"),
        },
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
        ControlResult::Job { snapshot } => match snapshot.output {
            Some(JobOutput::Preview(result)) => result,
            other => panic!("preview output: {other:?}"),
        },
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
fn lent_font_operations_run_only_after_admission_and_never_on_replay() {
    use std::cell::Cell;
    let calls = Cell::new(0u32);
    let fail = Cell::new(false);
    let list = |_: Option<&str>| {
        calls.set(calls.get() + 1);
        Ok(Vec::new())
    };
    let load = |_: &str| -> engine::control::Result<engine::control::FontLoad> {
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
    let import = |_: &mut JobRunner, _: &std::path::Path, _: bool| {
        calls.set(calls.get() + 1);
        if fail.get() {
            Err(engine::FontError::InvalidImport.into())
        } else {
            Ok(engine::jobs::JobId(2))
        }
    };
    let access = engine::control::FontAccess::new(&list, &load, &download, &import);
    let lent = |policy| Caller {
        fonts: Some(&access),
        sounds: None,
        music: None,
        ..caller(policy)
    };
    for (command, params) in [
        (
            "fonts.download",
            json!({"family":"Lobster","accept_license":true}),
        ),
        (
            "fonts.import",
            json!({"path":"ungranted-private.ttf","accept_license":true}),
        ),
    ] {
        calls.set(0);
        let mut s = session();
        let mut r = runner();

        // Denied by policy: no operation runs.
        let req = request(&s, command, params.clone(), "denied");
        error(
            s.control_request(req, lent(&Deny), &mut r).unwrap(),
            ErrorCode::Permission,
        );
        assert_eq!(calls.get(), 0);

        // An operation failure is a typed refusal, and the first accepted call runs it once.
        fail.set(true);
        let req = request(&s, command, params.clone(), "failing");
        error(
            s.control_request(req, lent(&Allow), &mut r).unwrap(),
            ErrorCode::Refused,
        );
        assert_eq!(calls.get(), 1);
        fail.set(false);
        let req = request(&s, command, params.clone(), "once");
        ok(s.control_request(req, lent(&Allow), &mut r).unwrap());
        assert_eq!(calls.get(), 2);
        // Replaying the same idempotency key returns the receipt without a second invocation.
        let req = request(&s, command, params, "once");
        ok(s.control_request(req, lent(&Allow), &mut r).unwrap());
        assert_eq!(calls.get(), 2);
    }
}
/// Pause candidates are one analysis job whose spans surface through job_status.
#[test]
fn pause_candidates_job_reports_reviewed_spans_through_job_status() {
    if !testkit::oracle::require("pause candidates") {
        return;
    }
    let dir = testkit::fixture::dir("control-pause-candidates");
    let expression = "(0.25*sin(2*PI*180*t)+0.15*sin(2*PI*320*t)+0.1*sin(2*PI*520*t))*\
        (between(t\\,0\\,2)+between(t\\,2.8\\,4.6)+between(t\\,5.8\\,7.8)+between(t\\,9.4\\,12))";
    let input = format!("aevalsrc={expression}:s=16000:d=12");
    let path = testkit::oracle::generate(
        &dir,
        "pause candidates",
        "control-pauses.wav",
        &["-f", "lavfi", "-i", input.as_str(), "-c:a", "pcm_s16le"],
    )
    .unwrap();
    let asset = media::SourceAsset::import(media::AssetId(1), path).unwrap();
    let duration = time::Tick(12 * time::TICKS_PER_SECOND);
    let mut data = project::Project::empty(project::ProjectId(0), "pauses".into())
        .unwrap()
        .into_data();
    data.assets.push(asset.clone());
    data.scene_mut().timeline.insert_track(Track {
        id: TrackId(1),
        name: "Audio".into(),
        kind: TrackKind::Audio,
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
            source_asset: Some(asset.id()),
            source_range: SourceRange::new(time::Tick::ZERO, duration).unwrap(),
            start: time::Tick::ZERO,
            content: ItemParams::Audio(project::AudioParams {
                fades: Default::default(),
                volume: 0.0,
                muted: false,
                source: Default::default(),
            }),
        }],
    });
    let mut s = Session::new(project::Project::new(data).unwrap(), 10).unwrap();
    let mut r = runner();
    let req = request(
        &s,
        "timeline.pause_candidates",
        json!({"min_pause_ms":500,"pad_ms":20,"crossfade_ms":20}),
        "pause-candidates",
    );
    let response = ok(s.control_request(req, caller(&Allow), &mut r).unwrap());
    let ControlResult::JobAccepted { job, .. } = response else {
        panic!("pause candidates must return a job");
    };
    finish(&mut r, job);
    let req = request(&s, "job_status", json!({"job":job.0}), "pause-status");
    let ControlResult::Job { snapshot } =
        ok(s.control_request(req, caller(&Allow), &mut r).unwrap())
    else {
        panic!("job_status result");
    };
    let Some(JobOutput::Pauses(pauses)) = snapshot.output else {
        panic!("pauses output");
    };
    assert_eq!(pauses.analyzed_sources, 1);
    assert_eq!(pauses.spans.len(), 3, "{pauses:?}");
    for span in &pauses.spans {
        assert!(span.end > span.start);
    }
    // Cancelling a second analysis reports the cancelled job without spans.
    let req = request(
        &s,
        "timeline.pause_candidates",
        json!({"min_pause_ms":500,"pad_ms":20,"crossfade_ms":20}),
        "pause-cancel",
    );
    let ControlResult::JobAccepted { job, .. } =
        ok(s.control_request(req, caller(&Allow), &mut r).unwrap())
    else {
        panic!("pause candidates must return a job");
    };
    r.cancel_job(job).unwrap();
    finish(&mut r, job);
    let req = request(&s, "job_status", json!({"job":job.0}), "pause-cancelled");
    let ControlResult::Job { snapshot } =
        ok(s.control_request(req, caller(&Allow), &mut r).unwrap())
    else {
        panic!("job_status result");
    };
    assert_eq!(snapshot.state, engine::jobs::JobState::Cancelled);
    assert!(snapshot.output.is_none());
    r.shutdown();
}
/// `media.detect_scenes` returns its candidates through `job_status` without consuming them,
/// publishes the cache entry once, and refuses what it cannot run before submitting.
#[test]
fn scene_detection_returns_candidates_warms_the_cache_and_refuses_bad_input() {
    let dir = tempfile::tempdir().unwrap();
    let video = dir.path().join("cut.mkv");
    let wav = dir.path().join("voice.wav");
    write_video(&video);
    write_wav(&wav, 8_000);
    let mut s = with_video(&seed(), &video);
    let mut r = runner();
    let media = Mutex::new(media_access(dir.path()));
    let job = submit_media(
        &mut s,
        &mut r,
        &media,
        "media.detect_scenes",
        json!({"asset":97}),
        "cold",
    );
    finish(&mut r, job);
    let view = media_status(&mut s, &mut r, &media, job, "cold-status");
    let Some(JobOutput::Scenes(first)) = view.output else {
        panic!("scenes output: {view:?}");
    };
    assert_eq!(view.state, JobState::Completed);
    assert_eq!(first.candidates.len(), 1, "{first:?}");
    assert_eq!(first.candidates[0].at, Tick(time::TICKS_PER_SECOND));
    assert_eq!(
        first.candidates[0].kind,
        engine::scene_detection::ChangeKind::HardCut
    );
    // Reading again returns the same retained output.
    let again = media_status(&mut s, &mut r, &media, job, "cold-again");
    assert!(matches!(again.output, Some(JobOutput::Scenes(ref second)) if *second == first));

    // The first status published the entry: a warm run completes with the source gone.
    std::fs::remove_file(&video).unwrap();
    let warm = submit_media(
        &mut s,
        &mut r,
        &media,
        "media.detect_scenes",
        json!({"asset":97}),
        "warm",
    );
    finish(&mut r, warm);
    let view = media_status(&mut s, &mut r, &media, warm, "warm-status");
    assert!(matches!(view.output, Some(JobOutput::Scenes(ref warmed)) if *warmed == first));

    // Refusals happen before a job exists: missing asset, bad settings, no video, no cache.
    let mut audio = with_waveform(&seed(), &wav);
    let caller = |media| Caller {
        principal: "test",
        external: true,
        policy: &Allow,
        fonts: None,
        sounds: None,
        music: None,
        media,
        chapters: None,
        captions: None,
        vision: None,
    };
    for (key, id, params, with_media) in [
        ("missing", "media.detect_scenes", json!({"asset":99}), true),
        (
            "threshold",
            "media.detect_scenes",
            json!({"asset":97,"settings":{"hard_threshold":2.0}}),
            true,
        ),
        (
            "audio-only",
            "media.detect_scenes",
            json!({"asset":97}),
            true,
        ),
        (
            "no-cache",
            "media.detect_scenes",
            json!({"asset":97}),
            false,
        ),
    ] {
        let target = if key == "audio-only" {
            &mut audio
        } else {
            &mut s
        };
        let req = request(target, id, params, key);
        error(
            target
                .control_request(req, caller(with_media.then_some(&media)), &mut r)
                .unwrap(),
            ErrorCode::Refused,
        );
    }
    r.shutdown();
}
/// `audio.measure_loudness` measures without editing: its gains reach the project only
/// through `audio.apply_loudness` bound to the revision the job started at.
#[test]
fn loudness_job_returns_gains_and_only_an_edit_at_the_started_revision_applies_them() {
    let dir = tempfile::tempdir().unwrap();
    let wav = dir.path().join("voice.wav");
    write_wav(&wav, 8_000);
    let mut s = with_loudness(&seed(), &wav);
    let mut r = runner();
    let started = s.revision();
    let before = project::encode(s.project(), false).unwrap();
    let ControlResult::JobAccepted { job, revision, .. } = ok(run(
        &mut s,
        &mut r,
        "audio.measure_loudness",
        json!({}),
        "measure",
    )) else {
        panic!("measure must return a job");
    };
    assert_eq!(revision, started);
    finish(&mut r, job);
    let ControlResult::Job { snapshot } = ok(run(
        &mut s,
        &mut r,
        "job_status",
        json!({"job":job.0}),
        "status",
    )) else {
        panic!("job status");
    };
    let Some(JobOutput::Loudness(result)) = snapshot.output else {
        panic!("loudness output");
    };
    assert!(result.master.is_some(), "{result:?}");
    // The job edited nothing.
    assert_eq!(project::encode(s.project(), false).unwrap(), before);
    assert!(matches!(
        engine::loudness::admit(s.project()),
        Err(engine::RuntimeError::LoudnessStale)
    ));

    // An edit after the job started makes the started revision stale.
    ok(run(
        &mut s,
        &mut r,
        "audio.set_master",
        json!({"volume":-3.0,"muted":false}),
        "trim",
    ));
    let mut stale = request(
        &s,
        "audio.apply_loudness",
        json!({"gains":result.gains}),
        "stale",
    );
    stale.base_rev = Some(started);
    error(
        s.control_request(stale, caller(&Allow), &mut r).unwrap(),
        ErrorCode::StaleRevision,
    );
    // The master fader is outside the measured content, so the current revision applies.
    ok(run(
        &mut s,
        &mut r,
        "audio.apply_loudness",
        json!({"gains":result.gains}),
        "apply",
    ));
    engine::loudness::admit(s.project()).unwrap();

    // No target set, and a cancelled job without output.
    let mut untargeted = seed();
    let req = request(&untargeted, "audio.measure_loudness", json!({}), "none");
    error(
        untargeted
            .control_request(req, caller(&Allow), &mut r)
            .unwrap(),
        ErrorCode::Refused,
    );
    let ControlResult::JobAccepted { job, .. } = ok(run(
        &mut s,
        &mut r,
        "audio.measure_loudness",
        json!({}),
        "cancel",
    )) else {
        panic!("measure must return a job");
    };
    r.cancel_job(job).unwrap();
    finish(&mut r, job);
    let ControlResult::Job { snapshot } = ok(run(
        &mut s,
        &mut r,
        "job_status",
        json!({"job":job.0}),
        "cancelled",
    )) else {
        panic!("job status");
    };
    assert_eq!(snapshot.state, JobState::Cancelled);
    assert!(snapshot.output.is_none());
    r.shutdown();
}
/// Generation commands warm the cache through an attached media access, cancel
/// before publication and refuse without the caches.
#[test]
fn media_generation_commands_warm_the_cache_and_refuse_without_it() {
    let dir = tempfile::tempdir().unwrap();
    let wav = dir.path().join("voice.wav");
    let changed = dir.path().join("voice-16k.wav");
    let png = dir.path().join("still.png");
    write_wav(&wav, 8_000);
    write_wav(&changed, 16_000);
    write_png(&png);
    let mut wave_session = with_waveform(&seed(), &wav);
    let mut thumbnail_session = with_thumbnails(&seed(), &png);
    let mut runner = runner();
    let media = Mutex::new(media_access(dir.path()));

    let wave = submit_media(
        &mut wave_session,
        &mut runner,
        &media,
        "media.generate_waveform",
        json!({"asset":97}),
        "wave-cold",
    );
    finish(&mut runner, wave);
    assert_eq!(
        media_state(&mut wave_session, &mut runner, &media, wave, "wave"),
        JobState::Completed
    );

    // A warm entry completes with the source gone: submission never opens it.
    std::fs::remove_file(&wav).unwrap();
    let warm = submit_media(
        &mut wave_session,
        &mut runner,
        &media,
        "media.generate_waveform",
        json!({"asset":97}),
        "wave-warm",
    );
    finish(&mut runner, warm);
    assert_eq!(
        media_state(&mut wave_session, &mut runner, &media, warm, "wave-warm"),
        JobState::Completed
    );

    let thumbs = submit_media(
        &mut thumbnail_session,
        &mut runner,
        &media,
        "media.generate_clip_thumbnails",
        json!({"item":6,"visible":{"start":0,"end":254016000000_i64},"pixels_per_second":71.5}),
        "thumbs-cold",
    );
    finish(&mut runner, thumbs);
    assert_eq!(
        media_state(
            &mut thumbnail_session,
            &mut runner,
            &media,
            thumbs,
            "thumbs"
        ),
        JobState::Completed
    );
    std::fs::remove_file(&png).unwrap();
    let warm_thumbs = submit_media(
        &mut thumbnail_session,
        &mut runner,
        &media,
        "media.generate_clip_thumbnails",
        json!({"item":6,"visible":{"start":0,"end":254016000000_i64},"pixels_per_second":71.5}),
        "thumbs-warm",
    );
    finish(&mut runner, warm_thumbs);
    assert_eq!(
        media_state(
            &mut thumbnail_session,
            &mut runner,
            &media,
            warm_thumbs,
            "thumbs-warm"
        ),
        JobState::Completed
    );

    // Different audio layout is a cold key; cancellation publishes nothing.
    let mut cancel_session = with_waveform(&seed(), &changed);
    let cancelled = submit_media(
        &mut cancel_session,
        &mut runner,
        &media,
        "media.generate_waveform",
        json!({"asset":97}),
        "wave-cancel",
    );
    runner.cancel_job(cancelled).unwrap();
    finish(&mut runner, cancelled);
    assert_eq!(
        media_state(
            &mut cancel_session,
            &mut runner,
            &media,
            cancelled,
            "wave-cancelled"
        ),
        JobState::Cancelled
    );
    // The earlier summary survived the cancellation.
    let again = submit_media(
        &mut wave_session,
        &mut runner,
        &media,
        "media.generate_waveform",
        json!({"asset":97}),
        "wave-after-cancel",
    );
    finish(&mut runner, again);
    assert_eq!(
        media_state(&mut wave_session, &mut runner, &media, again, "wave-after"),
        JobState::Completed
    );

    // Without attached caches the same commands refuse before submitting.
    for (key, id, params) in [
        (
            "no-media-wave",
            "media.generate_waveform",
            json!({"asset":97}),
        ),
        (
            "no-media-thumbs",
            "media.generate_clip_thumbnails",
            json!({"item":6,"visible":{"start":0,"end":254016000000_i64},"pixels_per_second":71.5}),
        ),
    ] {
        let req = request(&wave_session, id, params, key);
        error(
            wave_session
                .control_request(req, caller(&Allow), &mut runner)
                .unwrap(),
            ErrorCode::Refused,
        );
    }
    // A missing target is a typed refusal with the caches attached.
    let req = request(
        &wave_session,
        "media.generate_waveform",
        json!({"asset":99}),
        "missing-asset",
    );
    let caller = Caller {
        principal: "test",
        external: true,
        policy: &Allow,
        fonts: None,
        sounds: None,
        music: None,
        media: Some(&media),
        chapters: None,
        captions: None,
        vision: None,
    };
    error(
        wave_session
            .control_request(req, caller, &mut runner)
            .unwrap(),
        ErrorCode::Refused,
    );
    for (key, pixels_per_second) in [("zero-zoom", 0.0), ("negative-zoom", -1.0)] {
        let req = request(
            &thumbnail_session,
            "media.generate_clip_thumbnails",
            json!({"item":6,"visible":{"start":0,"end":254016000000_i64},"pixels_per_second":pixels_per_second}),
            key,
        );
        let caller = Caller {
            principal: "test",
            external: true,
            policy: &Allow,
            fonts: None,
            sounds: None,
            music: None,
            media: Some(&media),
            chapters: None,
            captions: None,
            vision: None,
        };
        error(
            thumbnail_session
                .control_request(req, caller, &mut runner)
                .unwrap(),
            ErrorCode::Refused,
        );
    }
    runner.shutdown();
}
#[test]
fn chapters_control_preserves_resolved_custom_model_and_requires_unknown_price_acknowledgement() {
    use engine::providers::{ConsentPrompt, TextAccess, UploadRequest};
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };
    struct Wire(AtomicUsize);
    impl transport::HttpTransport for Wire {
        fn send(
            &self,
            request: transport::HttpRequest,
        ) -> Result<transport::HttpResponse, transport::Error> {
            self.0.fetch_add(1, Ordering::SeqCst);
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            assert_eq!(body["model"], "fixture-control-model");
            let input: Value = serde_json::from_str(body["input"].as_str().unwrap()).unwrap();
            let proposal = json!({"title":"Fixture title","description":"Fixture description","chapters":[{"start_segment_id":input["segments"][0]["id"],"title":"Opening"}]});
            Ok(transport::HttpResponse {status:200,headers:vec![],body:serde_json::to_vec(&json!({"status":"completed","output":[{"type":"message","role":"assistant","status":"completed","content":[{"type":"output_text","text":proposal.to_string()}]}]})).unwrap()})
        }
    }
    struct Prompt(Arc<AtomicBool>);
    impl ConsentPrompt for Prompt {
        fn approve(&self, _: &UploadRequest) -> engine::providers::Result<()> {
            Ok(())
        }
        fn approve_unknown_price(&self, request: &UploadRequest) -> engine::providers::Result<()> {
            assert_eq!(request.model, "fixture-control-model");
            assert!(matches!(request.cost, engine::providers::Cost::Unknown));
            if self.0.load(Ordering::SeqCst) {
                Ok(())
            } else {
                Err(ai::Error::Cost.into())
            }
        }
    }
    let wire = Arc::new(Wire(AtomicUsize::new(0)));
    let accepted = Arc::new(AtomicBool::new(false));
    let config =
        ai::chapters::OpenAiChapterGenerator::provider_config("fixture-control-model").unwrap();
    let filler =
        ai::fillers::OpenAiFillerClassifier::provider_config("fixture-control-model").unwrap();
    let access = TextAccess::new(
        Arc::new(
            ai::chapters::OpenAiChapterGenerator::new(
                config,
                ai::transport::ApiKey::new("synthetic").unwrap(),
                wire.clone(),
            )
            .unwrap(),
        ),
        Arc::new(
            ai::fillers::OpenAiFillerClassifier::new(
                filler,
                ai::transport::ApiKey::new("synthetic").unwrap(),
                wire.clone(),
            )
            .unwrap(),
        ),
        Box::new(Prompt(accepted.clone())),
    );
    let mut session = with_words(&seed());
    let mut runner = runner();
    let caller = || Caller {
        principal: "fixture",
        external: false,
        policy: &Allow,
        fonts: None,
        sounds: None,
        music: None,
        media: None,
        chapters: Some(&access),
        captions: None,
        vision: None,
    };
    let req = request(
        &session,
        "chapters.generate",
        json!({"language":"en","max_chapters":2}),
        "custom-refused",
    );
    let refused = session.control_request(req, caller(), &mut runner).unwrap();
    assert!(!matches!(
        refused.value,
        ResponseValue::Result(ControlResult::JobAccepted { .. })
    ));
    assert_eq!(wire.0.load(Ordering::SeqCst), 0);
    accepted.store(true, Ordering::SeqCst);
    let req = request(
        &session,
        "chapters.generate",
        json!({"language":"en","max_chapters":2}),
        "custom-accepted",
    );
    let ControlResult::JobAccepted { job, .. } =
        ok(session.control_request(req, caller(), &mut runner).unwrap())
    else {
        panic!("expected job")
    };
    finish(&mut runner, job);
    assert_eq!(runner.snapshot(job).unwrap().state, JobState::Completed);
    assert_eq!(wire.0.load(Ordering::SeqCst), 1);
    runner.shutdown();
}

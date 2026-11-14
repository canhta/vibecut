#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
//! Export evidence that needs the window registry, not one dialog alone.
use egui_kittest::kittest::Queryable;
use std::sync::{Arc, Condvar, Mutex, mpsc};
use std::time::{Duration, Instant};
use vibecut::egui;
use vibecut::preferences::Preferences;
use vibecut::services::playback::Sources;

fn project(empty: bool, source: bool) -> project::Project {
    let mut d = project::Project::empty(project::ProjectId(18), "Xuất video".into())
        .unwrap()
        .into_data();
    d.canvas.width = 64;
    d.canvas.height = 64;
    let range =
        time::SourceRange::new(time::Tick::ZERO, time::Tick(time::TICKS_PER_SECOND / 10)).unwrap();
    if source {
        d.assets.push(
            testkit::source::asset(
                1,
                std::env::temp_dir().join("vibecut-export-source-missing.mp4"),
            )
            .format("mp4")
            .duration(range.duration())
            .content_hash(media::ContentHash::from_bytes(b"source"))
            .stream(
                testkit::source::video()
                    .time_base(media::TimeBase::new(1, 30).unwrap())
                    .duration(range.duration())
                    .frame_count(3)
                    .variable_frame_rate(false)
                    .size(64, 64)
                    .build(),
            )
            .build(),
        );
    }
    if !empty {
        d.scene_mut().timeline.main.items.push(
            testkit::item::item(
                project::ItemId(1),
                if source {
                    project::ItemParams::Video(project::VideoParams::plain())
                } else {
                    project::ItemParams::Overlay(project::OverlayParams {
                        color: project::Color([100, 150, 200, 255]),
                    })
                },
            )
            .source_asset(source.then_some(media::AssetId(1)))
            .source(range)
            .build(),
        );
    }
    project::Project::new(d).unwrap()
}

struct Gate {
    entered: Mutex<mpsc::Sender<()>>,
    release: (Mutex<bool>, Condvar),
}
struct Release(Arc<Gate>);
impl Drop for Release {
    fn drop(&mut self) {
        *self.0.release.0.lock().unwrap() = true;
        self.0.release.1.notify_all();
    }
}
struct GateFactory(Arc<Gate>);
impl Sources for GateFactory {
    fn create(&self) -> Box<dyn render::SourceProvider> {
        Box::new(Gated(self.0.clone()))
    }
}
struct Gated(Arc<Gate>);
impl render::SourceProvider for Gated {
    fn status(
        &mut self,
        _: &media::SourceAsset,
        _: bool,
        hooks: &dyn render::ExecutionHooks,
    ) -> render::Result<Option<render::UnavailableReason>> {
        self.0.entered.lock().unwrap().send(()).unwrap();
        let mut release = self.0.release.0.lock().unwrap();
        while !*release {
            release = self.0.release.1.wait(release).unwrap();
        }
        if hooks.cancelled() {
            Err(render::Error::Cancelled)
        } else {
            Err(render::Error::Unsupported("injected source failure"))
        }
    }
    fn video(
        &mut self,
        _: &media::SourceAsset,
        _: project::ItemId,
        _: time::Tick,
        _: &dyn render::ExecutionHooks,
    ) -> render::Result<Option<render::Image>> {
        panic!("unexpected source request after injected refusal")
    }
    fn audio(
        &mut self,
        _: &media::SourceAsset,
        _: project::ItemId,
        _: i64,
        _: usize,
        _: u32,
        _: &dyn render::ExecutionHooks,
    ) -> render::Result<render::AudioBlock> {
        panic!("unexpected source request after injected refusal")
    }
}

#[test]
fn closing_one_window_preserves_another_windows_dirty_edit_and_running_export() {
    use vibecut::app::Workspace;
    fn idle(app: &mut Workspace) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while app.busy() {
            app.tick();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    let root = tempfile::tempdir().unwrap();
    let first_path = root.path().join("first.vibecut");
    let second_path = root.path().join("second.vibecut");
    project::WriterLease::acquire(&first_path)
        .unwrap()
        .save(&project::Project::empty(project::ProjectId(1), "first".into()).unwrap())
        .unwrap();
    project::WriterLease::acquire(&second_path)
        .unwrap()
        .save(&project(false, true))
        .unwrap();
    let second_bytes = std::fs::read(&second_path).unwrap();
    let mut app =
        Workspace::new(root.path().join("recovery"), Preferences::default(), None).unwrap();
    idle(&mut app);
    app.open(first_path.clone()).unwrap();
    idle(&mut app);
    app.open(second_path.clone()).unwrap();
    idle(&mut app);
    let first = project::ProjectId(1);
    let second = project::ProjectId(18);
    let (send, recv) = mpsc::channel();
    let gate = Arc::new(Gate {
        entered: Mutex::new(send),
        release: (Mutex::new(false), Condvar::new()),
    });
    let release = Release(gate.clone());
    let editor = app.editor_mut(second).unwrap();
    editor
        .session
        .apply(engine::Command::RenameProject {
            name: "unsaved second".into(),
        })
        .unwrap();
    editor.activate("file.export").unwrap();
    editor
        .export_dialog()
        .unwrap()
        .set_sources(Arc::new(GateFactory(gate.clone())))
        .unwrap();
    editor.activate("export.run").unwrap();
    let revision = editor.session.revision();
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        app.tick();
        if recv.try_recv().is_ok() {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    let job = app
        .editor_mut(second)
        .unwrap()
        .export_state()
        .unwrap()
        .inspect()
        .job
        .unwrap()
        .id;
    app.request_close();
    assert!(!app.home_open());
    assert!(!app.ready_to_close());
    app.focus(first).unwrap();
    // A saved, unchanged project closes without asking; an edited one asks first.
    app.editor_mut(first)
        .unwrap()
        .session
        .apply(engine::Command::RenameProject {
            name: "unsaved first".into(),
        })
        .unwrap();
    app.windows.get_mut(&first).unwrap().request_close();
    let mut h = crate::gpu::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .build_ui_state(
            move |ui, a: &mut Workspace| {
                a.tick();
                if a.windows.contains_key(&first) {
                    a.window_ui(first, ui);
                }
            },
            app,
        );
    let _release = release;
    h.run_steps(3);
    h.get(
        egui_kittest::kittest::By::new()
            .predicate(|n| n.author_id() == Some("project.close.cancel")),
    )
    .click();
    h.run_steps(3);
    assert!(
        h.state_mut().editor_mut(first).is_some(),
        "cancel retains only the requested window"
    );
    h.state_mut()
        .windows
        .get_mut(&first)
        .unwrap()
        .request_close();
    h.run_steps(3);
    h.get(
        egui_kittest::kittest::By::new()
            .predicate(|n| n.author_id() == Some("project.close.recover")),
    )
    .click();
    h.run_steps(2);
    idle(h.state_mut());
    assert!(!h.state().windows.contains_key(&first));
    assert!(project::WriterLease::acquire(&first_path).is_ok());
    let editor = h.state_mut().editor_mut(second).unwrap();
    assert_eq!(editor.session.revision(), revision);
    assert_eq!(
        editor.session.project().data().name.as_str(),
        "unsaved second"
    );
    assert!(editor.session.autosave().unwrap().is_dirty());
    assert!(editor.export_state().unwrap().active());
    assert_eq!(
        editor.export_state().unwrap().inspect().job.unwrap().id,
        job
    );
    assert!(matches!(
        project::WriterLease::acquire(&second_path),
        Err(project::Error::Busy)
    ));
    assert_eq!(std::fs::read(&second_path).unwrap(), second_bytes);
    *gate.release.0.lock().unwrap() = true;
    gate.release.1.notify_all();
    while h
        .state_mut()
        .editor_mut(second)
        .unwrap()
        .export_state()
        .unwrap()
        .active()
    {
        h.state_mut().tick();
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    h.state_mut()
        .windows
        .get_mut(&second)
        .unwrap()
        .persist(None, true, true, true)
        .unwrap();
    idle(h.state_mut());
    assert!(h.state().ready_to_close());
    assert!(project::WriterLease::acquire(&second_path).is_ok());
}

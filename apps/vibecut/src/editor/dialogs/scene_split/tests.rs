//! Drives the scene split dialog alone over the one-area bench, through the real detection job.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
use super::{Action, DetectSource, Phase, State, apply, show};
use crate::editor::frame::bench::Bench;
use crate::editor::{Editor, egui};
use crate::i18n::{Locale, Text};
use crate::preferences::Preferences;
use crate::services::playback::{Files, Playback};
use egui_kittest::kittest::Queryable;
use engine::{ByteBudget, Command, CreatedId, NewItem, Placement, Session, control::MediaAccess};
use media::{AssetId, SourceAsset};
use project::{ItemId, ItemParams, Project, ProjectId, VideoParams};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use time::{SourceRange, Tick};

type Mounted = Bench<State, Action>;

/// Two solid colours joined by a hard cut at one second, one second each side.
const CUT: &str =
    "color=c=red:s=64x64:r=10:d=1[a];color=c=blue:s=64x64:r=10:d=1[b];[a][b]concat=n=2:v=1:a=0[v]";
/// One colour throughout: no candidate survives.
const FLAT: &str = "color=c=gray:s=64x64:r=10:d=2[v]";

/// Renders one lavfi graph into the fixture directory, or `None` without ffmpeg.
fn render(name: &str, graph: &str) -> Option<PathBuf> {
    if !testkit::oracle::require("scene split") {
        return None;
    }
    let dir = testkit::fixture::dir("p2-ai-12-scene-split");
    Some(testkit::fixture::publish(&dir, name, |temp| {
        let status = testkit::oracle::command("ffmpeg")
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-filter_complex",
                graph,
                "-map",
                "[v]",
                "-c:v",
                "ffv1",
                "-f",
                "matroska",
            ])
            .arg(temp)
            .status()
            .unwrap();
        assert!(status.success());
    }))
}

/// A project with one video clip of `path` selected, and a media cache attached.
fn mounted(path: &Path, cache: &tempfile::TempDir) -> (Mounted, ItemId) {
    let mut session =
        Session::new(Project::empty(ProjectId(12), "Scenes".into()).unwrap(), 50).unwrap();
    let asset = SourceAsset::import(AssetId(1), path.to_path_buf()).unwrap();
    let duration = asset.probe().duration.unwrap();
    session.apply(Command::AddAsset { asset }).unwrap();
    let created = session
        .apply(Command::InsertItem {
            placement: Placement::Auto,
            item: NewItem {
                retime: Default::default(),
                animations: Default::default(),
                source_asset: Some(AssetId(1)),
                source_range: SourceRange::new(Tick::ZERO, duration).unwrap(),
                start: Tick::ZERO,
                content: ItemParams::Video(VideoParams::plain()),
            },
        })
        .unwrap()
        .created;
    let item = match created.first() {
        Some(CreatedId::Item(id)) => *id,
        _ => panic!("item"),
    };
    let playback = Playback::new(
        &session,
        ByteBudget::new(256 << 20).unwrap(),
        Arc::new(Files),
    )
    .unwrap();
    let mut editor = Editor::new(session, playback, Preferences::default());
    editor.select(BTreeSet::from([item])).unwrap();
    let media = MediaAccess::open_in(cache.path(), ByteBudget::new(256 << 20).unwrap()).unwrap();
    editor
        .dialogs
        .scene_split
        .set_media(Arc::new(Mutex::new(media)));
    let bench = Bench::hosted(
        editor,
        |editor| std::mem::take(&mut editor.dialogs.scene_split),
        |editor, state| editor.dialogs.scene_split = state,
        show,
        route,
        egui::vec2(640.0, 480.0),
    );
    (bench, item)
}

fn route(editor: &mut Editor, actions: Vec<Action>) {
    for action in actions {
        editor.with_dialogs(|state, port| apply(&mut state.scene_split, port, action));
    }
}

fn open(bench: &mut Mounted) {
    let editor = bench.editor_mut();
    editor.dialogs.scene_split.open(&editor.session).unwrap();
    // A modal's first frame only sizes it; its widgets reach the tree on the next.
    bench.harness_mut().run_steps(2);
}

/// Frames until the dialog reaches a state the predicate accepts.
fn settle(bench: &mut Mounted, done: impl Fn(&State) -> bool) {
    let started = Instant::now();
    while !done(&bench.editor().dialogs.scene_split) {
        assert!(
            started.elapsed() < Duration::from_secs(120),
            "scene detection did not settle"
        );
        bench.step();
        std::thread::sleep(Duration::from_millis(10));
    }
    // The state changed after the last frame painted; paint it once before it is queried.
    bench.step();
}

fn clip_count(bench: &Mounted) -> usize {
    bench
        .editor()
        .session
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| &track.items)
        .count()
}

#[test]
fn opens_only_for_one_selected_video_clip() {
    let Some(path) = render("cut.mkv", CUT) else {
        return;
    };
    let cache = tempfile::tempdir().unwrap();
    let (mut bench, item) = mounted(&path, &cache);
    let editor = bench.editor_mut();
    editor.select(BTreeSet::new()).unwrap();
    assert!(editor.dialogs.scene_split.open(&editor.session).is_err());
    editor.select(BTreeSet::from([item])).unwrap();
    assert!(editor.dialogs.scene_split.open(&editor.session).is_ok());
    assert!(editor.dialogs.scene_split.is_open());
}

#[test]
fn a_missing_media_cache_refuses_the_search_and_keeps_the_timeline() {
    let Some(path) = render("cut.mkv", CUT) else {
        return;
    };
    let cache = tempfile::tempdir().unwrap();
    let (mut bench, _) = mounted(&path, &cache);
    bench.editor_mut().dialogs.scene_split.media = None;
    open(&mut bench);
    bench.click("scene_split.find");
    assert!(matches!(
        bench.editor().dialogs.scene_split.phase,
        Phase::Failed(_)
    ));
    assert_eq!(clip_count(&bench), 1);
}

#[test]
fn detect_review_apply_splits_every_reviewed_cut() {
    let Some(path) = render("cut.mkv", CUT) else {
        return;
    };
    let cache = tempfile::tempdir().unwrap();
    let (mut bench, _) = mounted(&path, &cache);
    open(&mut bench);
    bench.click("scene_split.find");
    settle(&mut bench, |state| {
        matches!(state.phase, Phase::Review { .. })
    });
    let candidates = match &bench.editor().dialogs.scene_split.phase {
        Phase::Review { result, .. } => result.candidates.len(),
        _ => panic!("review"),
    };
    assert!(candidates >= 1);
    // The count line picks the locale's plural row: one candidate reads in the singular.
    let line = if candidates == 1 {
        "1 scene change".to_owned()
    } else {
        format!("{candidates} scene changes")
    };
    assert!(
        bench.harness_mut().query_by_label(&line).is_some(),
        "the review shows {line:?}"
    );
    let screenshot =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/p2-ai-05-scene-split.png");
    std::fs::create_dir_all(screenshot.parent().unwrap()).unwrap();
    bench
        .harness_mut()
        .render()
        .unwrap()
        .save(&screenshot)
        .unwrap();
    bench.click("scene_split.apply");
    assert!(!bench.editor().dialogs.scene_split.is_open());
    assert_eq!(clip_count(&bench), 1 + candidates);
}

/// A scripted detection job whose body is held open by `gate` until it is cancelled. `started`
/// is set once the body runs, so a test clicks Cancel while the job is provably running.
/// With `gate` never opened, the only way the job ends is the dialog's cancel.
fn held(started: Arc<AtomicBool>, gate: Arc<AtomicBool>) -> DetectSource {
    Arc::new(
        move |runner: &mut engine::jobs::JobRunner, _asset: AssetId| {
            let key = gated_key()?;
            let started = started.clone();
            let gate = gate.clone();
            runner
                .submit::<engine::control::Error>(
                    engine::jobs::JobSpec {
                        kind: engine::jobs::JobKind::DetectScenes,
                        class: engine::jobs::ResourceClass::Cpu,
                        input: key,
                        bytes: Default::default(),
                    },
                    move |context| {
                        started.store(true, Ordering::SeqCst);
                        while !gate.load(Ordering::SeqCst) {
                            context.check_cancelled()?;
                            std::thread::yield_now();
                        }
                        Ok(Vec::new())
                    },
                )
                .ok()
                .map(|(job, _)| job)
        },
    )
}

/// A scripted detection job that reports a typed failure as soon as it runs.
fn failing() -> DetectSource {
    Arc::new(
        move |runner: &mut engine::jobs::JobRunner, _asset: AssetId| {
            let key = gated_key()?;
            runner
                .submit::<engine::control::Error>(
                    engine::jobs::JobSpec {
                        kind: engine::jobs::JobKind::DetectScenes,
                        class: engine::jobs::ResourceClass::Cpu,
                        input: key,
                        bytes: Default::default(),
                    },
                    move |_context| -> Result<Vec<u8>, engine::control::Error> {
                        Err(engine::control::Error::Runtime(
                            engine::RuntimeError::CacheCorrupt,
                        ))
                    },
                )
                .ok()
                .map(|(job, _)| job)
        },
    )
}

/// The cache key the scripted jobs are admitted under; nothing else reads it.
fn gated_key() -> Option<engine::cache::CacheKey> {
    engine::cache::CacheKey::new(engine::cache::KeyInputs {
        kind: engine::cache::ArtifactKind::SceneDetection,
        source: [0; 32],
        provider: "fixture",
        model: "gate",
        version: "1",
        settings: &[],
        transcript: [0; 32],
        cues: [0; 32],
        clip_range: [0; 32],
        translation: [0; 32],
    })
    .ok()
}

#[test]
fn cancel_during_detection_leaves_the_timeline_unchanged() {
    let Some(path) = render("cut.mkv", CUT) else {
        return;
    };
    let cache = tempfile::tempdir().unwrap();
    let (mut bench, _) = mounted(&path, &cache);
    let started = Arc::new(AtomicBool::new(false));
    let gate = Arc::new(AtomicBool::new(false));
    bench
        .editor_mut()
        .dialogs
        .scene_split
        .set_detect_source(held(started.clone(), gate.clone()));
    open(&mut bench);
    bench.click("scene_split.find");
    // The job body has run and is parked on the gate, so it cannot finish before Cancel.
    settle(&mut bench, |_| started.load(Ordering::SeqCst));
    assert!(matches!(
        bench.editor().dialogs.scene_split.phase,
        Phase::Detecting { .. }
    ));
    bench.click("scene_split.cancel");
    settle(&mut bench, |state| matches!(state.phase, Phase::Cancelled));
    assert!(!gate.load(Ordering::SeqCst));
    assert_eq!(clip_count(&bench), 1);
}

#[test]
fn the_idle_frame_offers_find_and_close_without_a_job() {
    let Some(path) = render("cut.mkv", CUT) else {
        return;
    };
    let cache = tempfile::tempdir().unwrap();
    let (mut bench, _) = mounted(&path, &cache);
    open(&mut bench);
    bench.step();
    assert!(matches!(
        bench.editor().dialogs.scene_split.phase,
        Phase::Idle
    ));
    assert!(
        bench
            .harness_mut()
            .query_by_label(Text::SceneSplitIdle.text(Locale::En))
            .is_some()
    );
    assert_eq!(clip_count(&bench), 1);
}

#[test]
fn a_job_that_reports_failure_shows_the_typed_line_and_keeps_the_timeline() {
    let Some(path) = render("cut.mkv", CUT) else {
        return;
    };
    let cache = tempfile::tempdir().unwrap();
    let (mut bench, _) = mounted(&path, &cache);
    bench
        .editor_mut()
        .dialogs
        .scene_split
        .set_detect_source(failing());
    open(&mut bench);
    bench.click("scene_split.find");
    settle(&mut bench, |state| {
        matches!(state.phase, Phase::Failed(Text::SceneSplitFailed))
    });
    assert!(
        bench
            .harness_mut()
            .query_by_label(Text::SceneSplitFailed.text(Locale::En))
            .is_some()
    );
    assert_eq!(clip_count(&bench), 1);
}

#[test]
fn a_clip_without_cuts_reports_no_scene_changes() {
    let Some(path) = render("flat.mkv", FLAT) else {
        return;
    };
    let cache = tempfile::tempdir().unwrap();
    let (mut bench, _) = mounted(&path, &cache);
    open(&mut bench);
    bench.click("scene_split.find");
    settle(&mut bench, |state| matches!(state.phase, Phase::NoCuts));
    bench.click("scene_split.close");
    assert!(!bench.editor().dialogs.scene_split.is_open());
    assert_eq!(clip_count(&bench), 1);
}

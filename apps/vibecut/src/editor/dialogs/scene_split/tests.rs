//! Drives the scene split dialog alone over the one-area bench, through the real detection job.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
use super::{Action, Phase, State, apply, show};
use crate::editor::frame::bench::Bench;
use crate::editor::{Editor, egui};
use crate::preferences::Preferences;
use crate::services::playback::{Files, Playback};
use engine::{ByteBudget, Command, CreatedId, NewItem, Placement, Session, control::MediaAccess};
use media::{AssetId, SourceAsset};
use project::{ItemId, ItemParams, Project, ProjectId, VideoParams};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use time::{SourceRange, Tick};

type Mounted = Bench<State, Action>;

/// Two solid colours joined by a hard cut at one second, one second each side.
const CUT: &str =
    "color=c=red:s=64x64:r=10:d=1[a];color=c=blue:s=64x64:r=10:d=1[b];[a][b]concat=n=2:v=1:a=0[v]";
/// A long, large clip: the job is still running when the test clicks Cancel.
const LONG: &str = "testsrc2=s=1280x720:r=30:d=20[v]";
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

#[test]
fn cancel_during_detection_leaves_the_timeline_unchanged() {
    let Some(path) = render("long.mkv", LONG) else {
        return;
    };
    let cache = tempfile::tempdir().unwrap();
    let (mut bench, _) = mounted(&path, &cache);
    open(&mut bench);
    bench.click("scene_split.find");
    settle(&mut bench, |state| {
        matches!(state.phase, Phase::Detecting { .. })
    });
    bench.click("scene_split.cancel");
    settle(&mut bench, |state| matches!(state.phase, Phase::Cancelled));
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

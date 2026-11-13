#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::expect_used
)]
//! Freeze frame and replace media through the desktop editor.
use crate::timeline_ui::{fixture, harness, widget};
use engine::{ByteBudget, Command, Session};
use media::{AssetId, ContentHash, SourceAsset, StreamKind};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use time::{TICKS_PER_SECOND, Tick};
use vibecut::{AppCommand, Editor, services::lifecycle::EditorSession};

const SECOND: i64 = TICKS_PER_SECOND;

fn video_asset(id: u64, path: PathBuf, duration: Tick) -> SourceAsset {
    testkit::source::asset(id, path)
        .format("mov,mp4")
        .duration(duration)
        .content_hash(ContentHash::from_bytes(&[id as u8; 8]))
        .stream(
            testkit::source::stream(StreamKind::Video)
                .duration(duration)
                .build(),
        )
        .build()
}

/// Make the editor a persistent session saved as `<root>/title.vibecut`, so freeze
/// has a project folder to publish into.
fn saved(editor: &mut Editor, root: &Path) {
    let session = Session::new((**editor.session.project()).clone(), 10).unwrap();
    let mut saving = engine::recovery::Autosave::new(
        &root.join("recovery"),
        session,
        ByteBudget::new(64 << 20).unwrap(),
        Arc::new(engine::jobs::SystemClock::default()),
        Default::default(),
    )
    .unwrap();
    saving.save_as(root.join("title.vibecut")).unwrap();
    editor.session = EditorSession::Persistent(Box::new(saving));
}

fn select_at(editor: &mut Editor, item: project::ItemId, at: i64) {
    editor
        .session
        .set_selection([item].into_iter().collect())
        .unwrap();
    editor
        .dispatch(AppCommand::Seek(Tick(at)))
        .unwrap();
}

fn freeze_stills(folder: &Path) -> usize {
    std::fs::read_dir(folder)
        .map(|entries| {
            entries
                .filter(|entry| {
                    entry
                        .as_ref()
                        .unwrap()
                        .file_name()
                        .to_string_lossy()
                        .starts_with("freeze-")
                })
                .count()
        })
        .unwrap_or(0)
}

#[test]
fn toolbar_freeze_holds_the_selected_frame_as_a_still_beside_the_project() {
    let root = tempfile::tempdir().unwrap();
    let mut f = fixture();
    saved(&mut f.editor, root.path());
    select_at(&mut f.editor, f.video_a, 2 * SECOND);
    let before = f.editor.session.project().clone();
    let mut harness = harness(f.editor);
    harness.run_steps(3);
    // The toolbar cell exists; `widget` panics when the button is absent.
    let _ = widget(&harness, "timeline.freeze");
    harness.state_mut().activate("timeline.freeze").unwrap();
    harness.run_steps(2);
    let after = harness.state().session.project().clone();
    assert_ne!(after, before, "the freeze edit changed the project");
    assert_eq!(
        freeze_stills(&root.path().join("title.media")),
        1,
        "one still published beside the saved project"
    );
    let image = harness.render().unwrap();
    let target = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target");
    std::fs::create_dir_all(&target).unwrap();
    image
        .save(target.join("p1-53-freeze-replace.png"))
        .unwrap();
    // One undo step restores the project exactly.
    harness.state_mut().activate("edit.undo").unwrap();
    assert_eq!(harness.state().session.project(), &before);
}

#[test]
fn freeze_refuses_an_unsaved_project_and_changes_nothing() {
    let mut f = fixture();
    select_at(&mut f.editor, f.video_a, 2 * SECOND);
    let before = f.editor.session.project().clone();
    let mut harness = harness(f.editor);
    harness.run_steps(3);
    let error = harness.state_mut().activate("timeline.freeze");
    assert!(error.is_err(), "an unsaved project refuses the freeze");
    assert_eq!(harness.state().session.project(), &before);
}

#[test]
fn replace_swaps_the_clip_source_and_refuses_unlisted_or_short_files() {
    let root = tempfile::tempdir().unwrap();
    let mut f = fixture();
    let other = root.path().join("other.mp4");
    let short = root.path().join("short.mp4");
    for (id, path, duration) in [
        (3, other.clone(), Tick(10 * SECOND)),
        (4, short.clone(), Tick(SECOND)),
    ] {
        f.editor
            .session
            .apply(Command::AddAsset {
                asset: video_asset(id, path, duration),
            })
            .unwrap();
    }
    let before = f.editor.session.project().clone();
    let video = f.video_a;
    let source_of = |editor: &Editor| {
        editor
            .session
            .project()
            .data()
            .scene()
            .timeline
            .iter()
            .flat_map(|track| track.items.iter())
            .find(|item| item.id == video)
            .unwrap()
            .source_asset
    };
    // A file the project does not hold refuses before any edit.
    assert!(
        f.editor
            .replace_picked_file(f.video_a, root.path().join("missing.mp4"))
            .is_err()
    );
    // A source shorter than the retained window refuses and changes nothing.
    assert!(f.editor.replace_picked_file(f.video_a, short).is_err());
    assert_eq!(f.editor.session.project(), &before);
    // The listed longer file replaces the source; undo restores the exact project.
    f.editor.replace_picked_file(f.video_a, other).unwrap();
    assert_eq!(source_of(&f.editor), Some(AssetId(3)));
    f.editor.activate("edit.undo").unwrap();
    assert_eq!(f.editor.session.project(), &before);
}

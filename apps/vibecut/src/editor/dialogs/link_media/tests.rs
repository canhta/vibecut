//! Drives the Link media dialog alone over the one-area bench: the rows, the footer picks
//! and the relink a pick produces, through the real `media.relink` command.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
use super::{Action, Hashed, Hasher, Pick, State, apply, ellipsize, find_by_name, picked, show};
use crate::editor::frame::bench::Bench;
use crate::editor::{Editor, egui};
use crate::preferences::Preferences;
use crate::services::playback::{Files, Playback};
use egui_kittest::kittest::{By, Queryable};
use engine::{ByteBudget, Command, Session};
use media::{AssetId, ContentHash, SourceAsset};
use project::{Project, ProjectId};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use time::{TICKS_PER_SECOND, Tick};

type Mounted = Bench<State, Action>;

const BYTES: &[u8] = b"the footage that was moved";

/// A ten second clip recorded at `path` with the hash of `BYTES`.
fn recorded(id: u64, path: &Path) -> SourceAsset {
    testkit::source::asset(id, path.to_path_buf())
        .format("mp4")
        .duration(Tick(10 * TICKS_PER_SECOND))
        .hash(ContentHash::from_bytes(BYTES).0)
        .stream(
            testkit::source::video()
                .duration(Tick(10 * TICKS_PER_SECOND))
                .build(),
        )
        .build()
}

/// One project whose only asset's file is not where it was recorded.
fn mounted(root: &Path) -> Mounted {
    let mut session =
        Session::new(Project::empty(ProjectId(7), "Link".into()).unwrap(), 20).unwrap();
    session
        .apply(Command::AddAsset {
            asset: recorded(1, &root.join("gone").join("clip.mp4")),
        })
        .unwrap();
    let playback = Playback::new(
        &session,
        ByteBudget::new(64 << 20).unwrap(),
        Arc::new(Files),
    )
    .unwrap();
    let editor = Editor::new(session, playback, Preferences::default());
    Bench::hosted(
        editor,
        |editor| std::mem::take(&mut editor.dialogs.link_media),
        |editor, state| editor.dialogs.link_media = state,
        show,
        route,
        egui::vec2(760.0, 640.0),
    )
}

fn route(editor: &mut Editor, actions: Vec<Action>) {
    for action in actions {
        editor.with_dialogs(|state, port| apply(&mut state.link_media, port, action));
    }
}

fn open(bench: &mut Mounted) {
    let editor = bench.editor_mut();
    editor.dialogs.link_media.open(editor.session.project());
    // A modal's first frame only sizes it; its widgets reach the tree on the next.
    bench.harness_mut().run_steps(2);
}

/// Frames until the background match has finished and its result was committed or refused.
fn settle(bench: &mut Mounted) {
    let started = Instant::now();
    while bench.editor().dialogs.link_media.job.is_some() {
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "the match did not finish"
        );
        bench.step();
        std::thread::sleep(Duration::from_millis(5));
    }
    bench.step();
}

fn recorded_path(bench: &Mounted) -> std::path::PathBuf {
    bench.editor().session.project().data().assets[0]
        .path()
        .to_owned()
}

#[test]
fn the_dialog_lists_the_missing_source_in_the_measured_columns() {
    let root = tempfile::tempdir().unwrap();
    let mut bench = mounted(root.path());
    open(&mut bench);
    let editor = bench.editor();
    let view = editor.dialogs.link_media.view(editor.session.project());
    assert!(view.open && view.folder);
    assert_eq!(view.names, ["clip.mp4"]);
    assert_eq!(view.linked, 0);
    // Painted column heads are not widgets; the title, body line and footer controls are.
    let harness = bench.harness_mut();
    for label in ["Link media", "Cancel", "Search the folder I pick"] {
        assert!(
            harness.query_all_by_label_contains(label).next().is_some(),
            "{label} is on the dialog"
        );
    }
    // The list is 524 x 317 inside the 556 px dialog (capture 095518, logical px).
    let row = bench
        .harness()
        .get(By::new().predicate(|node| node.author_id() == Some("link_media.row.1")))
        .rect();
    assert!(
        (row.width() - 524.0).abs() < 1.0,
        "row width {}",
        row.width()
    );
    let link = bench
        .harness()
        .get(By::new().predicate(|node| node.author_id() == Some("link_media.link")))
        .rect();
    assert!((link.width() - 72.0).abs() < 1.0 && (link.height() - 22.0).abs() < 1.0);
    let screenshot =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/425-link-media-dialog.png");
    std::fs::create_dir_all(screenshot.parent().unwrap()).unwrap();
    bench
        .harness_mut()
        .render()
        .unwrap()
        .save(&screenshot)
        .unwrap();
}

#[test]
fn the_footer_asks_for_a_folder_or_for_one_file() {
    let root = tempfile::tempdir().unwrap();
    let mut bench = mounted(root.path());
    open(&mut bench);
    bench.click("link_media.link");
    assert_eq!(
        bench.editor_mut().dialogs.link_media.take_request(),
        Some(Pick::Folder)
    );
    bench.click("link_media.row.1");
    assert_eq!(
        bench.editor_mut().dialogs.link_media.take_request(),
        Some(Pick::File(AssetId(1)))
    );
    // With the folder option off the button picks the first unlinked row's file.
    bench.click("link_media.folder");
    bench.click("link_media.link");
    assert_eq!(
        bench.editor_mut().dialogs.link_media.take_request(),
        Some(Pick::File(AssetId(1)))
    );
    bench.click("link_media.cancel");
    assert!(!bench.editor().dialogs.link_media.is_open());
}

#[test]
fn a_folder_pick_relinks_by_name_and_content_in_one_undo_step() {
    let root = tempfile::tempdir().unwrap();
    let mut bench = mounted(root.path());
    open(&mut bench);
    // A look-alike with the right name and the wrong bytes is not the file.
    let decoy = root.path().join("search").join("deep");
    std::fs::create_dir_all(&decoy).unwrap();
    std::fs::write(decoy.join("clip.mp4"), b"another take").unwrap();
    let editor = bench.editor_mut();
    editor.with_dialogs(|state, port| {
        picked(
            &mut state.link_media,
            port,
            Pick::Folder,
            root.path().join("search"),
        )
    });
    settle(&mut bench);
    assert_eq!(
        bench.editor().dialogs.link_media.notice,
        Some(crate::i18n::Text::LinkMediaNoMatch)
    );
    assert!(bench.editor().dialogs.link_media.is_open());
    let history = bench.editor().session.history_len();
    // The real file sits above the decoy, so the breadth-first search meets it first.
    std::fs::write(root.path().join("search").join("clip.mp4"), BYTES).unwrap();
    let editor = bench.editor_mut();
    editor.with_dialogs(|state, port| {
        picked(
            &mut state.link_media,
            port,
            Pick::Folder,
            root.path().join("search"),
        )
    });
    settle(&mut bench);
    let found = std::fs::canonicalize(root.path().join("search").join("clip.mp4")).unwrap();
    assert_eq!(recorded_path(&bench), found);
    assert_eq!(bench.editor().session.history_len(), history + 1);
    assert!(
        !bench.editor().dialogs.link_media.is_open(),
        "every row is linked, so the dialog closes"
    );
    bench.editor_mut().session.undo().unwrap();
    assert_ne!(recorded_path(&bench), found, "undo restores the old path");
}

#[test]
fn a_file_pick_links_only_the_row_it_was_asked_for() {
    let root = tempfile::tempdir().unwrap();
    let mut bench = mounted(root.path());
    open(&mut bench);
    let wrong = root.path().join("other.mp4");
    std::fs::write(&wrong, b"not it").unwrap();
    let editor = bench.editor_mut();
    editor.with_dialogs(|state, port| {
        picked(&mut state.link_media, port, Pick::File(AssetId(1)), wrong)
    });
    settle(&mut bench);
    assert!(bench.editor().dialogs.link_media.notice.is_some());
    let right = root.path().join("renamed.mp4");
    std::fs::write(&right, BYTES).unwrap();
    let editor = bench.editor_mut();
    editor.with_dialogs(|state, port| {
        picked(
            &mut state.link_media,
            port,
            Pick::File(AssetId(1)),
            right.clone(),
        )
    });
    settle(&mut bench);
    assert_eq!(recorded_path(&bench), std::fs::canonicalize(right).unwrap());
}

#[test]
fn long_paths_keep_their_head_and_tail() {
    let short = ellipsize(Path::new("/a/b.mp4"));
    assert_eq!(short, "/a/b.mp4");
    let long = ellipsize(Path::new(
        "/Users/someone/Desktop/a-folder/with/a/very/long/clip.mp4",
    ));
    assert_eq!(long.chars().count(), super::PATH_CHARS);
    assert!(long.contains('\u{2026}') && long.ends_with("clip.mp4"));
}

#[test]
fn the_folder_search_is_bounded_in_depth() {
    let root = tempfile::tempdir().unwrap();
    let mut deep = root.path().to_path_buf();
    for level in 0..=super::SEARCH_DEPTH + 1 {
        deep = deep.join(format!("d{level}"));
    }
    std::fs::create_dir_all(&deep).unwrap();
    std::fs::write(deep.join("far.mp4"), b"x").unwrap();
    std::fs::write(root.path().join("d0").join("near.mp4"), b"x").unwrap();
    assert!(find_by_name(root.path(), "near.mp4".as_ref()).is_some());
    assert!(find_by_name(root.path(), "far.mp4".as_ref()).is_none());
}

#[test]
fn the_match_runs_in_the_background_with_progress_and_cancel_links_nothing() {
    let root = tempfile::tempdir().unwrap();
    let mut bench = mounted(root.path());
    open(&mut bench);
    // A hasher that reports half way, then waits on a gate the test never opens, so the
    // only way out is the dialog's own Cancel.
    let started = Arc::new(AtomicBool::new(false));
    let gate = Arc::new(AtomicBool::new(false));
    let hasher: Hasher = {
        let (started, gate) = (started.clone(), gate.clone());
        Arc::new(
            move |_: &SourceAsset,
                  _: PathBuf,
                  cancelled: &AtomicBool,
                  progress: &mut dyn FnMut(u64, u64)|
                  -> Hashed {
                progress(4 << 20, 8 << 20);
                started.store(true, Ordering::SeqCst);
                while !gate.load(Ordering::SeqCst) {
                    if cancelled.load(Ordering::SeqCst) {
                        return Hashed::Cancelled;
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
                Hashed::Miss
            },
        )
    };
    bench.editor_mut().dialogs.link_media.set_hasher(hasher);
    let file = root.path().join("candidate.mp4");
    std::fs::write(&file, BYTES).unwrap();
    let before = recorded_path(&bench);
    let history = bench.editor().session.history_len();
    let editor = bench.editor_mut();
    editor.with_dialogs(|state, port| {
        picked(&mut state.link_media, port, Pick::File(AssetId(1)), file)
    });
    let waited = Instant::now();
    while !started.load(Ordering::SeqCst) {
        assert!(
            waited.elapsed() < Duration::from_secs(30),
            "worker never ran"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    // The dialog keeps painting while the worker holds the file: progress shows per file.
    for _ in 0..200 {
        bench.step();
        let editor = bench.editor();
        if editor
            .dialogs
            .link_media
            .view(editor.session.project())
            .progress
            .is_some()
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let view = {
        let editor = bench.editor();
        editor.dialogs.link_media.view(editor.session.project())
    };
    assert!(view.matching);
    assert_eq!(
        view.progress,
        Some(("clip.mp4".to_owned(), 4 << 20, 8 << 20))
    );
    assert!(
        bench
            .harness_mut()
            .query_all_by_label_contains("Checking clip.mp4")
            .next()
            .is_some(),
        "the dialog names the file being read"
    );
    assert_eq!(
        recorded_path(&bench),
        before,
        "nothing links before the match is done"
    );
    bench.click("link_media.cancel");
    settle(&mut bench);
    let editor = bench.editor();
    assert!(
        !editor
            .dialogs
            .link_media
            .view(editor.session.project())
            .matching
    );
    assert_eq!(
        editor.dialogs.link_media.notice,
        Some(crate::i18n::Text::LinkMediaCancelled)
    );
    assert!(
        editor.dialogs.link_media.is_open(),
        "Cancel stops the match, not the dialog"
    );
    assert_eq!(recorded_path(&bench), before);
    assert_eq!(bench.editor().session.history_len(), history);
}

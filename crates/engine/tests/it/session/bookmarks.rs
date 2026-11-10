#![allow(clippy::unwrap_used, reason = "validated test fixtures and assertions")]
use crate::support::session::project;
use engine::{Command, CreatedId, EditError, Session};
use project::{Color, Project, ProjectId, TrackId};
use serde_json::json;
use time::Tick;

#[test]
fn bookmark_move_is_one_undo_step_and_refuses_missing_or_negative_targets() {
    let mut s = Session::new(Project::empty(ProjectId(35), "Marks".into()).unwrap(), 20).unwrap();
    let CreatedId::Bookmark(bookmark) = *s
        .apply(Command::BookmarkAdd {
            start: Tick(10),
            duration: Tick(5),
            note: "Beat".into(),
            color: Color([1, 2, 3, 255]),
        })
        .unwrap()
        .created
        .first()
        .unwrap()
    else {
        panic!("bookmark id")
    };
    let before = s.project().clone();
    s.apply(Command::MoveBookmark {
        bookmark,
        start: Tick(40),
    })
    .unwrap();
    assert_eq!(s.project().data().scene().bookmarks[0].start, Tick(40));
    assert_eq!(s.history_len().0, 2);
    s.undo().unwrap();
    assert_eq!(s.project(), &before);
    s.redo().unwrap();
    assert_eq!(s.project().data().scene().bookmarks[0].start, Tick(40));
    assert!(matches!(
        s.apply(Command::MoveBookmark {
            bookmark,
            start: Tick(-1)
        }),
        Err(EditError::Invalid("bookmark start"))
    ));
    assert!(matches!(
        s.apply(Command::MoveBookmark {
            bookmark: project::BookmarkId(999),
            start: Tick(0)
        }),
        Err(EditError::Invalid("missing bookmark"))
    ));
    assert_eq!(s.project().data().scene().bookmarks[0].start, Tick(40));
}
#[test]
fn bookmark_toggle_snaps_adds_defaults_removes_and_undoes_every_step() {
    let mut data = project().into_data();
    data.canvas.fps = time::FrameRate::FPS_30;
    let mut s = Session::new(Project::new(data).unwrap(), 20).unwrap();
    let frame = s.project().data().canvas.fps.tick_of(1).unwrap();

    // One off-frame tick snaps to the nearest frame and adds a default bookmark.
    let at = Tick(frame.0 * 2 + 1);
    let outcome = s.apply(Command::ToggleBookmark { at }).unwrap();
    assert!(outcome.changed);
    let CreatedId::Bookmark(bookmark) = *outcome.created.first().unwrap() else {
        panic!("bookmark id")
    };
    let added = s.project().data().scene().bookmarks[0].clone();
    assert_eq!(added.id, bookmark);
    assert_eq!(added.start, Tick(frame.0 * 2));
    assert_eq!(added.duration, Tick::ZERO);
    assert_eq!(added.note.as_str(), "");
    assert_eq!(added.color, Color([0, 157, 255, 255]));
    assert_eq!(s.history_len(), (1, 0));

    // The same frame toggles it off; undo and redo restore each side exactly.
    let outcome = s.apply(Command::ToggleBookmark { at }).unwrap();
    assert!(outcome.changed);
    assert!(outcome.created.is_empty());
    assert!(s.project().data().scene().bookmarks.is_empty());
    assert_eq!(s.history_len(), (2, 0));
    s.undo().unwrap();
    assert_eq!(s.project().data().scene().bookmarks[0], added);
    s.redo().unwrap();
    assert!(s.project().data().scene().bookmarks.is_empty());

    // Off-frame inputs snap to the nearest frame; an exact tie snaps later.
    for (index, (input, expected)) in [
        (frame.0 * 3 + 1, frame.0 * 3),
        (frame.0 * 4 + frame.0 / 2 + 1, frame.0 * 5),
        (frame.0 * 6 - 1, frame.0 * 6),
        (frame.0 * 7 + frame.0 / 2, frame.0 * 8),
    ]
    .into_iter()
    .enumerate()
    {
        let outcome = s
            .apply(Command::ToggleBookmark { at: Tick(input) })
            .unwrap();
        let CreatedId::Bookmark(id) = *outcome.created.first().unwrap() else {
            panic!("bookmark id")
        };
        let bookmark = s
            .project()
            .data()
            .scene()
            .bookmarks
            .iter()
            .find(|bookmark| bookmark.id == id)
            .unwrap();
        assert_eq!(bookmark.start, Tick(expected), "row {index}");
        assert_eq!(s.project().data().scene().bookmarks.len(), index + 1);
    }
}
#[test]
fn bookmark_update_and_remove_are_one_undo_step_with_persistence() {
    let mut data = project().into_data();
    data.canvas.fps = time::FrameRate::FPS_30;
    let mut s = Session::new(Project::new(data).unwrap(), 20).unwrap();
    let CreatedId::Bookmark(bookmark) = *s
        .apply(Command::BookmarkAdd {
            start: Tick::ZERO,
            duration: Tick::ZERO,
            note: "Beat".into(),
            color: Color([1, 2, 3, 255]),
        })
        .unwrap()
        .created
        .first()
        .unwrap()
    else {
        panic!("bookmark id")
    };
    let mut previous = s.project().data().scene().bookmarks[0].clone();

    // Every field updates alone, and every update is one undo step.
    for (index, (command, note, color, duration)) in [
        (
            Command::UpdateBookmark {
                bookmark,
                note: Some("Intro".into()),
                color: None,
                duration: None,
            },
            "Intro",
            Color([1, 2, 3, 255]),
            Tick::ZERO,
        ),
        (
            Command::UpdateBookmark {
                bookmark,
                note: None,
                color: Some(Color([9, 8, 7, 255])),
                duration: None,
            },
            "Intro",
            Color([9, 8, 7, 255]),
            Tick::ZERO,
        ),
        (
            Command::UpdateBookmark {
                bookmark,
                note: None,
                color: None,
                duration: Some(Tick(500)),
            },
            "Intro",
            Color([9, 8, 7, 255]),
            Tick(100),
        ),
    ]
    .into_iter()
    .enumerate()
    {
        s.apply(command).unwrap();
        let current = s.project().data().scene().bookmarks[0].clone();
        assert_eq!(current.note.as_str(), note, "row {index}");
        assert_eq!(current.color, color, "row {index}");
        assert_eq!(current.duration, duration, "row {index}");
        assert_eq!(s.history_len(), (index + 2, 0), "row {index}");
        s.undo().unwrap();
        assert_eq!(s.project().data().scene().bookmarks[0], previous);
        s.redo().unwrap();
        assert_eq!(s.project().data().scene().bookmarks[0], current);
        previous = current;
    }
    let saved = project::encode(s.project(), false).unwrap();
    assert_eq!(
        project::decode(&saved)
            .unwrap()
            .project
            .data()
            .scene()
            .bookmarks,
        s.project().data().scene().bookmarks
    );

    // Removal is one undo step and restores the exact bookmark.
    let before_remove = s.project().clone();
    let history = s.history_len();
    s.apply(Command::RemoveBookmark { bookmark }).unwrap();
    assert!(s.project().data().scene().bookmarks.is_empty());
    assert_eq!(s.history_len(), (history.0 + 1, 0));
    s.undo().unwrap();
    assert_eq!(s.project(), &before_remove);
    s.redo().unwrap();
    assert!(s.project().data().scene().bookmarks.is_empty());

    // A later bookmark clamps to the remaining scene content.
    let CreatedId::Bookmark(tail) = *s
        .apply(Command::BookmarkAdd {
            start: Tick(60),
            duration: Tick::ZERO,
            note: "Tail".into(),
            color: Color([1, 2, 3, 255]),
        })
        .unwrap()
        .created
        .first()
        .unwrap()
    else {
        panic!("bookmark id")
    };
    s.apply(Command::UpdateBookmark {
        bookmark: tail,
        note: None,
        color: None,
        duration: Some(Tick(500)),
    })
    .unwrap();
    assert_eq!(s.project().data().scene().bookmarks[0].duration, Tick(40));
}
#[test]
fn bookmark_commands_refuse_hostile_inputs_without_history() {
    let mut data = project().into_data();
    data.canvas.fps = time::FrameRate::FPS_30;
    let mut s = Session::new(Project::new(data).unwrap(), 20).unwrap();
    let CreatedId::Bookmark(bookmark) = *s
        .apply(Command::BookmarkAdd {
            start: Tick::ZERO,
            duration: Tick::ZERO,
            note: "Beat".into(),
            color: Color([1, 2, 3, 255]),
        })
        .unwrap()
        .created
        .first()
        .unwrap()
    else {
        panic!("bookmark id")
    };
    let before = s.project().clone();
    let history = s.history_len();
    for (index, command) in [
        Command::ToggleBookmark { at: Tick(-1) },
        Command::UpdateBookmark {
            bookmark: project::BookmarkId(999),
            note: Some("x".into()),
            color: None,
            duration: None,
        },
        Command::UpdateBookmark {
            bookmark,
            note: None,
            color: None,
            duration: Some(Tick(-1)),
        },
        Command::UpdateBookmark {
            bookmark,
            note: None,
            color: None,
            duration: None,
        },
        Command::UpdateBookmark {
            bookmark,
            note: Some("Beat".into()),
            color: None,
            duration: None,
        },
        Command::RemoveBookmark {
            bookmark: project::BookmarkId(999),
        },
    ]
    .into_iter()
    .enumerate()
    {
        assert!(s.apply(command).is_err(), "row {index} accepted");
        assert_eq!(s.project(), &before, "row {index} changed the project");
        assert_eq!(s.history_len(), history, "row {index} added history");
    }
    for (index, color) in [
        [256.0, 0.0, 0.0, 255.0],
        [-1.0, 0.0, 0.0, 255.0],
        [0.0, 0.0, 0.0, 1.5],
    ]
    .into_iter()
    .enumerate()
    {
        assert!(
            crate::wire::run(
                &mut s,
                "bookmark.update",
                json!({"bookmark":bookmark,"color":color})
            )
            .is_err(),
            "row {index} accepted an out-of-range colour"
        );
        assert_eq!(s.project(), &before, "row {index} changed the project");
        assert_eq!(s.history_len(), history, "row {index} added history");
    }

    // The project owner's 1 MiB text limit remains the only note bound.
    let oversized = "n".repeat((1 << 20) + 1);
    assert!(matches!(
        s.apply(Command::UpdateBookmark {
            bookmark,
            note: Some(oversized.into()),
            color: None,
            duration: None,
        }),
        Err(EditError::Project(project::Error::TooLarge("text")))
    ));
    assert_eq!(s.project(), &before);
    assert_eq!(s.history_len(), history);

    // A duration beyond the scene end clamps once, then the same request is a no-op.
    s.apply(Command::UpdateBookmark {
        bookmark,
        note: None,
        color: None,
        duration: Some(Tick(500)),
    })
    .unwrap();
    assert_eq!(s.project().data().scene().bookmarks[0].duration, Tick(100));
    let changed = s.project().clone();
    let changed_history = s.history_len();
    assert!(matches!(
        s.apply(Command::UpdateBookmark {
            bookmark,
            note: None,
            color: None,
            duration: Some(Tick(500))
        }),
        Err(EditError::Invalid("bookmark update"))
    ));
    assert_eq!(s.project(), &changed);
    assert_eq!(s.history_len(), changed_history);

    // A locked track cannot hide a scene-owned bookmark: scenes have no lock state.
    let frame = s.project().data().canvas.fps.tick_of(1).unwrap();
    s.apply(Command::SetTrackLocked {
        track: TrackId(0),
        locked: true,
    })
    .unwrap();
    s.apply(Command::ToggleBookmark { at: frame }).unwrap();
    assert_eq!(s.project().data().scene().bookmarks.len(), 2);
}

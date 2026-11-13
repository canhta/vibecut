#![allow(clippy::unwrap_used, reason = "validated test fixtures and assertions")]
use crate::support::session::{new_item, project, range};
use engine::{Command, CreatedId, EditError, ItemParamPatch, Move, Placement, Session};
use media::{AssetId, ContentHash};
use project::{
    Color, ItemId, ItemParams, OverlayParams, Project, ProjectId, Track, TrackId, TrackKind,
};
use serde_json::json;
use std::sync::Arc;
use time::Tick;

#[test]
fn track_output_controls_enforce_kind_capabilities() {
    let mut s = Session::new(Project::empty(ProjectId(1), "Tracks".into()).unwrap(), 100).unwrap();
    for (kind, mute_allowed, hide_allowed) in [
        (TrackKind::Video, true, true),
        (TrackKind::Audio, true, false),
        (TrackKind::Text, false, true),
        (TrackKind::Sticker, false, true),
        (TrackKind::Effect, false, true),
        (TrackKind::Filter, false, true),
        (TrackKind::Adjustment, false, true),
    ] {
        let result = s
            .apply(Command::AddTrack {
                placement: engine::TrackPlacement::Auto,
                name: "Track".into(),
                kind,
            })
            .unwrap();
        let track = result
            .created
            .iter()
            .find_map(|id| match id {
                CreatedId::Track(track) => Some(*track),
                _ => None,
            })
            .unwrap();
        for (command, field, allowed) in [
            (
                Command::SetTrackMuted { track, muted: true },
                "muted",
                mute_allowed,
            ),
            (
                Command::SetTrackHidden {
                    track,
                    hidden: true,
                },
                "hidden",
                hide_allowed,
            ),
        ] {
            let before = s.project().clone();
            let result = s.apply(command);
            if allowed {
                result.unwrap();
                let value = s
                    .project()
                    .data()
                    .scene()
                    .timeline
                    .iter()
                    .find(|value| value.id == track)
                    .unwrap();
                assert!(match field {
                    "muted" => value.muted,
                    _ => value.hidden,
                });
            } else {
                assert!(result.is_err());
                assert!(Arc::ptr_eq(&before, s.project()));
            }
        }
    }
}
#[test]
fn track_rename_reorder_lock_and_add_persist_and_undo_exactly() {
    let mut s = Session::new(Project::empty(ProjectId(31), "Tracks".into()).unwrap(), 20).unwrap();
    let CreatedId::Track(a) = *s
        .apply(Command::AddTrack {
            name: "First".into(),
            kind: TrackKind::Video,
            placement: Default::default(),
        })
        .unwrap()
        .created
        .first()
        .unwrap()
    else {
        panic!("track id")
    };
    let outcome = s
        .apply(Command::AddTrack {
            name: "Second".into(),
            kind: TrackKind::Video,
            placement: Default::default(),
        })
        .unwrap();
    let CreatedId::Track(b) = outcome.created[0] else {
        panic!("track id")
    };
    assert_eq!(s.project().data().scene().timeline.overlays.len(), 2);
    assert_eq!(s.project().data().scene().timeline.overlays[0].id, b);

    let original_main = s.project().data().scene().timeline.main.clone();
    s.apply(Command::RenameTrack {
        track: original_main.id,
        name: "Renamed main".into(),
    })
    .unwrap();
    s.apply(Command::RenameTrack {
        track: a,
        name: "Rãnh đã đổi".into(),
    })
    .unwrap();
    s.apply(Command::ReorderTrack { track: b, index: 1 })
        .unwrap();
    s.apply(Command::SetTrackLocked {
        track: a,
        locked: true,
    })
    .unwrap();
    let changed = s.project().clone();
    let overlays = &changed.data().scene().timeline.overlays;
    assert_eq!(overlays[0].id, a);
    assert_eq!(overlays[1].id, b);
    assert_eq!(overlays[0].name.as_str(), "Rãnh đã đổi");
    assert!(overlays[0].locked);
    assert!(!overlays[1].locked);

    let bytes = project::encode(&changed, false).unwrap();
    let loaded = project::decode(&bytes).unwrap().project;
    assert_eq!(&loaded, changed.as_ref());
    assert_eq!(
        loaded.data().scene().timeline.main.name.as_str(),
        "Renamed main"
    );

    for _ in 0..4 {
        s.undo().unwrap();
    }
    assert_eq!(s.project().data().scene().timeline.main, original_main);
    let original = s.project().data().scene().timeline.overlays.clone();
    assert_eq!(original[0].id, b);
    assert_eq!(original[0].name.as_str(), "Second");
    assert_eq!(original[1].name.as_str(), "First");
    assert!(original.iter().all(|track| !track.locked));
    s.undo().unwrap();
    s.undo().unwrap();
    assert!(s.project().data().scene().timeline.overlays.is_empty());
    for _ in 0..6 {
        s.redo().unwrap();
    }
    assert_eq!(s.project(), &changed);
}
#[test]
fn locked_tracks_refuse_structural_item_edits_but_allow_metadata_and_unlock() {
    let mut s = Session::new(Project::empty(ProjectId(32), "Lock".into()).unwrap(), 20).unwrap();
    let outcome = s
        .apply(Command::AddTrack {
            name: "Locked".into(),
            kind: TrackKind::Video,
            placement: Default::default(),
        })
        .unwrap();
    let CreatedId::Track(track) = outcome.created[0] else {
        panic!("track id")
    };
    let item = new_item(0, 20);
    let outcome = s
        .apply(Command::InsertItem {
            placement: Placement::Track { track },
            item,
        })
        .unwrap();
    let CreatedId::Item(one) = outcome.created[0] else {
        panic!("item id")
    };
    let outcome = s
        .apply(Command::InsertItem {
            placement: Placement::Track { track: TrackId(0) },
            item: new_item(0, 20),
        })
        .unwrap();
    let CreatedId::Item(other) = outcome.created[0] else {
        panic!("item id")
    };
    s.apply(Command::SetTrackLocked {
        track,
        locked: true,
    })
    .unwrap();
    let before = s.project().clone();

    let edits = [
        Command::InsertItem {
            placement: Placement::Track { track },
            item: new_item(30, 10),
        },
        Command::MoveItem {
            item: one,
            track,
            start: Tick(5),
        },
        Command::MoveItem {
            item: one,
            track: TrackId(0),
            start: Tick(30),
        },
        Command::MoveItem {
            item: other,
            track,
            start: Tick(30),
        },
        Command::MoveItems {
            moves: vec![Move {
                item: one,
                track,
                start: Tick(5),
            }],
        },
        Command::SplitAt {
            item: one,
            at: Tick(5),
        },
        Command::TrimStart {
            item: one,
            at: Tick(2),
        },
        Command::TrimEnd {
            item: one,
            at: Tick(10),
        },
        Command::Delete { items: vec![one] },
        Command::RippleDelete { item: one },
        Command::RemoveTrack { track },
        Command::Batch {
            commands: vec![
                Command::RenameTrack {
                    track: TrackId(0),
                    name: "Must roll back".into(),
                },
                Command::Delete { items: vec![one] },
            ],
        },
        Command::SetItemParam {
            item: one,
            content: ItemParams::Overlay(OverlayParams {
                color: Color([1, 2, 3, 255]),
            }),
        },
    ];
    for command in edits {
        assert!(
            matches!(
                s.apply(command.clone()),
                Err(EditError::Locked(found)) if found == track
            ),
            "did not refuse a locked track: {command:?}"
        );
        assert_eq!(s.project(), &before, "changed a locked project");
    }

    // Selection, mute and the name remain editable while items are protected.
    s.set_selection([one].into_iter().collect()).unwrap();
    assert_eq!(s.selection().len(), 1);
    s.apply(Command::SetTrackMuted { track, muted: true })
        .unwrap();
    s.apply(Command::RenameTrack {
        track,
        name: "Still locked".into(),
    })
    .unwrap();
    assert!(s.project().data().scene().timeline.overlays[0].locked);
    s.apply(Command::SetTrackLocked {
        track,
        locked: false,
    })
    .unwrap();
    s.apply(Command::MoveItem {
        item: one,
        track,
        start: Tick(5),
    })
    .unwrap();
}
#[test]
fn audio_reorder_is_screen_order_and_main_cannot_move() {
    let mut s = Session::new(Project::empty(ProjectId(33), "Audio".into()).unwrap(), 20).unwrap();
    let CreatedId::Track(a) = *s
        .apply(Command::AddTrack {
            name: "A".into(),
            kind: TrackKind::Audio,
            placement: Default::default(),
        })
        .unwrap()
        .created
        .first()
        .unwrap()
    else {
        panic!("track id")
    };
    let CreatedId::Track(b) = *s
        .apply(Command::AddTrack {
            name: "B".into(),
            kind: TrackKind::Audio,
            placement: Default::default(),
        })
        .unwrap()
        .created
        .first()
        .unwrap()
    else {
        panic!("track id")
    };
    assert_eq!(s.project().data().scene().timeline.audio[0].id, b);
    s.apply(Command::ReorderTrack { track: a, index: 0 })
        .unwrap();
    assert_eq!(s.project().data().scene().timeline.audio[0].id, a);
    assert_eq!(s.project().data().scene().timeline.audio[1].id, b);
    let main = s.project().data().scene().timeline.main.id;
    assert!(matches!(
        s.apply(Command::ReorderTrack {
            track: main,
            index: 0
        }),
        Err(EditError::Invalid("main track cannot be moved"))
    ));
    assert!(matches!(
        s.apply(Command::ReorderTrack { track: a, index: 5 }),
        Err(EditError::Invalid("track index"))
    ));
    assert!(
        !Project::empty(ProjectId(34), "Fresh".into())
            .unwrap()
            .data()
            .scene()
            .timeline
            .main
            .locked
    );
}
#[test]
fn audio_controls_are_atomic_undoable_persisted_and_strict() {
    let mut data = project().into_data();
    // Reuse the existing synthetic media fixture without filesystem IO.
    let source = testkit::source::asset(7, std::env::temp_dir().join("audio-controls.wav"))
        .format("wav")
        .duration(Tick(1000))
        .content_hash(ContentHash::from_bytes(&[7]))
        .stream(
            testkit::source::audio()
                .codec("pcm_f32le")
                .time_base(media::TimeBase::new(1, 48000).unwrap())
                .duration(Tick(1000))
                .build(),
        )
        .build();
    data.assets.push(source);
    data.scene_mut().timeline.insert_track(Track {
        id: TrackId(8),
        name: "".into(),
        kind: TrackKind::Audio,
        hidden: false,
        muted: false,
        locked: false,
        items: vec![
            testkit::item::audio(ItemId(20))
                .asset(AssetId(7))
                .source(range(0, 1000))
                .build(),
        ],
    });
    let mut session = Session::new(Project::new(data).unwrap(), 100).unwrap();
    for command in [
        Command::SetItemField {
            item: ItemId(20),
            patch: ItemParamPatch::AudioGain(-6.01),
        },
        Command::SetItemField {
            item: ItemId(20),
            patch: ItemParamPatch::AudioMuted(true),
        },
        Command::SetMaster {
            volume: -1.25,
            muted: true,
        },
        Command::SetLoudness {
            scope: project::LoudnessScope::Clip,
            target_lufs: -16.0,
            true_peak_db: -2.0,
        },
        Command::SetLoudness {
            scope: project::LoudnessScope::Master,
            target_lufs: -14.0,
            true_peak_db: -1.0,
        },
        Command::ApplyLoudness {
            gains: project::LoudnessGains {
                key: [7; 32],
                clip: vec![(ItemId(20), 3.5)],
                master_db: -1.0,
            },
        },
        Command::ClearLoudness {
            scope: project::LoudnessScope::Clip,
        },
    ] {
        let before = session.project().clone();
        session.apply(command).unwrap();
        let after = session.project().clone();
        assert_ne!(before, after);
        session.undo().unwrap();
        assert_eq!(&before, session.project());
        session.redo().unwrap();
        assert_eq!(&after, session.project());
        let encoded = project::encode(session.project(), false).unwrap();
        assert_eq!(&project::decode(&encoded).unwrap().project, after.as_ref());
    }
    assert_eq!(session.project().data().settings.master_volume, -1.25);
    assert!(session.project().data().settings.master_muted);
    let loudness = &session.project().data().settings.loudness;
    assert!(loudness.clip.is_none() && loudness.master.is_some() && loudness.measured.is_none());
    // Invalid targets and gains refuse atomically.
    for command in [
        Command::SetLoudness {
            scope: project::LoudnessScope::Master,
            target_lufs: -4.0,
            true_peak_db: -1.0,
        },
        Command::SetLoudness {
            scope: project::LoudnessScope::Master,
            target_lufs: -51.0,
            true_peak_db: -1.0,
        },
        Command::SetLoudness {
            scope: project::LoudnessScope::Master,
            target_lufs: -14.0,
            true_peak_db: 0.5,
        },
        Command::SetLoudness {
            scope: project::LoudnessScope::Clip,
            target_lufs: -14.0,
            true_peak_db: -21.0,
        },
        Command::SetLoudness {
            scope: project::LoudnessScope::Clip,
            target_lufs: -14.001,
            true_peak_db: -1.0,
        },
        Command::ApplyLoudness {
            gains: project::LoudnessGains {
                key: [0; 32],
                clip: vec![],
                master_db: 61.0,
            },
        },
        Command::ApplyLoudness {
            gains: project::LoudnessGains {
                key: [0; 32],
                clip: vec![(ItemId(2), 0.0), (ItemId(1), 0.0)],
                master_db: 0.0,
            },
        },
    ] {
        let before = session.project().clone();
        assert!(session.apply(command).is_err());
        assert_eq!(session.project(), &before);
    }
    for value in [-60.01, 20.01, 0.001] {
        let before = session.project().clone();
        assert!(
            session
                .apply(Command::SetMaster {
                    volume: value,
                    muted: false
                })
                .is_err()
        );
        assert!(
            session
                .apply(Command::SetItemField {
                    item: ItemId(20),
                    patch: ItemParamPatch::AudioGain(value)
                })
                .is_err()
        );
        assert_eq!(session.project(), &before);
    }
    for value in [-60.0, 20.0] {
        session
            .apply(Command::SetMaster {
                volume: value,
                muted: false,
            })
            .unwrap();
    }
    assert!(
        crate::wire::run(
            &mut session,
            "timeline.set_item_param",
            json!({"item":20,"patch":{"field":"audio_gain","value":0.0}})
        )
        .is_err()
    );
    let mut encoded: serde_json::Value =
        serde_json::from_slice(&project::encode(session.project(), false).unwrap()).unwrap();
    encoded["schema_version"] = json!(10);
    assert!(matches!(
        project::decode(&serde_json::to_vec(&encoded).unwrap()),
        Err(project::Error::TooOld { .. })
    ));
}

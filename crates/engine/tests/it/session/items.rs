#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::unreachable
)]
use crate::support::session::{
    get, get_id, get_project, get_project_id, new_item, project, range, session,
};
use engine::{Command, CreatedId, EditError, Move, NewItem, Session, registry};
use media::{AssetId, ContentHash};
use project::{
    AnimationPath, BuiltinPath, Color, FlipAxis, Item, ItemId, ItemParams, Keyframe, Param,
    ParamValue, Project, ProjectId, Segment, Track, TrackId, TrackKind,
};
use proptest::prelude::*;
use std::sync::Arc;
use time::Tick;

#[test]
fn split_trim_ripple_and_history_use_exact_source_and_output_coordinates() {
    let mut s = session();
    let original = s.project().clone();
    let out = s
        .apply(Command::SplitAt {
            item: ItemId(10),
            at: Tick(20),
        })
        .unwrap();
    let CreatedId::Item(right) = *out.created.first().unwrap() else {
        panic!("item ID");
    };
    assert_eq!(get(&s, 10).source_range, range(100, 120));
    assert_eq!(get(&s, right.0).source_range, range(120, 140));
    assert_eq!(get(&s, right.0).start, Tick(20));
    s.apply(Command::TrimStart {
        item: right,
        at: Tick(25),
    })
    .unwrap();
    assert_eq!(get(&s, right.0).source_range, range(125, 140));
    s.apply(Command::TrimEnd {
        item: right,
        at: Tick(35),
    })
    .unwrap();
    assert_eq!(get(&s, right.0).source_range, range(125, 135));
    s.apply(Command::RippleDelete { item: ItemId(10) }).unwrap();
    assert_eq!(get(&s, right.0).start, Tick(5));
    assert_eq!(get(&s, 11).start, Tick(40));
    let final_state = s.project().clone();
    for _ in 0..4 {
        s.undo().unwrap();
    }
    assert!(Arc::ptr_eq(s.project(), &original));
    for _ in 0..4 {
        s.redo().unwrap();
    }
    assert!(Arc::ptr_eq(s.project(), &final_state));
}
#[test]
fn split_variants_refuse_boundaries_and_remove_the_requested_side() {
    let (mut s, _video, _audio, clip) = audible_session();
    let start = Tick(0);
    let end = Tick(100);
    let before = s.project().clone();
    for at in [start, end, Tick(101)] {
        for command in [
            Command::TimelineSplitLeft {
                items: vec![clip],
                at,
            },
            Command::TimelineSplitRight {
                items: vec![clip],
                at,
            },
        ] {
            let result = s.apply(command);
            let Err(error) = result else {
                panic!("at {at:?} unexpectedly succeeded");
            };
            assert!(
                matches!(
                    error,
                    EditError::Invalid(_) | EditError::Project(project::Error::Invalid(_))
                ),
                "at {at:?}: {error:?}"
            );
            assert!(Arc::ptr_eq(&before, s.project()));
        }
    }
    let out = s
        .apply(Command::TimelineSplitLeft {
            items: vec![clip],
            at: Tick(40),
        })
        .unwrap();
    let CreatedId::Item(kept) = out.created.first().copied().unwrap() else {
        panic!("right piece");
    };
    assert_eq!(get(&s, kept.0).start, Tick(40));
    assert_eq!(get(&s, kept.0).source_range, range(40, 100));
    assert!(
        !s.project()
            .data()
            .scene()
            .timeline
            .iter()
            .any(|track| track.items.iter().any(|item| item.id == clip))
    );
    s.undo().unwrap();
    assert_eq!(get(&s, clip.0).source_range, range(0, 100));
    s.redo().unwrap();
    assert_eq!(get(&s, kept.0).source_range, range(40, 100));

    let (mut s, _, _, clip) = audible_session();
    let out = s
        .apply(Command::TimelineSplitRight {
            items: vec![clip],
            at: Tick(40),
        })
        .unwrap();
    assert!(out.created.is_empty());
    assert_eq!(get(&s, clip.0).source_range, range(0, 40));
    assert_eq!(get(&s, clip.0).start, Tick::ZERO);
    assert_eq!(
        s.project()
            .data()
            .scene()
            .timeline
            .iter()
            .flat_map(|track| &track.items)
            .count(),
        2,
        "the right piece is removed and the audio item stays"
    );
    let after = s.project().clone();
    s.undo().unwrap();
    assert_eq!(get(&s, clip.0).source_range, range(0, 100));
    s.redo().unwrap();
    assert!(Arc::ptr_eq(&after, s.project()));
}
#[test]
fn duplicate_keeps_start_and_places_named_tracks_at_highest_insert() {
    let (mut s, video, _audio, clip) = audible_session();
    let audio_clip = s
        .project()
        .data()
        .scene()
        .timeline
        .audio
        .first()
        .unwrap()
        .items
        .first()
        .unwrap()
        .id;
    assert_eq!(video, s.project().data().scene().timeline.overlays[0].id);
    let original_start = get_id(&s, clip).start;
    let out = s
        .apply(Command::DuplicateItems {
            items: vec![clip, audio_clip],
        })
        .unwrap();
    let created: Vec<ItemId> = out
        .created
        .iter()
        .filter_map(|id| match id {
            CreatedId::Item(item) => Some(*item),
            _ => None,
        })
        .collect();
    assert_eq!(created.len(), 2);
    let timeline = &s.project().data().scene().timeline;
    assert_eq!(timeline.overlays[0].name.as_str(), "Video (copy)");
    assert_eq!(timeline.overlays[0].kind, TrackKind::Video);
    assert_eq!(timeline.audio[0].name.as_str(), "Audio (copy)");
    assert_eq!(timeline.audio[0].kind, TrackKind::Audio);
    for copy in &created {
        assert_eq!(get(&s, copy.0).start, original_start);
        assert_eq!(get(&s, copy.0).hidden, get_id(&s, clip).hidden);
    }
    let after = s.project().clone();
    s.undo().unwrap();
    assert_eq!(s.project().data().scene().timeline.overlays.len(), 1);
    assert_eq!(s.project().data().scene().timeline.audio.len(), 1);
    s.redo().unwrap();
    assert!(Arc::ptr_eq(&after, s.project()));
    assert!(matches!(
        s.apply(Command::DuplicateItems {
            items: vec![ItemId(999)],
        }),
        Err(EditError::NoItem(ItemId(999)))
    ));
}
#[test]
fn hide_and_mute_refuse_incompatible_kinds_and_survive_persistence() {
    let (mut s, video, _, clip) = audible_session();
    let audio_clip = s
        .project()
        .data()
        .scene()
        .timeline
        .audio
        .first()
        .unwrap()
        .items
        .first()
        .unwrap()
        .id;
    s.apply(Command::SetItemsHidden {
        items: vec![clip],
        hidden: true,
    })
    .unwrap();
    assert!(get_id(&s, clip).hidden);
    s.apply(Command::SetItemsMuted {
        items: vec![clip, audio_clip],
        muted: true,
    })
    .unwrap();
    for id in [clip, audio_clip] {
        assert!(match &get_id(&s, id).content {
            ItemParams::Video(params) => params.muted,
            ItemParams::Audio(params) => params.muted,
            _ => false,
        });
    }
    let before = s.project().clone();
    assert!(matches!(
        s.apply(Command::SetItemsHidden {
            items: vec![audio_clip],
            hidden: true,
        }),
        Err(EditError::Invalid("audio item has no video"))
    ));
    assert!(Arc::ptr_eq(&before, s.project()));
    s.undo().unwrap();
    s.undo().unwrap();
    assert!(!get_id(&s, clip).hidden);
    s.redo().unwrap();
    s.redo().unwrap();
    assert!(get_id(&s, clip).hidden);
    let bytes = project::encode(s.project(), false).unwrap();
    let loaded = project::decode(&bytes).unwrap();
    assert_eq!(loaded.project.data(), s.project().data());
    assert!(get_project_id(&loaded.project, clip).hidden);
    assert_eq!(video, s.project().data().scene().timeline.overlays[0].id);
}
#[test]
fn flip_sets_one_boolean_axis_per_list_and_refuses_unsupported_keyed_locked_and_missing() {
    let mut s = crate::support::control::seed();
    let path = AnimationPath::from(BuiltinPath::TransformFlipX);
    let flipped = |project: &Project, item: ItemId, axis: FlipAxis| {
        axis.value_at(&get_project_id(project, item).animations, Tick::ZERO)
            .unwrap()
    };
    s.apply(Command::SetItemsFlipped {
        items: vec![ItemId(1), ItemId(5)],
        axis: FlipAxis::Horizontal,
        flipped: true,
    })
    .unwrap();
    assert!(flipped(s.project(), ItemId(1), FlipAxis::Horizontal));
    assert!(flipped(s.project(), ItemId(5), FlipAxis::Horizontal));
    assert!(!flipped(s.project(), ItemId(1), FlipAxis::Vertical));
    let bytes = project::encode(s.project(), false).unwrap();
    s.apply(Command::SetItemsFlipped {
        items: vec![ItemId(1), ItemId(5)],
        axis: FlipAxis::Horizontal,
        flipped: true,
    })
    .unwrap();
    assert_eq!(project::encode(s.project(), false).unwrap(), bytes);
    s.undo().unwrap();
    assert!(!flipped(s.project(), ItemId(1), FlipAxis::Horizontal));
    assert!(!flipped(s.project(), ItemId(5), FlipAxis::Horizontal));
    s.redo().unwrap();
    assert!(flipped(s.project(), ItemId(1), FlipAxis::Horizontal));
    let loaded = project::decode(&project::encode(s.project(), false).unwrap()).unwrap();
    assert!(flipped(&loaded.project, ItemId(1), FlipAxis::Horizontal));

    let before = s.project().clone();
    for command in [
        Command::SetItemsFlipped {
            items: Vec::new(),
            axis: FlipAxis::Horizontal,
            flipped: true,
        },
        Command::SetItemsFlipped {
            items: vec![ItemId(1), ItemId(1)],
            axis: FlipAxis::Horizontal,
            flipped: true,
        },
    ] {
        assert!(matches!(s.apply(command), Err(EditError::Invalid(_))));
        assert_eq!(s.project(), &before);
    }
    assert!(matches!(
        s.apply(Command::SetItemsFlipped {
            items: vec![ItemId(999)],
            axis: FlipAxis::Horizontal,
            flipped: true,
        }),
        Err(EditError::NoItem(id)) if id == ItemId(999)
    ));
    assert_eq!(s.project(), &before);
    // A caption owns its cue layout, so the whole list refuses without a partial write.
    assert!(matches!(
        s.apply(Command::SetItemsFlipped {
            items: vec![ItemId(5), ItemId(4)],
            axis: FlipAxis::Vertical,
            flipped: true,
        }),
        Err(EditError::Invalid("flip on unsupported item"))
    ));
    assert_eq!(s.project(), &before);
    let (mut audio, _, _, clip) = audible_session();
    let audio_clip = audio
        .project()
        .data()
        .scene()
        .timeline
        .audio
        .first()
        .unwrap()
        .items
        .first()
        .unwrap()
        .id;
    assert!(matches!(
        audio.apply(Command::SetItemsFlipped {
            items: vec![clip, audio_clip],
            axis: FlipAxis::Horizontal,
            flipped: true,
        }),
        Err(EditError::Invalid("flip on unsupported item"))
    ));
    // A keyed axis keeps its curve; the static command refuses rather than erasing it.
    let mut param = Param::new(ParamValue::Bool(true)).unwrap();
    param
        .channel_mut(None)
        .unwrap()
        .put({
            let mut key = Keyframe::new(Tick::ZERO, ParamValue::Bool(false));
            key.segment = Segment::Step;
            key
        })
        .unwrap();
    s.apply(Command::SetAnimatedParam {
        item: ItemId(5),
        path: path.clone(),
        param,
    })
    .unwrap();
    assert!(matches!(
        s.apply(Command::SetItemsFlipped {
            items: vec![ItemId(5)],
            axis: FlipAxis::Horizontal,
            flipped: false,
        }),
        Err(EditError::Invalid("flip axis is keyed"))
    ));
    assert!(
        !get_id(&s, ItemId(5))
            .animations
            .get(&path)
            .unwrap()
            .channel(None)
            .unwrap()
            .keys()
            .is_empty()
    );
    s.apply(Command::SetTrackLocked {
        track: TrackId(1),
        locked: true,
    })
    .unwrap();
    assert!(matches!(
        s.apply(Command::SetItemsFlipped {
            items: vec![ItemId(1)],
            axis: FlipAxis::Vertical,
            flipped: true,
        }),
        Err(EditError::Locked(id)) if id == TrackId(1)
    ));
}
#[test]
fn extract_and_recover_source_audio_reuse_the_source_range() {
    let (mut s, _, _, clip) = audible_session();
    let source = get_id(&s, clip).source_range;
    let out = s
        .apply(Command::ExtractAudio { items: vec![clip] })
        .unwrap();
    let CreatedId::Item(audio) = out
        .created
        .iter()
        .find(|id| matches!(id, CreatedId::Item(_)))
        .copied()
        .unwrap()
    else {
        panic!("audio item");
    };
    assert!(!matches!(
        &get_id(&s, clip).content,
        ItemParams::Video(params) if params.source_audio_enabled
    ));
    assert_eq!(get(&s, audio.0).source_range, source);
    assert_eq!(get(&s, audio.0).start, get_id(&s, clip).start);
    assert!(matches!(get(&s, audio.0).content, ItemParams::Audio(_)));
    assert!(s.project().data().scene().timeline.audio.len() >= 2);
    let extracted = s.project().clone();
    assert!(matches!(
        s.apply(Command::ExtractAudio { items: vec![clip] }),
        Err(EditError::Invalid("source audio is extracted"))
    ));
    let bytes = project::encode(s.project(), false).unwrap();
    let loaded = project::decode(&bytes).unwrap();
    assert!(matches!(
        &get_project_id(&loaded.project, clip).content,
        ItemParams::Video(params) if !params.source_audio_enabled
    ));
    assert!(matches!(
        get_project_id(&loaded.project, audio).content,
        ItemParams::Audio(_)
    ));
    assert_eq!(get_project_id(&loaded.project, audio).source_range, source);
    s.undo().unwrap();
    assert!(matches!(
        &get_id(&s, clip).content,
        ItemParams::Video(params) if params.source_audio_enabled
    ));
    s.redo().unwrap();
    assert!(Arc::ptr_eq(&extracted, s.project()));
    s.apply(Command::RecoverAudio { items: vec![clip] })
        .unwrap();
    assert!(matches!(
        &get_id(&s, clip).content,
        ItemParams::Video(params) if params.source_audio_enabled
    ));
    assert!(matches!(
        s.apply(Command::RecoverAudio { items: vec![clip] }),
        Err(EditError::Invalid("source audio is enabled"))
    ));
    assert!(matches!(
        s.apply(Command::RecoverAudio { items: vec![audio] }),
        Err(EditError::Invalid("item is not video"))
    ));
    let bytes = project::encode(s.project(), false).unwrap();
    let loaded = project::decode(&bytes).unwrap();
    let recovered = &loaded.project;
    assert!(matches!(
        &get_project_id(recovered, clip).content,
        ItemParams::Video(params) if params.source_audio_enabled
    ));
    assert!(matches!(
        get_project(recovered, audio.0).content,
        ItemParams::Audio(_)
    ));
    assert_eq!(loaded.project.data(), s.project().data());
}
/// One unlocked video overlay with audible source audio and one audio clip.
fn audible_session() -> (Session, TrackId, TrackId, ItemId) {
    let mut s = Session::new(Project::empty(ProjectId(7), "Actions".into()).unwrap(), 50).unwrap();
    let asset = testkit::source::asset(1, std::env::temp_dir().join("vibecut-actions.mp4"))
        .format("mp4")
        .duration(Tick(100))
        .content_hash(ContentHash::from_bytes(b"actions"))
        .stream(
            testkit::source::video()
                .time_base(media::TimeBase::new(1, 30000).unwrap())
                .duration(Tick(100))
                .start(Tick::ZERO)
                .size(64, 64)
                .build(),
        )
        .stream(
            testkit::source::audio()
                .index(1)
                .time_base(media::TimeBase::new(1, 48000).unwrap())
                .duration(Tick(100))
                .start(Tick::ZERO)
                .build(),
        )
        .build();
    s.apply(Command::AddAsset { asset }).unwrap();
    let mut add = |name: &str, kind: TrackKind| {
        s.apply(Command::AddTrack {
            placement: engine::TrackPlacement::Auto,
            name: name.into(),
            kind,
        })
        .unwrap()
        .created
        .first()
        .and_then(|id| match id {
            CreatedId::Track(id) => Some(*id),
            _ => None,
        })
        .unwrap()
    };
    let video = add("Video", TrackKind::Video);
    let audio = add("Audio", TrackKind::Audio);
    let insert = |s: &mut Session, track, start: i64, content: ItemParams| {
        s.apply(Command::InsertItem {
            placement: engine::Placement::Track { track },
            item: NewItem {
                animations: Default::default(),
                source_asset: Some(AssetId(1)),
                source_range: range(start, start + 100),
                start: Tick(start),
                retime: Default::default(),
                content,
            },
        })
        .unwrap()
        .created
        .first()
        .and_then(|id| match id {
            CreatedId::Item(id) => Some(*id),
            _ => None,
        })
        .unwrap()
    };
    let clip = insert(
        &mut s,
        video,
        0,
        ItemParams::Video(project::VideoParams {
            fades: Default::default(),
            source_audio_enabled: true,
            volume: 0.0,
            muted: false,
            crop: None,
        }),
    );
    insert(
        &mut s,
        audio,
        0,
        ItemParams::Audio(project::AudioParams {
            fades: Default::default(),
            volume: 0.0,
            muted: false,
            source: Default::default(),
        }),
    );
    (s, video, audio, clip)
}
#[test]
fn simultaneous_moves_can_swap_but_never_leave_overlap_or_touch_locked_tracks() {
    let mut s = session();
    s.apply(Command::MoveItems {
        moves: vec![
            Move {
                item: ItemId(10),
                track: TrackId(1),
                start: Tick(60),
            },
            Move {
                item: ItemId(11),
                track: TrackId(1),
                start: Tick(0),
            },
        ],
    })
    .unwrap();
    assert_eq!(get(&s, 10).start, Tick(60));
    let before = s.project().clone();
    assert!(matches!(
        s.apply(Command::MoveItem {
            item: ItemId(10),
            track: TrackId(1),
            start: Tick(20)
        }),
        Err(EditError::Overlap(TrackId(1)))
    ));
    assert!(Arc::ptr_eq(&before, s.project()));
    s.apply(Command::SetTrackFlags {
        track: TrackId(2),
        muted: false,
        hidden: false,
        locked: true,
    })
    .unwrap();
    let before = s.project().clone();
    for command in [
        Command::MoveItem {
            item: ItemId(10),
            track: TrackId(2),
            start: Tick(0),
        },
        Command::InsertItem {
            placement: engine::Placement::Track { track: TrackId(2) },
            item: new_item(0, 10),
        },
        Command::Delete {
            items: vec![ItemId(10), ItemId(99)],
        },
        Command::MoveItems {
            moves: vec![
                Move {
                    item: ItemId(10),
                    track: TrackId(1),
                    start: Tick(60),
                },
                Move {
                    item: ItemId(10),
                    track: TrackId(1),
                    start: Tick(60),
                },
            ],
        },
    ] {
        assert!(s.apply(command).is_err());
        assert!(Arc::ptr_eq(&before, s.project()));
    }
}
#[test]
fn text_presets_save_apply_all_selected_items_undo_redo_and_reopen() {
    use engine::ItemParamPatch;
    use project::{SavedTextPreset, TextStyle};
    let mut data = project().into_data();
    let track = data.scene_mut().timeline.overlays.first_mut().unwrap();
    track.kind = TrackKind::Text;
    for (item, size) in track.items.iter_mut().zip([20.0, 30.0]) {
        item.content = ItemParams::Text(project::TextParams {
            content: format!("item {}", item.id.0).into(),
            font_size: size,
            color: Color([10, 20, 30, 255]),
            background: Some(Default::default()),
            ..Default::default()
        });
    }
    let mut s = Session::new(Project::new(data).unwrap(), 10).unwrap();
    let first = ItemParams::Text(project::TextParams {
        content: "item 10".into(),
        font_size: 20.0,
        color: Color([10, 20, 30, 255]),
        background: Some(Default::default()),
        ..Default::default()
    });
    let ItemParams::Text(first_params) = &first else {
        unreachable!()
    };
    let saved = s
        .apply(Command::SaveTextPreset {
            item: ItemId(10),
            name: "Look".into(),
        })
        .unwrap();
    let CreatedId::TextPreset(preset) = saved.created[0] else {
        panic!("text preset")
    };
    assert_eq!(
        s.project().data().text_presets,
        vec![SavedTextPreset {
            id: preset,
            name: "Look".into(),
            style: TextStyle::from_params(first_params),
        }]
    );
    assert_eq!(s.history_len(), (1, 0));
    // Each item keeps its own words while the saved style replaces every field.
    for (item, size, color) in [
        (ItemId(10), 72.0, [1, 2, 3, 255]),
        (ItemId(11), 8.0, [4, 5, 6, 255]),
    ] {
        s.apply(Command::SetItemField {
            item,
            patch: ItemParamPatch::TextSize(size),
        })
        .unwrap();
        s.apply(Command::SetItemField {
            item,
            patch: ItemParamPatch::TextColor(Color(color)),
        })
        .unwrap();
    }
    let before = s.project().clone();
    let outcome = s
        .apply(Command::ApplyTextPreset {
            items: vec![ItemId(10), ItemId(11)],
            preset,
        })
        .unwrap();
    assert!(outcome.changed);
    assert_eq!(s.history_len().0, 6);
    for id in [10, 11] {
        let ItemParams::Text(params) = &get(&s, id).content else {
            panic!("text")
        };
        assert_eq!(
            TextStyle::from_params(params),
            s.project().data().text_presets[0].style
        );
        assert_eq!(params.content.as_str(), format!("item {id}"));
    }
    s.undo().unwrap();
    assert_eq!(s.project(), &before);
    s.redo().unwrap();
    let applied = s.project().clone();
    let bytes = project::encode(&applied, false).unwrap();
    assert_eq!(project::decode(&bytes).unwrap().project, *applied);
    // Refusals are atomic and name reuse is refused.
    for command in [
        Command::ApplyTextPreset {
            items: Vec::new(),
            preset,
        },
        Command::ApplyTextPreset {
            items: vec![ItemId(10), ItemId(10)],
            preset,
        },
        Command::ApplyTextPreset {
            items: vec![ItemId(999)],
            preset,
        },
        Command::ApplyTextPreset {
            items: vec![ItemId(10)],
            preset: project::TextPresetId(999),
        },
        Command::ApplyTextPreset {
            items: vec![ItemId(1)],
            preset,
        },
        Command::SaveTextPreset {
            item: ItemId(10),
            name: "Look".into(),
        },
        Command::SaveTextPreset {
            item: ItemId(1),
            name: "Other".into(),
        },
    ] {
        let before = s.project().clone();
        let history = s.history_len();
        assert!(s.apply(command).is_err());
        assert_eq!(s.project(), &before);
        assert_eq!(s.history_len(), history);
    }
    let locked = s
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .find(|track| track.items.iter().any(|item| item.id == ItemId(10)))
        .unwrap()
        .id;
    s.apply(Command::SetTrackLocked {
        track: locked,
        locked: true,
    })
    .unwrap();
    assert!(matches!(
        s.apply(Command::ApplyTextPreset {
            items: vec![ItemId(10)],
            preset,
        }),
        Err(EditError::Locked(_))
    ));
    assert!(matches!(
        s.apply(Command::SaveTextPreset {
            item: ItemId(10),
            name: "Locked".into(),
        }),
        Err(EditError::Locked(_))
    ));
}

#[test]
#[allow(clippy::panic, reason = "validated fixture assertions")]
fn shared_fields_are_atomic_and_history_restores_transient_selection() {
    use engine::ItemParamPatch;
    use std::collections::BTreeSet;
    let mut data = project().into_data();
    let track = data.scene_mut().timeline.overlays.first_mut().unwrap();
    track.kind = TrackKind::Text;
    for (item, size) in track.items.iter_mut().zip([20.0, 30.0]) {
        item.content = ItemParams::Text(project::TextParams {
            content: format!("item {}", item.id.0).into(),
            font_size: size,
            color: Color([10, 20, 30, 255]),
            ..Default::default()
        });
    }
    let mut s = Session::new(Project::new(data).unwrap(), 2).unwrap();
    let before = s.project().clone();
    let bytes = serde_json::to_vec(s.project().data()).unwrap();
    let both: BTreeSet<_> = [ItemId(10), ItemId(11)].into_iter().collect();
    s.set_selection(both.clone()).unwrap();
    assert_eq!(serde_json::to_vec(s.project().data()).unwrap(), bytes);
    assert_eq!(s.history_len(), (0, 0));
    assert!(
        s.set_selection([ItemId(999)].into_iter().collect())
            .is_err()
    );
    assert_eq!(s.selection(), &both);
    s.apply(Command::SetItemParamMany {
        items: vec![ItemId(10), ItemId(11)],
        patch: ItemParamPatch::TextSize(72.0),
    })
    .unwrap();
    let after = s.project().clone();
    assert_eq!(s.history_len(), (1, 0));
    for (id, size) in [(10, 20.0), (11, 30.0)] {
        let ItemParams::Text(p) = &get(&s, id).content else {
            panic!("text fixture")
        };
        assert_eq!(p.font_size, 72.0);
        assert_eq!(p.content.as_str(), format!("item {id}"));
        assert_eq!(p.color, Color([10, 20, 30, 255]));
        let ItemParams::Text(p) = &before
            .data()
            .scene()
            .timeline
            .overlays
            .first()
            .unwrap()
            .items
            .iter()
            .find(|i| i.id == ItemId(id))
            .unwrap()
            .content
        else {
            panic!("text fixture")
        };
        assert_eq!(p.font_size, size);
    }
    s.set_selection(BTreeSet::new()).unwrap();
    s.undo().unwrap();
    assert_eq!(s.project(), &before);
    assert_eq!(s.selection(), &both);
    s.redo().unwrap();
    assert_eq!(s.project(), &after);
    assert_eq!(s.selection(), &both);
    for (items, patch) in [
        (vec![ItemId(10), ItemId(10)], ItemParamPatch::TextSize(10.0)),
        (
            vec![ItemId(10), ItemId(999)],
            ItemParamPatch::TextSize(10.0),
        ),
        (vec![], ItemParamPatch::TextSize(10.0)),
        (vec![ItemId(10)], ItemParamPatch::TextSize(f32::NAN)),
        (vec![ItemId(10)], ItemParamPatch::AudioGain(25.0)),
        (vec![ItemId(10)], ItemParamPatch::Opacity(f64::NAN)),
        (vec![ItemId(10)], ItemParamPatch::Opacity(f64::INFINITY)),
        (vec![ItemId(10)], ItemParamPatch::Opacity(f64::NEG_INFINITY)),
    ] {
        assert!(s.apply(Command::SetItemParamMany { items, patch }).is_err());
        assert_eq!(s.project(), &after);
        assert_eq!(s.history_len(), (1, 0));
    }
    // Incompatible kinds are intentionally skipped, including locked incompatible tracks.
    assert!(
        !s.apply(Command::SetItemParamMany {
            items: vec![ItemId(10), ItemId(11)],
            patch: ItemParamPatch::AudioGain(-6.0)
        })
        .unwrap()
        .changed
    );
    s.set_selection(both.clone()).unwrap();
    s.apply(Command::SetItemParam {
        item: ItemId(10),
        content: ItemParams::Text(project::TextParams {
            content: "independent".into(),
            font_size: 18.0,
            color: Color([1, 2, 3, 255]),
            ..Default::default()
        }),
    })
    .unwrap();
    let single = s.project().clone();
    s.set_selection(BTreeSet::new()).unwrap();
    s.undo().unwrap();
    assert_eq!(s.selection(), &both);
    s.redo().unwrap();
    assert_eq!(s.project(), &single);
    s.set_selection(both.clone()).unwrap();
    s.begin_gesture().unwrap();
    assert!(matches!(
        s.set_selection(BTreeSet::new()),
        Err(EditError::Session(engine::SessionError::TransactionActive))
    ));
    s.apply(Command::SetItemParamMany {
        items: vec![ItemId(10)],
        patch: ItemParamPatch::TextSize(44.0),
    })
    .unwrap();
    s.cancel_gesture().unwrap();
    assert_eq!(s.project(), &single);
    assert_eq!(s.selection(), &both);
    s.apply(Command::Delete {
        items: vec![ItemId(10)],
    })
    .unwrap();
    assert_eq!(s.selection(), &[ItemId(11)].into_iter().collect());
    assert_eq!(s.history_len(), (2, 0));
    s.undo().unwrap();
    assert_eq!(s.selection(), &both);
    s.undo().unwrap();
    assert!(!s.can_undo());
}
#[test]
#[allow(clippy::panic, reason = "validated fixture assertions")]
fn text_style_patches_update_one_field_and_validate() {
    use engine::ItemParamPatch;
    let mut data = project().into_data();
    let track = data.scene_mut().timeline.overlays.first_mut().unwrap();
    track.kind = TrackKind::Text;
    for item in &mut track.items {
        item.content = ItemParams::Text(project::TextParams {
            content: "Keep".into(),
            font_size: 24.0,
            color: Color([1, 2, 3, 255]),
            ..Default::default()
        });
    }
    let mut s = Session::new(Project::new(data).unwrap(), 20).unwrap();
    let item = ItemId(10);
    for patch in [
        ItemParamPatch::TextContent("Xin chào".into()),
        ItemParamPatch::TextFont("Noto Serif".into()),
        ItemParamPatch::TextWeight(700),
        ItemParamPatch::TextItalic(true),
        ItemParamPatch::TextLetterSpacing(-2.5),
        ItemParamPatch::TextLineHeight(60.0),
        ItemParamPatch::TextBackground(Some(project::TextBackground {
            color: Color([9, 8, 7, 128]),
            padding_x: 6.0,
            padding_y: 5.0,
            offset_x: -2.0,
            offset_y: 3.0,
            radius: 3.0,
        })),
        ItemParamPatch::TextBackgroundColor(Color([7, 6, 5, 128])),
        ItemParamPatch::TextBackgroundPaddingX(14.0),
        ItemParamPatch::TextBackgroundPaddingY(9.0),
        ItemParamPatch::TextBackgroundOffsetX(-30.5),
        ItemParamPatch::TextBackgroundOffsetY(12.0),
        ItemParamPatch::TextBackgroundCornerRadius(20.0),
        ItemParamPatch::TextAlign(project::TextAlign::Right),
        ItemParamPatch::TextDecoration(project::TextDecoration::LineThrough),
        ItemParamPatch::TextPositionX(25.0),
        ItemParamPatch::TextPositionY(30.0),
    ] {
        let before = s.project().clone();
        assert!(
            s.apply(Command::SetItemParamMany {
                items: vec![item],
                patch: patch.clone()
            })
            .unwrap()
            .changed
        );
        // Every key is one undo step and redo restores it exactly.
        let edited = s.project().clone();
        s.undo().unwrap();
        assert_eq!(s.project(), &before, "{patch:?} undo");
        s.redo().unwrap();
        assert_eq!(s.project(), &edited, "{patch:?} redo");
    }
    let ItemParams::Text(p) = &get(&s, item.0).content else {
        panic!("text fixture")
    };
    assert_eq!(p.content.as_str(), "Xin chào");
    assert_eq!(p.font.as_str(), "Noto Serif");
    assert_eq!(
        (p.weight, p.italic, p.letter_spacing, p.line_height),
        (700, true, -2.5, 60.0)
    );
    assert_eq!(p.font_size, 24.0);
    assert_eq!(p.color, Color([1, 2, 3, 255]));
    assert_eq!(p.background.as_ref().unwrap().color, Color([7, 6, 5, 128]));
    let background = p.background.as_ref().unwrap();
    assert_eq!(
        (
            background.padding_x,
            background.padding_y,
            background.radius
        ),
        (14.0, 9.0, 20.0)
    );
    assert_eq!((background.offset_x, background.offset_y), (-30.5, 12.0));
    assert_eq!(
        (p.align, p.decoration),
        (
            project::TextAlign::Right,
            project::TextDecoration::LineThrough
        )
    );
    // The patched style serialises with the stable field names the file format and CLI use.
    let json = serde_json::to_value(&get(&s, item.0).content).unwrap();
    assert_eq!(json["params"]["align"], "right");
    assert_eq!(json["params"]["decoration"], "line_through");
    assert_eq!(json["params"]["background"]["offset_x"], -30.5);
    assert_eq!(p.position, project::TextPosition { x: 25.0, y: 30.0 });

    let after = s.project().clone();
    for patch in [
        ItemParamPatch::TextFont("".into()),
        ItemParamPatch::TextWeight(0),
        ItemParamPatch::TextWeight(1000),
        ItemParamPatch::TextLetterSpacing(f32::NAN),
        ItemParamPatch::TextLetterSpacing(project::MAX_LETTER_SPACING + 1.0),
        ItemParamPatch::TextLineHeight(-1.0),
        ItemParamPatch::TextLineHeight(f32::INFINITY),
        ItemParamPatch::TextPositionX(101.0),
        ItemParamPatch::TextPositionY(f32::NAN),
        ItemParamPatch::TextBackgroundPaddingX(-1.0),
        ItemParamPatch::TextBackgroundPaddingY(129.0),
        ItemParamPatch::TextBackgroundCornerRadius(f32::NAN),
        ItemParamPatch::TextBackgroundOffsetX(project::TextBackground::MAX_OFFSET + 1.0),
        ItemParamPatch::TextBackgroundOffsetY(f32::INFINITY),
        ItemParamPatch::TextBackground(Some(project::TextBackground {
            color: Color([0; 4]),
            padding_x: 129.0,
            padding_y: 0.0,
            offset_x: 0.0,
            offset_y: 0.0,
            radius: 0.0,
        })),
    ] {
        assert!(
            s.apply(Command::SetItemParamMany {
                items: vec![item],
                patch
            })
            .is_err()
        );
        assert_eq!(s.project(), &after);
    }
}
#[test]
fn text_patch_expansion_is_bounded_and_atomic() {
    for selections in [vec![4], vec![5], vec![3, 3]] {
        let mut data = project().into_data();
        let track = data.scene_mut().timeline.overlays.first_mut().unwrap();
        track.kind = TrackKind::Text;
        let template = track.items.first().unwrap().clone();
        track.items = (0..6)
            .map(|index| Item {
                hidden: false,
                id: ItemId(100 + index),
                start: Tick(index as i64 * 1000),
                content: ItemParams::Text(project::TextParams::new("Keep")),
                ..template.clone()
            })
            .collect();
        let mut offset = 0;
        let mut commands: Vec<_> = selections
            .iter()
            .map(|count| {
                let items = track
                    .items
                    .iter()
                    .skip(offset)
                    .take(*count)
                    .map(|item| item.id)
                    .collect();
                offset += count;
                Command::SetItemParamMany {
                    items,
                    patch: engine::ItemParamPatch::TextContent("x".repeat(1 << 20).into()),
                }
            })
            .collect();
        let command = if commands.len() == 1 {
            commands.pop().unwrap()
        } else {
            Command::Batch {
                commands: vec![Command::Batch { commands }],
            }
        };
        let mut session = Session::new(Project::new(data).unwrap(), 20).unwrap();
        let before = session.project().clone();
        let result = session.apply(command);
        if selections == [4] {
            assert!(result.unwrap().changed);
            assert_eq!(session.history_len(), (1, 0));
            session.undo().unwrap();
        } else {
            assert!(matches!(
                result,
                Err(EditError::Control(engine::control::ControlError::Limit))
            ));
            assert_eq!(session.history_len(), (0, 0));
        }
        assert_eq!(session.project(), &before);
    }
}
#[test]
fn shared_field_refuses_locked_compatible_target_without_partial_edit() {
    use engine::ItemParamPatch;
    let mut data = project().into_data();
    let source = data.scene_mut().timeline.overlays.first_mut().unwrap();
    source.kind = TrackKind::Text;
    for item in &mut source.items {
        item.content = ItemParams::Text(project::TextParams {
            content: "Keep".into(),
            font_size: 24.0,
            color: Color([1, 2, 3, 255]),
            ..Default::default()
        });
    }
    let second = source.items.pop().unwrap();
    data.scene_mut().timeline.insert_track(Track {
        id: TrackId(99),
        name: "Locked".into(),
        kind: TrackKind::Text,
        muted: false,
        hidden: false,
        locked: true,
        items: vec![second],
    });
    let mut s = Session::new(Project::new(data).unwrap(), 2).unwrap();
    let before = s.project().clone();
    assert!(matches!(
        s.apply(Command::SetItemParamMany {
            items: vec![ItemId(10), ItemId(11)],
            patch: ItemParamPatch::TextSize(80.0)
        }),
        Err(EditError::Locked(TrackId(99)))
    ));
    assert_eq!(s.project(), &before);
    assert_eq!(s.history_len(), (0, 0));
    assert!(
        !s.apply(Command::SetItemParamMany {
            items: vec![ItemId(10), ItemId(11)],
            patch: ItemParamPatch::AudioGain(-6.0)
        })
        .unwrap()
        .changed
    );
}
#[test]
#[allow(clippy::indexing_slicing, clippy::panic)]
fn placement_is_atomic_and_respects_groups_locks_and_main_magnet() {
    use engine::{Placement, TrackPlacement};
    let mut s = Session::new(
        Project::empty(ProjectId(9), "Placement".into()).unwrap(),
        100,
    )
    .unwrap();
    for (at, duration) in [(50, 10), (100, 20), (5, 4)] {
        let outcome = s
            .apply(Command::InsertItem {
                placement: Placement::Auto,
                item: new_item(at, duration),
            })
            .unwrap();
        assert_eq!(outcome.placements.len(), 1);
        assert_eq!(outcome.placements[0].track, TrackId(0));
    }
    assert!(s.project().data().scene().timeline.overlays.is_empty());
    assert_eq!(
        s.project()
            .data()
            .scene()
            .timeline
            .main
            .items
            .iter()
            .map(|i| i.start.0)
            .collect::<Vec<_>>(),
        vec![0, 4, 14]
    );
    let magnetic = s.project().clone();
    s.apply(Command::SetTrackMagnet { enabled: false }).unwrap();
    // Occupied main range falls back to a secondary track; next free range reuses it.
    let a = s
        .apply(Command::InsertItem {
            placement: Placement::Auto,
            item: new_item(5, 10),
        })
        .unwrap();
    let overlay = a.placements[0].track;
    assert_ne!(overlay, TrackId(0));
    let b = s
        .apply(Command::InsertItem {
            placement: Placement::Auto,
            item: new_item(15, 10),
        })
        .unwrap();
    assert_eq!(b.placements[0].track, overlay);
    let before = s.project().clone();
    let history = s.history_len();
    for placement in [
        Placement::Below { track: TrackId(0) },
        Placement::Track { track: overlay },
        Placement::Above {
            track: TrackId(999),
        },
    ] {
        assert!(
            s.apply(Command::InsertItem {
                placement,
                item: new_item(5, 10)
            })
            .is_err()
        );
        assert_eq!(s.project(), &before);
        assert_eq!(s.history_len(), history);
    }
    assert!(
        s.apply(Command::InsertItem {
            placement: Placement::Above { track: overlay },
            item: new_item(i64::MAX, 10)
        })
        .is_err()
    );
    assert_eq!(s.project(), &before);
    assert_eq!(s.history_len(), history);
    let result = s
        .apply(Command::InsertItem {
            placement: Placement::Above { track: overlay },
            item: new_item(5, 10),
        })
        .unwrap();
    assert_eq!(result.created.len(), 2);
    let layout = s.project().clone();
    assert_eq!(
        layout.data().scene().timeline.overlays[0].id,
        result.placements[0].track
    );
    s.undo().unwrap();
    assert_eq!(s.project(), &before);
    s.redo().unwrap();
    assert_eq!(s.project(), &layout);
    assert_eq!(
        project::decode(&project::encode(&layout, false).unwrap())
            .unwrap()
            .project,
        *layout
    );
    // Explicit secondary creation supports both sides and refuses crossing groups.
    let lower = s
        .apply(Command::AddTrack {
            placement: TrackPlacement::Below { track: overlay },
            name: "Lower".into(),
            kind: TrackKind::Text,
        })
        .unwrap();
    assert_eq!(
        s.project()
            .data()
            .scene()
            .timeline
            .overlays
            .last()
            .unwrap()
            .id,
        match lower.created[0] {
            CreatedId::Track(id) => id,
            _ => panic!("track"),
        }
    );
    s.apply(Command::SetTrackFlags {
        track: TrackId(0),
        muted: false,
        hidden: false,
        locked: true,
    })
    .unwrap();
    let result = s
        .apply(Command::InsertItem {
            placement: Placement::Auto,
            item: new_item(100, 10),
        })
        .unwrap();
    assert_ne!(result.placements[0].track, TrackId(0));
    assert!(
        s.apply(Command::InsertItem {
            placement: Placement::Track { track: TrackId(0) },
            item: new_item(100, 10)
        })
        .is_err()
    );
    let ids: Vec<_> = s
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .map(|t| t.id)
        .collect();
    for track in ids {
        s.apply(Command::SetTrackFlags {
            track,
            muted: false,
            hidden: false,
            locked: true,
        })
        .unwrap();
    }
    assert!(
        registry()
            .iter()
            .find(|spec| spec.id == "timeline.insert")
            .unwrap()
            .enabled(&s)
    );
    let tracks = s.project().data().scene().timeline.track_count();
    s.apply(Command::InsertItem {
        placement: Placement::Auto,
        item: new_item(100, 10),
    })
    .unwrap();
    assert_eq!(
        s.project().data().scene().timeline.track_count(),
        tracks + 1
    );
    // Every placement entry point clamps empty main; occupied main keeps normal timing.
    for magnet in [false, true] {
        for (occupied, command, starts) in [
            (
                false,
                Command::InsertItem {
                    placement: Placement::Track { track: TrackId(0) },
                    item: new_item(100, 10),
                },
                vec![0],
            ),
            (
                false,
                Command::MoveItem {
                    item: ItemId(10),
                    track: TrackId(0),
                    start: Tick(100),
                },
                vec![0],
            ),
            (
                false,
                Command::MoveItems {
                    moves: vec![
                        Move {
                            item: ItemId(10),
                            track: TrackId(0),
                            start: Tick(100),
                        },
                        Move {
                            item: ItemId(11),
                            track: TrackId(0),
                            start: Tick(160),
                        },
                    ],
                },
                vec![0, 60],
            ),
            (
                true,
                Command::InsertItem {
                    placement: Placement::Track { track: TrackId(0) },
                    item: new_item(100, 10),
                },
                vec![0, 100],
            ),
            (
                true,
                Command::MoveItem {
                    item: ItemId(10),
                    track: TrackId(0),
                    start: Tick(100),
                },
                vec![0, 100],
            ),
            (
                true,
                Command::MoveItems {
                    moves: vec![
                        Move {
                            item: ItemId(10),
                            track: TrackId(0),
                            start: Tick(100),
                        },
                        Move {
                            item: ItemId(11),
                            track: TrackId(0),
                            start: Tick(160),
                        },
                    ],
                },
                vec![0, 100, 160],
            ),
        ] {
            let mut placed = session();
            placed
                .apply(Command::SetTrackMagnet { enabled: magnet })
                .unwrap();
            if occupied {
                placed
                    .apply(Command::InsertItem {
                        placement: Placement::Track { track: TrackId(0) },
                        item: new_item(0, 10),
                    })
                    .unwrap();
            }
            let before = placed.project().clone();
            let history = placed.history_len();
            placed.apply(command).unwrap();
            let after = placed.project().clone();
            assert_eq!(
                after
                    .data()
                    .scene()
                    .timeline
                    .main
                    .items
                    .iter()
                    .map(|item| item.start.0)
                    .collect::<Vec<_>>(),
                starts
            );
            assert_eq!(placed.history_len().0, history.0 + 1);
            placed.undo().unwrap();
            assert_eq!(placed.project(), &before);
            placed.redo().unwrap();
            assert_eq!(placed.project(), &after);
        }
    }
    assert!(magnetic.data().scene().timeline.magnet);
}
proptest! {
    #![proptest_config(ProptestConfig { cases: 64, rng_seed: proptest::test_runner::RngSeed::Fixed(14002021), ..ProptestConfig::default() })]

#[test]
    fn random_placements_preserve_layout_and_history(operations in prop::collection::vec((0u8..5,0i64..300,1i64..30,any::<bool>()),1..60)) {
        use engine::Placement;
        let mut s = Session::new(Project::empty(ProjectId(9), "Placement".into()).unwrap(), 1000).unwrap();
        let mut snapshots = vec![s.project().clone()];
        for (mode, at, duration, text) in operations {
            if s.apply(Command::SetTrackMagnet { enabled: mode % 2 == 0 }).unwrap().changed { snapshots.push(s.project().clone()); }
            let placement = match mode {
                0 => Placement::Above {track: TrackId(0)},
                1 => Placement::Below {track: TrackId(0)},
                2 => Placement::Track {track: TrackId(0)},
                _ => Placement::Auto,
            };
            let mut item = new_item(at,duration);
            if text { item.content = ItemParams::Text(project::TextParams {content:"Tiếng Việt".into(),font_size:24.0,color:Color([255;4]),..Default::default()}); }
            let before = s.project().clone();
            match s.apply(Command::InsertItem {placement,item}) {
                Ok(outcome) => {
                    prop_assert!(outcome.changed);
                    prop_assert_eq!(outcome.placements.len(),1);
                    snapshots.push(s.project().clone());
                }
                Err(_) => prop_assert_eq!(s.project(), &before),
            }
            let timeline = &s.project().data().scene().timeline;
            for track in timeline.iter() {
                let mut end=Tick::ZERO;
                for item in &track.items {
                    prop_assert!(item.start>=end);
                    prop_assert_eq!(item.content.kind(),track.kind);
                    end=item.end().unwrap();
                }
            }
            if let Some(first)=timeline.main.items.first(){ prop_assert_eq!(first.start,Tick::ZERO); }
        }
        let final_layout = s.project().clone();
        for before in snapshots.iter().rev().skip(1) { s.undo().unwrap(); prop_assert_eq!(s.project(),before); }
        for after in snapshots.iter().skip(1) { s.redo().unwrap(); prop_assert_eq!(s.project(),after); }
        prop_assert_eq!(s.project(),&final_layout);
        prop_assert_eq!(&project::decode(&project::encode(&final_layout, false).unwrap()).unwrap().project,final_layout.as_ref());
    }
}

#[test]
fn crop_position_channels_need_a_crop_stay_in_frame_and_follow_split_duplicate_and_scale() {
    let (mut s, _video, _audio, clip) = audible_session();
    let x = project::AnimationPath::new("crop.x").unwrap();
    let key =
        |at: i64, value: f64| project::Keyframe::new(Tick(at), project::ParamValue::Scalar(value));
    let add = |s: &mut Session, keys: Vec<project::Keyframe>| {
        let channel = project::Channel::new(
            project::ParamValue::Scalar(0.0),
            keys,
            project::Extrapolation::Hold,
            project::Extrapolation::Hold,
        )
        .unwrap();
        s.apply(Command::SetAnimatedParam {
            item: clip,
            path: x.clone(),
            param: project::Param::Value(channel),
        })
    };
    assert!(
        add(&mut s, vec![key(0, 0.0)]).is_err(),
        "keys need a static crop"
    );
    let crop = project::Crop {
        x: 0.0,
        y: 0.0,
        width: 0.5,
        height: 0.5,
    };
    s.apply(Command::SetItemField {
        item: clip,
        patch: engine::ItemParamPatch::Crop(Some(crop)),
    })
    .unwrap();
    assert!(add(&mut s, vec![key(0, 0.6)]).is_err(), "past 1 - width");
    assert!(add(&mut s, vec![key(0, -0.1)]).is_err(), "before the frame");
    add(&mut s, vec![key(0, 0.0), key(80, 0.5)]).unwrap();
    let at = |s: &Session, id: ItemId, t: i64| {
        let item = get_id(s, id);
        let crop = crop_of(item)
            .expect("the animated item is a video clip")
            .at(&item.animations, Tick(t))
            .unwrap();
        (crop.x, crop.y)
    };
    assert_eq!(at(&s, clip, 40), (0.25, 0.0), "linear between keys");
    let split = s
        .apply(Command::SplitAt {
            item: clip,
            at: Tick(40),
        })
        .unwrap();
    let CreatedId::Item(right) = *split.created.first().unwrap() else {
        panic!("item ID");
    };
    assert_eq!(at(&s, clip, 20), (0.125, 0.0));
    assert_eq!(
        at(&s, right, 20),
        (0.375, 0.0),
        "right piece keeps the curve"
    );
    let copy = s
        .apply(Command::DuplicateItems { items: vec![right] })
        .unwrap();
    let copy = copy
        .created
        .iter()
        .find_map(|id| match id {
            CreatedId::Item(item) => Some(*item),
            _ => None,
        })
        .unwrap();
    assert_eq!(get_id(&s, copy).animations, get_id(&s, right).animations);
    s.apply(Command::ScaleKeyframeTime {
        item: right,
        numerator: 2,
        denominator: 1,
    })
    .unwrap();
    assert_eq!(at(&s, right, 40), (0.375, 0.0), "keys stretch with time");
    let auto = project::AnimationPath::new("crop.auto").unwrap();
    let mut step = project::Keyframe::new(Tick(0), project::ParamValue::Bool(true));
    step.segment = project::Segment::Linear;
    assert!(
        project::Channel::new(
            project::ParamValue::Bool(false),
            vec![step],
            project::Extrapolation::Hold,
            project::Extrapolation::Hold,
        )
        .is_err(),
        "the auto flag only steps"
    );
    let held = project::Param::new(project::ParamValue::Bool(true)).unwrap();
    s.apply(Command::SetAnimatedParam {
        item: clip,
        path: auto,
        param: held,
    })
    .unwrap();
}

/// The crop carried by a video item, `None` for any other item kind.
fn crop_of(item: &Item) -> Option<project::Crop> {
    let ItemParams::Video(p) = &item.content else {
        return None;
    };
    p.crop
}

#[test]
#[allow(clippy::indexing_slicing)]
fn graphic_insertion_edits_refusals_masks_and_history_are_atomic() {
    use engine::{ItemParamPatch as P, Placement};
    for graphic in [
        project::GraphicParams::rectangle(),
        project::GraphicParams::ellipse(),
    ] {
        let mut s =
            Session::new(Project::empty(ProjectId(90), "Shapes".into()).unwrap(), 100).unwrap();
        let canvas = s.project().data().canvas.clone();
        let insert = |content| Command::InsertItem {
            placement: Placement::Auto,
            item: NewItem {
                retime: Default::default(),
                animations: Default::default(),
                source_asset: None,
                source_range: range(20, 120),
                start: Tick(50),
                content: ItemParams::Graphic(content),
            },
        };
        let initial = s.project().clone();
        let outcome = s.apply(insert(graphic.clone())).unwrap();
        let id = outcome.placements[0].item;
        assert_eq!(outcome.placements[0].track, TrackId(0));
        assert_eq!(s.project().data().canvas, canvas);
        assert_eq!(s.project().data().original_canvas_size, None);
        let path = project::AnimationPath::new("params.strokeWidth").unwrap();
        s.apply(Command::SetAnimatedParam {
            item: id,
            path: path.clone(),
            param: project::Param::new(project::ParamValue::Scalar(12.)).unwrap(),
        })
        .unwrap();
        s.apply(Command::SetItemField {
            item: id,
            patch: P::GraphicStrokeWidth(40.),
        })
        .unwrap();
        let current = || get_project(s.project(), id.0);
        assert_eq!(
            current().animations.value_at(&path, Tick(0)).unwrap(),
            Some(project::ParamValue::Scalar(12.))
        );
        s.apply(Command::AddMask {
            item: id,
            mask: project::Mask::new("shape-mask", project::MaskShape::Ellipse),
        })
        .unwrap();
        s.apply(Command::AddEffect {
            item: id,
            effect: project::effect_definition("blur")
                .unwrap()
                .instance("shape-effect"),
        })
        .unwrap();
        let before = s.project().clone();
        let history = s.history_len();
        for patch in [
            P::GraphicStrokeWidth(-1.),
            P::GraphicStrokeWidth(f64::NAN),
            P::GraphicCornerRadius(51.),
        ] {
            assert!(s.apply(Command::SetItemField { item: id, patch }).is_err());
            assert!(Arc::ptr_eq(&before, s.project()));
            assert_eq!(history, s.history_len());
        }
        if matches!(graphic, project::GraphicParams::Ellipse { .. }) {
            assert!(
                s.apply(Command::SetItemField {
                    item: id,
                    patch: P::GraphicCornerRadius(1.)
                })
                .is_err()
            );
        }
        assert!(
            s.apply(Command::SetSpeed {
                items: vec![id],
                speed: time::Speed::from_f64(2.).unwrap(),
                maintain_pitch: true
            })
            .is_err()
        );
        assert!(
            s.apply(Command::Batch {
                commands: vec![
                    Command::SetItemField {
                        item: id,
                        patch: P::GraphicFill(Color([1, 2, 3, 4]))
                    },
                    Command::SetItemField {
                        item: id,
                        patch: P::GraphicStrokeWidth(65.)
                    }
                ]
            })
            .is_err()
        );
        assert!(Arc::ptr_eq(&before, s.project()));
        s.apply(Command::SetTrackLocked {
            track: TrackId(0),
            locked: true,
        })
        .unwrap();
        let locked = s.project().clone();
        assert!(
            s.apply(Command::SetItemField {
                item: id,
                patch: P::GraphicFill(Color([0; 4]))
            })
            .is_err()
        );
        assert!(Arc::ptr_eq(&locked, s.project()));
        while s.history_len().0 > 0 {
            s.undo().unwrap();
        }
        assert_eq!(s.project(), &initial);
        s.apply(insert(graphic)).unwrap();
        assert_ne!(s.project().data().scene().timeline.main.items[0].id, id);
    }
}

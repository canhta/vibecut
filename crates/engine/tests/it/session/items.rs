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
use engine::{
    Command, CreatedId, EditError, ItemClipboard, ItemPasteSource, Move, NewItem, Session, registry,
};
use media::{AssetId, ContentHash};
use project::{
    AnimationPath, Animations, BuiltinPath, Color, DistributionAxis, FlipAxis, Item, ItemAlignment,
    ItemId, ItemParams, Keyframe, Param, ParamValue, Project, ProjectId, Segment, Track, TrackId,
    TrackKind, TransformField,
};
use proptest::prelude::*;
use serde_json::{Value, json};
use std::sync::Arc;
use time::{SourceRange, Tick};

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
fn split_all_cuts_every_unlocked_item_under_the_playhead_as_one_step() {
    let (mut s, video, _audio, clip) = audible_session();
    let before = s.project().clone();
    for at in [Tick::ZERO, Tick(100), Tick(101)] {
        let result = s.apply(Command::TimelineSplitAll { at });
        assert!(result.is_err(), "at {at:?} unexpectedly succeeded");
        assert!(Arc::ptr_eq(&before, s.project()));
    }
    s.apply(Command::SetTrackLocked {
        track: video,
        locked: true,
    })
    .unwrap();
    let locked = s.project().clone();
    let out = s.apply(Command::TimelineSplitAll { at: Tick(40) }).unwrap();
    assert_eq!(out.created.len(), 1, "only the unlocked audio item splits");
    assert_eq!(get(&s, clip.0).source_range, range(0, 100));
    s.undo().unwrap();
    assert!(Arc::ptr_eq(s.project(), &locked));
    s.undo().unwrap();
    assert!(Arc::ptr_eq(s.project(), &before));
    let out = s.apply(Command::TimelineSplitAll { at: Tick(40) }).unwrap();
    assert_eq!(
        out.created.len(),
        2,
        "every unlocked item splits in one step"
    );
    s.undo().unwrap();
    assert!(Arc::ptr_eq(s.project(), &before));
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
    let background_of = |project: &Project, id: ItemId| match &get_project_id(project, id).content {
        ItemParams::Video(params) => params.background,
        _ => panic!("video item expected"),
    };
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
    let before_background = s.project().clone();
    for (items, background) in [
        (vec![clip], project::ClipBackground::Blur { intensity: 501 }),
        (
            vec![audio_clip],
            project::ClipBackground::Solid {
                color: Color([9, 9, 9, 255]),
            },
        ),
    ] {
        assert!(
            s.apply(Command::SetItemsBackground { items, background })
                .is_err()
        );
        assert!(Arc::ptr_eq(&before_background, s.project()));
    }
    let blur = project::ClipBackground::Blur { intensity: 200 };
    s.apply(Command::SetItemsBackground {
        items: vec![clip],
        background: blur,
    })
    .unwrap();
    assert_eq!(background_of(s.project(), clip), blur);
    s.undo().unwrap();
    assert_eq!(
        background_of(s.project(), clip),
        project::ClipBackground::None
    );
    s.redo().unwrap();
    let bytes = project::encode(s.project(), false).unwrap();
    let loaded = project::decode(&bytes).unwrap();
    assert_eq!(loaded.project.data(), s.project().data());
    assert!(get_project_id(&loaded.project, clip).hidden);
    assert_eq!(background_of(&loaded.project, clip), blur);
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
            source_audio_enabled: true,
            ..project::VideoParams::plain()
        }),
    );
    insert(
        &mut s,
        audio,
        0,
        ItemParams::Audio(project::AudioParams::default()),
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
            stroke: Some(project::TextStroke {
                color: Color([255, 0, 0, 255]),
                width: 3.0,
            }),
            shadow: Some(project::TextShadow {
                color: Color([0, 0, 0, 128]),
                offset_x: 2.0,
                offset_y: 3.0,
                blur: 1.0,
            }),
            glow: true,
            background: Some(Default::default()),
            ..Default::default()
        });
    }
    let mut s = Session::new(Project::new(data).unwrap(), 10).unwrap();
    let first = ItemParams::Text(project::TextParams {
        content: "item 10".into(),
        font_size: 20.0,
        color: Color([10, 20, 30, 255]),
        stroke: Some(project::TextStroke {
            color: Color([255, 0, 0, 255]),
            width: 3.0,
        }),
        shadow: Some(project::TextShadow {
            color: Color([0, 0, 0, 128]),
            offset_x: 2.0,
            offset_y: 3.0,
            blur: 1.0,
        }),
        glow: true,
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
        ItemParamPatch::TextCase(project::TextCase::Upper),
        ItemParamPatch::TextStroke(Some(project::TextStroke {
            color: Color([9, 8, 7, 255]),
            width: 4.0,
        })),
        ItemParamPatch::TextShadow(Some(project::TextShadow {
            color: Color([3, 2, 1, 200]),
            offset_x: 5.0,
            offset_y: -6.0,
            blur: 2.0,
        })),
        ItemParamPatch::TextGlow(true),
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
    assert_eq!(p.case, project::TextCase::Upper);
    // The patched style serialises with the stable field names the file format and CLI use.
    let json = serde_json::to_value(&get(&s, item.0).content).unwrap();
    assert_eq!(json["params"]["align"], "right");
    assert_eq!(json["params"]["decoration"], "line_through");
    assert_eq!(json["params"]["case"], "upper");
    assert_eq!(json["params"]["background"]["offset_x"], -30.5);
    assert_eq!(p.stroke.as_ref().unwrap().width, 4.0);
    assert_eq!(p.shadow.as_ref().unwrap().blur, 2.0);
    assert!(p.glow);

    let after = s.project().clone();
    for patch in [
        ItemParamPatch::TextFont("".into()),
        ItemParamPatch::TextWeight(0),
        ItemParamPatch::TextWeight(1000),
        ItemParamPatch::TextLetterSpacing(f32::NAN),
        ItemParamPatch::TextLetterSpacing(project::MAX_LETTER_SPACING + 1.0),
        ItemParamPatch::TextLineHeight(-1.0),
        ItemParamPatch::TextLineHeight(f32::INFINITY),
        ItemParamPatch::TextStroke(Some(project::TextStroke {
            color: Color([0; 4]),
            width: project::TextStroke::MAX_WIDTH + 1.0,
        })),
        ItemParamPatch::TextShadow(Some(project::TextShadow {
            color: Color([0; 4]),
            offset_x: project::TextShadow::MAX_OFFSET + 1.0,
            offset_y: 0.0,
            blur: 0.0,
        })),
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

#[test]
fn reduce_noise_is_a_video_item_switch_with_one_undo_step() {
    let (mut s, _video, _audio, clip) = audible_session();
    let noise = |s: &Session| match &get_id(s, clip).content {
        ItemParams::Video(p) => p.reduce_noise,
        _ => panic!("video item"),
    };
    assert!(!noise(&s), "off at creation");
    s.apply(Command::SetItemField {
        item: clip,
        patch: engine::ItemParamPatch::ReduceNoise(true),
    })
    .unwrap();
    assert!(noise(&s));
    s.undo().unwrap();
    assert!(!noise(&s));
}

#[test]
fn stabilize_is_a_video_item_level_with_one_undo_step_and_reads_from_text() {
    use engine::ItemParamPatch as P;
    let (mut s, _video, _audio, clip) = audible_session();
    let level = |s: &Session| match &get_id(s, clip).content {
        ItemParams::Video(p) => p.stabilize,
        _ => panic!("video item"),
    };
    assert_eq!(level(&s), None, "off at creation");
    s.apply(Command::SetItemField {
        item: clip,
        patch: P::Stabilize(Some(project::StabilizeLevel::Stable)),
    })
    .unwrap();
    assert_eq!(level(&s), Some(project::StabilizeLevel::Stable));
    s.undo().unwrap();
    assert_eq!(level(&s), None);
    // The text an inspector or script holds: a level word switches on, `off` or `none` off.
    assert_eq!(
        P::parse("stabilize", "flexible").unwrap(),
        P::Stabilize(Some(project::StabilizeLevel::Flexible))
    );
    assert_eq!(P::parse("stabilize", "off").unwrap(), P::Stabilize(None));
    assert_eq!(P::parse("stabilize", "none").unwrap(), P::Stabilize(None));
    assert!(P::parse("stabilize", "extreme").is_err());
    // It addresses video items only.
    let patch = P::Stabilize(Some(project::StabilizeLevel::Normal));
    assert!(patch.compatible(&get_id(&s, clip).content));
    assert!(!patch.compatible(&ItemParams::Audio(project::AudioParams::default())));
}

#[test]
fn background_removal_is_a_video_item_switch_whose_feather_is_one_undo_step() {
    use engine::ItemParamPatch as P;
    use project::BackgroundRemoval as B;
    let (mut s, _video, _audio, clip) = audible_session();
    let removal = |s: &Session| match &get_id(s, clip).content {
        ItemParams::Video(p) => p.background_removal,
        _ => panic!("video item"),
    };
    assert_eq!(removal(&s), None, "off at creation");
    // The feather of a clip whose removal is off is refused and changes nothing.
    assert!(
        s.apply(Command::SetItemField {
            item: clip,
            patch: P::BackgroundRemovalFeather(20.0),
        })
        .is_err()
    );
    assert_eq!(removal(&s), None);
    s.apply(Command::SetItemField {
        item: clip,
        patch: P::BackgroundRemoval(Some(B::default())),
    })
    .unwrap();
    assert_eq!(removal(&s), Some(B { feather: 0.0 }));
    s.apply(Command::SetItemField {
        item: clip,
        patch: P::BackgroundRemovalFeather(35.0),
    })
    .unwrap();
    assert_eq!(removal(&s), Some(B { feather: 35.0 }));
    s.undo().unwrap();
    assert_eq!(removal(&s), Some(B { feather: 0.0 }));
    s.undo().unwrap();
    assert_eq!(removal(&s), None);
    // Out-of-range values never reach the project.
    assert!(
        s.apply(Command::SetItemField {
            item: clip,
            patch: P::BackgroundRemoval(Some(B { feather: 101.0 })),
        })
        .is_err()
    );
    // The text an inspector or script holds: `on`, a feather, `off` or `none`; typed feathers
    // beyond the slider are pulled in.
    assert_eq!(
        P::parse("background_removal", "on").unwrap(),
        P::BackgroundRemoval(Some(B::default()))
    );
    assert_eq!(
        P::parse("background_removal", "40").unwrap(),
        P::BackgroundRemoval(Some(B { feather: 40.0 }))
    );
    assert_eq!(
        P::parse("background_removal", "off").unwrap(),
        P::BackgroundRemoval(None)
    );
    assert_eq!(
        P::parse("background_removal", "none").unwrap(),
        P::BackgroundRemoval(None)
    );
    assert!(P::parse("background_removal", "150").is_err());
    assert_eq!(
        P::parse("background_removal_feather", "150").unwrap(),
        P::BackgroundRemovalFeather(100.0)
    );
    // It addresses video items only.
    let patch = P::BackgroundRemoval(Some(B::default()));
    assert!(patch.compatible(&get_id(&s, clip).content));
    assert!(!patch.compatible(&ItemParams::Audio(project::AudioParams::default())));
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
        project::GraphicParams::polygon(),
        project::GraphicParams::star(),
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
        if matches!(
            graphic,
            project::GraphicParams::Ellipse { .. } | project::GraphicParams::Star { .. }
        ) {
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
        // Radial parameters: own definition only, bounded sides/points 3..=12 and depth 1..=99.
        let polygon = matches!(graphic, project::GraphicParams::Polygon { .. });
        let star = matches!(graphic, project::GraphicParams::Star { .. });
        for (patch, own, valid) in [
            (P::GraphicSides(6.), polygon, true),
            (P::GraphicSides(2.), polygon, false),
            (P::GraphicSides(13.), polygon, false),
            (P::GraphicPoints(7.), star, true),
            (P::GraphicPoints(f64::NAN), star, false),
            (P::GraphicDepth(30.), star, true),
            (P::GraphicDepth(100.), star, false),
        ] {
            let before_radial = s.project().clone();
            let applied = s.apply(Command::SetItemField { item: id, patch });
            assert_eq!(applied.is_ok(), own && valid);
            if applied.is_err() {
                assert!(Arc::ptr_eq(&before_radial, s.project()));
            }
        }
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

/// One-item clipboard payload with a group-relative start and a zero placeholder id.
fn item_payload(item: &Item, kind: &str, track: u32) -> Value {
    let mut canonical = item.clone();
    canonical.id = ItemId(0);
    canonical.start = Tick::ZERO;
    json!({
        "version": 1,
        "span": canonical.duration().unwrap().0,
        "items": [{
            "trackKind": kind,
            "trackIndex": track,
            "item": canonical,
        }],
    })
}

fn payload_clipboard(payload: Value) -> ItemClipboard {
    serde_json::from_value(payload).unwrap()
}

fn paste(out: &engine::Outcome) -> Vec<ItemId> {
    out.created
        .iter()
        .filter_map(|id| match id {
            CreatedId::Item(item) => Some(*item),
            _ => None,
        })
        .collect()
}

#[test]
fn clipboard_items_round_trip_preserves_content_and_advances_the_playhead() {
    let mut s = session();
    let item = ItemId(10);
    s.apply(Command::SetAnimatedParam {
        item,
        path: BuiltinPath::Opacity.into(),
        param: Param::new(ParamValue::Scalar(0.4)).unwrap(),
    })
    .unwrap();
    s.apply(Command::AddKeyframe {
        item,
        path: BuiltinPath::Opacity.into(),
        component: None,
        key: Keyframe::new(Tick(2), ParamValue::Scalar(0.5)),
    })
    .unwrap();
    s.apply(Command::AddEffect {
        item,
        effect: project::effect_definition("blur")
            .unwrap()
            .instance("blur1"),
    })
    .unwrap();
    s.apply(Command::AddMask {
        item,
        mask: project::Mask::new("mask1", project::MaskShape::Rectangle),
    })
    .unwrap();
    s.apply(Command::TrimStart { item, at: Tick(5) }).unwrap();
    let source = get(&s, 10).clone();
    let span = source.duration().unwrap();
    let payload = item_payload(&source, "video", 0);
    let raw = json!({
        "command": "clipboard.paste",
        "params": {
            "at": 200,
            "source": {"kind": "payload", "payload": payload.clone()},
        },
    });
    let command: Command = serde_json::from_value(raw).unwrap();
    let round_trip = serde_json::to_value(&command).unwrap();
    assert_eq!(
        round_trip["params"]["source"]["payload"], payload,
        "trims, effects, masks, animations, retime and content survive serde"
    );
    let before = s.project().clone();
    let out = s.apply(command).unwrap();
    let created = paste(&out);
    assert_eq!(created.len(), 1);
    let pasted = get(&s, created[0].0);
    assert_eq!(pasted.start, Tick(200));
    assert_eq!(pasted.source_range, source.source_range);
    assert_eq!(pasted.retime, source.retime);
    assert_eq!(pasted.effects, source.effects);
    assert_eq!(pasted.masks, source.masks);
    assert_eq!(pasted.animations, source.animations);
    assert_eq!(pasted.content, source.content);
    assert_eq!(pasted.hidden, source.hidden);
    assert_eq!(s.metadata().view.playhead, Tick(200 + span.0));
    let after = s.project().clone();
    s.undo().unwrap();
    assert!(Arc::ptr_eq(&before, s.project()), "one paste is one undo");
    s.redo().unwrap();
    assert!(Arc::ptr_eq(&after, s.project()));
    let bytes = project::encode(s.project(), false).unwrap();
    let loaded = Session::new(project::decode(&bytes).unwrap().project, 10).unwrap();
    assert!(!loaded.clipboard_has_entry(), "clipboard is not persisted");

    // Cut copies the same payload and deletes the item in one undo step; a paste
    // after the cut restores it at the playhead.
    let mut s = session();
    let before = s.project().clone();
    let history = s.history_len();
    let out = s
        .apply(Command::CutSelection {
            target: engine::CutTarget::Items {
                items: vec![ItemId(10)],
            },
        })
        .unwrap();
    assert!(out.changed);
    assert!(
        s.project()
            .data()
            .scene()
            .timeline
            .iter()
            .all(|t| t.items.iter().all(|i| i.id != ItemId(10)))
    );
    assert_eq!(s.history_len().0, history.0 + 1);
    assert!(s.clipboard_has_entry());
    let pasted = s
        .apply(Command::PasteItems {
            at: Tick(200),
            source: ItemPasteSource::Stored,
        })
        .unwrap();
    assert_eq!(get(&s, paste(&pasted)[0].0).start, Tick(200));
    s.undo().unwrap();
    assert!(s.clipboard_has_entry(), "undo keeps the cut payload");
    s.undo().unwrap();
    assert!(Arc::ptr_eq(&before, s.project()));
    let bytes = project::encode(s.project(), false).unwrap();
    let loaded = Session::new(project::decode(&bytes).unwrap().project, 10).unwrap();
    assert_eq!(
        loaded.project(),
        s.project(),
        "cut then undo round-trips the project"
    );
    assert!(!loaded.clipboard_has_entry(), "clipboard is not persisted");

    // A locked track refuses the cut and leaves project, history and clipboard alone.
    let mut s = session();
    let track = s.project().data().scene().timeline.overlays[0].id;
    s.apply(Command::SetTrackLocked {
        track,
        locked: true,
    })
    .unwrap();
    let before = s.project().clone();
    let history = s.history_len();
    assert!(
        s.apply(Command::CutSelection {
            target: engine::CutTarget::Items {
                items: vec![ItemId(10)],
            },
        })
        .is_err()
    );
    assert!(Arc::ptr_eq(&before, s.project()));
    assert_eq!(s.history_len(), history);
    assert!(!s.clipboard_has_entry());

    // An empty selection is a no-op.
    let mut s = session();
    let before = s.project().clone();
    let history = s.history_len();
    assert!(
        !s.apply(Command::CutSelection {
            target: engine::CutTarget::Items { items: vec![] },
        })
        .unwrap()
        .changed
    );
    assert!(Arc::ptr_eq(&before, s.project()));
    assert_eq!(s.history_len(), history);
}

#[test]
fn clipboard_items_paste_offsets_overwrite_and_split_on_the_relative_track() {
    let mut s = session();
    let before = s.project().clone();
    assert!(
        !s.apply(Command::CopyItems {
            items: vec![ItemId(10), ItemId(11)],
        })
        .unwrap()
        .changed
    );
    let out = s
        .apply(Command::PasteItems {
            at: Tick(20),
            source: ItemPasteSource::Stored,
        })
        .unwrap();
    let created = paste(&out);
    assert_eq!(created.len(), 2);
    assert_eq!(get(&s, created[0].0).start, Tick(20));
    assert_eq!(get(&s, created[1].0).start, Tick(80));
    assert_eq!(get(&s, created[0].0).source_range, range(100, 140));
    assert_eq!(get(&s, created[1].0).source_range, range(200, 240));
    assert_eq!(get(&s, 10).start, Tick::ZERO);
    assert_eq!(get(&s, 10).source_range, range(100, 120));
    assert_eq!(get(&s, 11).start, Tick(60));
    assert_eq!(get(&s, 11).source_range, range(200, 220));
    assert_eq!(s.metadata().view.playhead, Tick(120));
    assert_eq!(s.history_len().0, 1);
    s.undo().unwrap();
    assert!(Arc::ptr_eq(&before, s.project()));

    // A pasted span strictly inside one item keeps both surviving heads.
    let mut s = session();
    let mut short = get(&s, 10).clone();
    short.source_range = range(100, 120);
    let payload = payload_clipboard(item_payload(&short, "video", 0));
    s.apply(Command::PasteItems {
        at: Tick(10),
        source: ItemPasteSource::Payload { payload },
    })
    .unwrap();
    let track = &s.project().data().scene().timeline.overlays[0];
    // Item 11 sits later on the same track and is untouched by the paste.
    assert_eq!(track.items.len(), 4);
    assert_eq!(get(&s, 10).source_range, range(100, 110));
    let right = track
        .items
        .iter()
        .find(|candidate| candidate.start == Tick(30))
        .unwrap();
    assert_eq!(right.source_range, range(130, 140));
    let pasted = track
        .items
        .iter()
        .find(|candidate| candidate.start == Tick(10))
        .unwrap();
    assert_eq!(pasted.source_range, range(100, 120));
}

#[test]
fn clipboard_items_empty_fallback_and_incompatible_kinds_refuse_atomically() {
    let mut s = Session::new(Project::empty(ProjectId(9), "Clip".into()).unwrap(), 10).unwrap();
    let before = s.project().clone();
    let revision = s.revision();
    let history = s.history_len();
    assert!(
        !s.apply(Command::PasteItems {
            at: Tick::ZERO,
            source: ItemPasteSource::Stored,
        })
        .unwrap()
        .changed
    );
    assert!(Arc::ptr_eq(&before, s.project()));
    assert_eq!(s.revision(), revision);
    assert_eq!(s.history_len(), history);

    // A missing relative index falls back to the last track of that kind.
    let source = get(&session(), 10).clone();
    let payload = payload_clipboard(item_payload(&source, "video", 5));
    let out = s
        .apply(Command::PasteItems {
            at: Tick::ZERO,
            source: ItemPasteSource::Payload { payload },
        })
        .unwrap();
    let created = paste(&out);
    assert_eq!(created.len(), 1);
    let main = &s.project().data().scene().timeline.main;
    assert_eq!(main.items.len(), 1);
    assert_eq!(main.items[0].id, created[0]);

    // A payload whose content kind does not match its stored track kind refuses.
    let empty = Project::empty(ProjectId(9), "Clip".into()).unwrap();
    let mut s = Session::new(empty, 10).unwrap();
    let before = s.project().clone();
    let payload = payload_clipboard(item_payload(&source, "audio", 0));
    assert!(matches!(
        s.apply(Command::PasteItems {
            at: Tick::ZERO,
            source: ItemPasteSource::Payload { payload },
        }),
        Err(EditError::Invalid("clipboard item kind"))
    ));
    assert!(Arc::ptr_eq(&before, s.project()));

    // An absent source asset refuses before any item or history change.
    let mut unlinked = source.clone();
    unlinked.source_asset = Some(AssetId(99));
    let payload = payload_clipboard(item_payload(&unlinked, "video", 0));
    assert!(matches!(
        s.apply(Command::PasteItems {
            at: Tick::ZERO,
            source: ItemPasteSource::Payload { payload },
        }),
        Err(EditError::NoAsset(AssetId(99)))
    ));
    assert!(Arc::ptr_eq(&before, s.project()));

    // A locked relative target refuses the stored paste atomically.
    let mut s = session();
    s.apply(Command::CopyItems {
        items: vec![ItemId(10)],
    })
    .unwrap();
    s.apply(Command::SetTrackLocked {
        track: TrackId(1),
        locked: true,
    })
    .unwrap();
    let before = s.project().clone();
    let history = s.history_len();
    assert!(matches!(
        s.apply(Command::PasteItems {
            at: Tick(200),
            source: ItemPasteSource::Stored,
        }),
        Err(EditError::Locked(TrackId(1)))
    ));
    assert!(Arc::ptr_eq(&before, s.project()));
    assert_eq!(s.history_len(), history);
}

fn arrange_overlay(id: u64, x: f64, y: f64) -> Item {
    let mut animations = Animations::default();
    for (path, value) in [
        (BuiltinPath::TransformPositionX, x),
        (BuiltinPath::TransformPositionY, y),
        (BuiltinPath::TransformScaleX, 0.1),
        (BuiltinPath::TransformScaleY, 0.1),
    ] {
        animations
            .set(
                AnimationPath::from(path),
                Param::new(ParamValue::Scalar(value)).unwrap(),
            )
            .unwrap();
    }
    testkit::item::item(
        ItemId(id),
        ItemParams::Overlay(project::OverlayParams {
            color: Color([10, 20, 30, 255]),
        }),
    )
    .source(SourceRange::new(Tick::ZERO, Tick(100)).unwrap())
    .animations(animations)
    .build()
}

fn arrange_session(items: &[(u64, f64, f64)]) -> Session {
    let mut data = crate::support::session::project().into_data();
    data.canvas.width = 200;
    data.canvas.height = 100;
    let timeline = &mut data.scene_mut().timeline;
    timeline.overlays[0].items.clear();
    timeline.overlays[1].items.clear();
    timeline.main.items.clear();
    for (index, (id, x, y)) in items.iter().enumerate() {
        let item = arrange_overlay(*id, *x, *y);
        match index {
            0 => timeline.overlays[0].items.push(item),
            1 => timeline.overlays[1].items.push(item),
            _ => timeline.main.items.push(item),
        }
    }
    Session::new(Project::new(data).unwrap(), 100).unwrap()
}

fn position(s: &Session, id: ItemId, field: TransformField) -> f64 {
    field
        .value_at(&get_id(s, id).animations, Tick::ZERO)
        .unwrap()
}

fn close(actual: f64, expected: f64) -> bool {
    (actual - expected).abs() < 1e-6
}

#[test]
fn align_moves_each_transformed_box_to_the_canvas_and_preserves_its_other_axis() {
    let mut s = arrange_session(&[(21, -60.0, 0.0)]);
    // Scaled 0.1 around the canvas centre: the box is x 30..=50, y 45..=55.
    for (alignment, x, y) in [
        (ItemAlignment::Left, -90.0, 0.0),
        (ItemAlignment::HorizontalCenter, 0.0, 0.0),
        (ItemAlignment::Right, 90.0, 0.0),
        (ItemAlignment::Top, 0.0, -45.0),
        (ItemAlignment::VerticalCenter, 0.0, 0.0),
        (ItemAlignment::Bottom, 0.0, 45.0),
    ] {
        s.apply(Command::AlignItems {
            items: vec![ItemId(21)],
            alignment,
            at: Tick::ZERO,
        })
        .unwrap();
        assert!(
            close(position(&s, ItemId(21), TransformField::PositionX), x)
                && close(position(&s, ItemId(21), TransformField::PositionY), y),
            "{alignment:?}"
        );
        // Reset only the axis this case moved, so the next case starts at the
        // documented saved position.
        for (field, value) in [
            (TransformField::PositionX, 0.0),
            (TransformField::PositionY, 0.0),
        ] {
            s.apply(Command::SetAnimatedParam {
                item: ItemId(21),
                path: AnimationPath::from(field.builtin()),
                param: Param::new(ParamValue::Scalar(value)).unwrap(),
            })
            .unwrap();
        }
    }

    // A quarter turn swaps the box axes, so left alignment uses the rotated
    // envelope rather than the saved destination.
    s.apply(Command::SetAnimatedParam {
        item: ItemId(21),
        path: AnimationPath::from(BuiltinPath::TransformRotate),
        param: Param::new(ParamValue::Scalar(90.0)).unwrap(),
    })
    .unwrap();
    let history = s.history_len().0;
    s.apply(Command::AlignItems {
        items: vec![ItemId(21)],
        alignment: ItemAlignment::Left,
        at: Tick::ZERO,
    })
    .unwrap();
    assert!(close(
        position(&s, ItemId(21), TransformField::PositionX),
        -95.0
    ));
    assert_eq!(s.history_len().0, history + 1);
    let bytes = project::encode(s.project(), false).unwrap();
    let loaded = project::decode(&bytes).unwrap();
    assert!(close(
        TransformField::PositionX
            .value_at(
                &get_project_id(&loaded.project, ItemId(21)).animations,
                Tick::ZERO
            )
            .unwrap(),
        -95.0
    ));
    // A second equal request is an exact no-op and adds no history.
    let history = s.history_len().0;
    s.apply(Command::AlignItems {
        items: vec![ItemId(21)],
        alignment: ItemAlignment::Left,
        at: Tick::ZERO,
    })
    .unwrap();
    assert_eq!(s.history_len().0, history);
    s.undo().unwrap();
    assert!(close(
        position(&s, ItemId(21), TransformField::PositionX),
        0.0
    ));
}

#[test]
fn distribute_pins_outer_edges_and_spaces_three_boxes_with_equal_gaps() {
    let mut s = arrange_session(&[(21, -60.0, 0.0), (22, 0.0, 0.0), (23, 100.0, 0.0)]);
    // Boxes are 20x10: leading edges 30, 90 and 190; span 180, total 60,
    // so each of the two gaps is 60 and the middle box starts at 110.
    let history = s.history_len().0;
    s.apply(Command::DistributeItems {
        items: vec![ItemId(23), ItemId(21), ItemId(22)],
        axis: DistributionAxis::Horizontal,
        at: Tick::ZERO,
    })
    .unwrap();
    for (id, x) in [(21, -60.0), (22, 20.0), (23, 100.0)] {
        assert!(close(
            position(&s, ItemId(id), TransformField::PositionX),
            x
        ));
    }
    assert_eq!(s.history_len().0, history + 1);
    s.undo().unwrap();
    assert!(close(
        position(&s, ItemId(22), TransformField::PositionX),
        0.0
    ));

    let mut s = arrange_session(&[(21, 0.0, -30.0), (22, 0.0, 0.0), (23, 0.0, 60.0)]);
    s.apply(Command::DistributeItems {
        items: vec![ItemId(21), ItemId(22), ItemId(23)],
        axis: DistributionAxis::Vertical,
        at: Tick::ZERO,
    })
    .unwrap();
    for (id, y) in [(21, -30.0), (22, 15.0), (23, 60.0)] {
        assert!(close(
            position(&s, ItemId(id), TransformField::PositionY),
            y
        ));
    }
}

#[test]
fn align_translates_curve_values_and_refuses_whole_lists() {
    let mut s = arrange_session(&[(21, 0.0, 0.0)]);
    let mut param = Param::new(ParamValue::Scalar(0.0)).unwrap();
    param
        .channel_mut(None)
        .unwrap()
        .put(Keyframe::new(Tick::ZERO, ParamValue::Scalar(0.0)))
        .unwrap();
    param
        .channel_mut(None)
        .unwrap()
        .put(Keyframe::new(Tick(50), ParamValue::Scalar(10.0)))
        .unwrap();
    s.apply(Command::SetAnimatedParam {
        item: ItemId(21),
        path: AnimationPath::from(BuiltinPath::TransformPositionX),
        param,
    })
    .unwrap();
    s.apply(Command::AlignItems {
        items: vec![ItemId(21)],
        alignment: ItemAlignment::Left,
        at: Tick::ZERO,
    })
    .unwrap();
    let channel = get_id(&s, ItemId(21))
        .animations
        .get(&AnimationPath::from(BuiltinPath::TransformPositionX))
        .unwrap()
        .channel(None)
        .unwrap();
    let scalar = |value: &ParamValue| match value {
        ParamValue::Scalar(value) => *value,
        _ => panic!("scalar transform"),
    };
    assert!(close(scalar(channel.base()), -90.0));
    let values = channel
        .keys()
        .iter()
        .map(|key| scalar(&key.value))
        .collect::<Vec<_>>();
    assert!(
        close(values[0], -90.0) && close(values[1], -80.0),
        "{values:?}"
    );
    assert_eq!(channel.keys()[1].time, Tick(50));
    assert!(!channel.keys().is_empty());

    let before = s.project().clone();
    for command in [
        Command::DistributeItems {
            items: vec![ItemId(21)],
            axis: DistributionAxis::Horizontal,
            at: Tick::ZERO,
        },
        Command::AlignItems {
            items: Vec::new(),
            alignment: ItemAlignment::Left,
            at: Tick::ZERO,
        },
        Command::AlignItems {
            items: vec![ItemId(21), ItemId(21)],
            alignment: ItemAlignment::Left,
            at: Tick::ZERO,
        },
    ] {
        assert!(matches!(s.apply(command), Err(EditError::Invalid(_))));
        assert_eq!(s.project(), &before);
    }
    assert!(matches!(
        s.apply(Command::AlignItems {
            items: vec![ItemId(21), ItemId(999)],
            alignment: ItemAlignment::Left,
            at: Tick::ZERO,
        }),
        Err(EditError::NoItem(id)) if id == ItemId(999)
    ));
    assert_eq!(s.project(), &before);
    // With only two boxes the pinned outer edges leave no even middle gap.
    let mut two = arrange_session(&[(21, -60.0, 0.0), (22, 100.0, 0.0)]);
    let before = two.project().clone();
    assert!(matches!(
        two.apply(Command::DistributeItems {
            items: vec![ItemId(21), ItemId(22)],
            axis: DistributionAxis::Horizontal,
            at: Tick::ZERO,
        }),
        Err(EditError::Invalid("distribute needs three items"))
    ));
    assert_eq!(two.project(), &before);
    // A locked track refuses the whole list.
    two.apply(Command::SetTrackLocked {
        track: two.project().data().scene().timeline.overlays[0].id,
        locked: true,
    })
    .unwrap();
    let before = two.project().clone();
    assert!(matches!(
        two.apply(Command::AlignItems {
            items: vec![ItemId(21), ItemId(22)],
            alignment: ItemAlignment::Left,
            at: Tick::ZERO,
        }),
        Err(EditError::Locked(_))
    ));
    assert_eq!(two.project(), &before);
}

/// The parameter table is the one declaration: every row names its kinds and bounds, is
/// found by its wire name, and is the field the patch serializes under.
#[test]
fn item_param_table_rows_declare_kinds_and_bounds_and_parse_by_wire_name() {
    use engine::{ITEM_PARAMS, ItemParamPatch, ParamBounds, ParamParseError, item_param};
    // One text per payload type: numbers, switches, words, colours and cleared shapes.
    const SAMPLES: [&str; 7] = [
        "1",
        "true",
        "multiply",
        "fill",
        "center",
        "#FF0000FF",
        "none",
    ];
    let mut names = std::collections::BTreeSet::new();
    for row in ITEM_PARAMS {
        assert!(names.insert(row.name), "{} is declared twice", row.name);
        assert!(!row.targets.is_empty(), "{} names no kind", row.name);
        assert_eq!(item_param(row.name), Some(row));
        if let ParamBounds::Range { min, max, .. } = row.bounds {
            assert!(min < max, "{} has an empty range", row.name);
            for edge in [min, max] {
                assert!(
                    ItemParamPatch::parse(row.name, &edge.to_string()).is_ok(),
                    "{} refuses its own bound {edge}",
                    row.name
                );
            }
        }
        // A crop is placed by dragging, so no text reads as one.
        if row.name == "crop" {
            assert_eq!(
                ItemParamPatch::parse(row.name, "1"),
                Err(ParamParseError::Value)
            );
            continue;
        }
        let patch = SAMPLES
            .iter()
            .find_map(|text| ItemParamPatch::parse(row.name, text).ok())
            .unwrap_or_else(|| panic!("{} reads no sample text", row.name));
        assert_eq!(patch.field(), row.name);
        assert_eq!(patch.row(), row);
        let wire = serde_json::to_value(&patch).unwrap();
        assert_eq!(wire["field"], row.name);
        assert_eq!(
            serde_json::from_value::<ItemParamPatch>(wire).unwrap(),
            patch
        );
    }
    assert_eq!(
        ItemParamPatch::parse("audio_gain", "1"),
        Err(ParamParseError::Unknown)
    );
    assert_eq!(item_param("audio_gain"), None);
}

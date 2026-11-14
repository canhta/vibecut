#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
use engine::{Command, EditError, NewItem, Placement, Session};
use media::{AssetId, SourceAsset};
use project::{ItemParams, Project, ProjectId, SceneId};
use time::{SourceRange, Tick};

fn asset(id: u64, path: &str) -> SourceAsset {
    testkit::source::asset(id, std::env::temp_dir().join(path))
        .format("mp4")
        .duration(Tick(10_000))
        .hash([id as u8; 32])
        .stream(testkit::source::video().duration(Tick(10_000)).build())
        .stream(
            testkit::source::audio()
                .index(1)
                .duration(Tick(10_000))
                .build(),
        )
        .build()
}

fn session() -> Session {
    let mut session =
        Session::new(Project::empty(ProjectId(90), "Media".into()).unwrap(), 20).unwrap();
    for (id, path) in [(1, "media-one.mp4"), (2, "media-two.mp4")] {
        session
            .apply(Command::AddAsset {
                asset: asset(id, path),
            })
            .unwrap();
    }
    session
}

/// Video of `asset` over `range` at output zero on the main track.
fn insert_video_range(
    session: &mut Session,
    asset: AssetId,
    range: SourceRange,
) -> project::ItemId {
    let main = session.project().data().scene().timeline.main.id;
    let outcome = session
        .apply(Command::InsertItem {
            placement: Placement::Track { track: main },
            item: NewItem {
                retime: Default::default(),
                animations: Default::default(),
                source_asset: Some(asset),
                source_range: range,
                start: Tick::ZERO,
                content: ItemParams::Video(project::VideoParams {
                    source_audio_enabled: true,
                    ..project::VideoParams::plain()
                }),
            },
        })
        .unwrap();
    match outcome.created.first() {
        Some(engine::CreatedId::Item(id)) => *id,
        _ => panic!("item"),
    }
}

fn insert_video(session: &mut Session, asset: AssetId) -> project::ItemId {
    insert_video_range(
        session,
        asset,
        SourceRange::new(Tick::ZERO, Tick(1_000)).unwrap(),
    )
}

/// A video (or audio-only) asset whose probed duration is `duration`.
fn asset_with(id: u64, path: &str, duration: Tick, video: bool) -> SourceAsset {
    let mut builder = testkit::source::asset(id, std::env::temp_dir().join(path))
        .format("mp4")
        .duration(duration)
        .hash([id as u8; 32]);
    if video {
        builder = builder.stream(testkit::source::video().duration(duration).build());
    }
    builder
        .stream(
            testkit::source::audio()
                .index(if video { 1 } else { 0 })
                .duration(duration)
                .build(),
        )
        .build()
}

#[test]
fn unused_asset_removal_is_undoable_and_restores_the_exact_record() {
    let mut session = session();
    let original = session.project().data().assets.clone();
    let history = session.history_len();
    session
        .apply(Command::RemoveAssets {
            assets: vec![AssetId(1)],
        })
        .unwrap();
    assert_eq!(
        session.project().data().assets,
        original[1..].to_vec(),
        "only the requested record is removed"
    );
    assert_eq!(session.history_len(), (history.0 + 1, 0));
    session.undo().unwrap();
    assert_eq!(
        session.project().data().assets,
        original,
        "undo restores the asset record byte-for-byte"
    );
    session.redo().unwrap();
    assert_eq!(session.project().data().assets, original[1..].to_vec());
}

#[test]
fn in_use_removal_refuses_with_the_exact_usage_and_changes_nothing() {
    let mut session = session();
    let item = insert_video(&mut session, AssetId(1));
    let before = session.project().clone();
    let history = session.history_len();
    let error = session
        .apply(Command::RemoveAssets {
            assets: vec![AssetId(1), AssetId(2)],
        })
        .unwrap_err();
    match error {
        EditError::AssetsInUse(usages) => {
            assert_eq!(usages.len(), 1);
            assert_eq!(usages[0].asset, AssetId(1));
            assert_eq!(usages[0].items, vec![item]);
        }
        other => panic!("wrong refusal: {other:?}"),
    }
    assert_eq!(session.project(), &before, "refused batch is atomic");
    assert_eq!(session.history_len(), history);
    // The unused sibling in the same batch is still present.
    assert_eq!(session.project().data().assets.len(), 2);
}

#[test]
fn usage_in_another_scene_is_reported() {
    let mut session = session();
    let scene = match session
        .apply(Command::CreateScene {
            name: "Second".into(),
        })
        .unwrap()
        .created
        .first()
        .copied()
    {
        Some(engine::CreatedId::Scene(id)) => id,
        _ => panic!("scene"),
    };
    session.apply(Command::SwitchScene { scene }).unwrap();
    let item = insert_video(&mut session, AssetId(2));
    session
        .apply(Command::SwitchScene { scene: SceneId(0) })
        .unwrap();
    match session
        .apply(Command::RemoveAssets {
            assets: vec![AssetId(2)],
        })
        .unwrap_err()
    {
        EditError::AssetsInUse(usages) => {
            assert_eq!(usages[0].asset, AssetId(2));
            assert_eq!(usages[0].items, vec![item]);
        }
        other => panic!("wrong refusal: {other:?}"),
    }
}

#[test]
fn hostile_removal_lists_are_refused_without_mutating_history() {
    let mut session = session();
    let before = session.project().clone();
    let history = session.history_len();
    for (assets, expected) in [
        (Vec::<AssetId>::new(), "empty"),
        (vec![AssetId(1), AssetId(1)], "duplicate"),
        (vec![AssetId(99)], "unknown"),
        (
            (0..=engine::control::MAX_COLLECTION as u64)
                .map(AssetId)
                .collect(),
            "oversized",
        ),
    ] {
        let error = session.apply(Command::RemoveAssets { assets }).unwrap_err();
        match (expected, error) {
            ("empty", EditError::Invalid("media removal list")) => {}
            ("duplicate", EditError::Invalid("duplicate asset ID")) => {}
            ("unknown", EditError::NoAsset(AssetId(99))) => {}
            ("oversized", EditError::Control(engine::control::ControlError::Limit)) => {}
            (_, other) => panic!("wrong refusal for {expected}: {other:?}"),
        }
        assert_eq!(session.project(), &before);
        assert_eq!(session.history_len(), history);
    }
}

#[test]
fn removing_a_referenced_asset_after_deleting_its_item_succeeds() {
    let mut session = session();
    let item = insert_video(&mut session, AssetId(1));
    session
        .apply(Command::Delete { items: vec![item] })
        .unwrap();
    session
        .apply(Command::RemoveAssets {
            assets: vec![AssetId(1)],
        })
        .unwrap();
    assert_eq!(
        session
            .project()
            .data()
            .assets
            .iter()
            .map(SourceAsset::id)
            .collect::<Vec<_>>(),
        vec![AssetId(2)]
    );
}

fn item_of(session: &Session, id: project::ItemId) -> project::Item {
    session
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| track.items.iter())
        .find(|item| item.id == id)
        .cloned()
        .unwrap()
}

#[test]
fn replace_refuses_short_kind_mismatched_and_hostile_targets_atomically() {
    let mut session = session();
    session
        .apply(Command::AddAsset {
            asset: asset_with(3, "media-short.mp4", Tick(500), true),
        })
        .unwrap();
    session
        .apply(Command::AddAsset {
            asset: asset_with(4, "media-audio.m4a", Tick(10_000), false),
        })
        .unwrap();
    let window = SourceRange::new(Tick(200), Tick(1_200)).unwrap();
    let item = insert_video_range(&mut session, AssetId(1), window);
    let main = session.project().data().scene().timeline.main.id;
    let overlay = match session
        .apply(Command::InsertItem {
            placement: Placement::Track { track: main },
            item: NewItem {
                retime: Default::default(),
                animations: Default::default(),
                source_asset: None,
                source_range: SourceRange::new(Tick::ZERO, Tick(1_000)).unwrap(),
                start: Tick(2_000),
                content: ItemParams::Overlay(project::OverlayParams {
                    color: project::Color([0, 0, 0, 255]),
                }),
            },
        })
        .unwrap()
        .created
        .first()
    {
        Some(engine::CreatedId::Item(id)) => *id,
        _ => panic!("overlay"),
    };
    let before = session.project().clone();
    let history = session.history_len();
    for (items, asset, expected) in [
        (
            vec![item],
            AssetId(3),
            EditError::Invalid("replacement source too short"),
        ),
        (
            vec![item],
            AssetId(4),
            EditError::Invalid("replacement media kind mismatch"),
        ),
        (vec![item], AssetId(9), EditError::NoAsset(AssetId(9))),
        (
            vec![overlay],
            AssetId(2),
            EditError::Unsupported("replace media on non-media item"),
        ),
        (
            Vec::new(),
            AssetId(2),
            EditError::Invalid("empty item selection"),
        ),
        (
            vec![item, item],
            AssetId(2),
            EditError::Invalid("duplicate replace item"),
        ),
        (
            vec![item, project::ItemId(99)],
            AssetId(2),
            EditError::NoItem(project::ItemId(99)),
        ),
    ] {
        let error = session
            .apply(Command::ReplaceMedia { items, asset })
            .unwrap_err();
        assert_eq!(format!("{error:?}"), format!("{expected:?}"));
        assert_eq!(session.project(), &before, "refusal changes nothing");
        assert_eq!(session.history_len(), history);
    }
    session
        .apply(Command::SetTrackLocked {
            track: main,
            locked: true,
        })
        .unwrap();
    let locked = session.project().clone();
    let error = session
        .apply(Command::ReplaceMedia {
            items: vec![item],
            asset: AssetId(2),
        })
        .unwrap_err();
    assert!(matches!(error, EditError::Locked(track) if track == main));
    assert_eq!(session.project(), &locked);
}

/// A clip at 2x over a mid-source window carrying an effect, a mask and an opacity key.
fn dressed_clip(session: &mut Session) -> project::ItemId {
    let item = insert_video_range(
        session,
        AssetId(1),
        SourceRange::new(Tick(2_000), Tick(6_000)).unwrap(),
    );
    session
        .apply(Command::SetSpeed {
            items: vec![item],
            speed: time::Speed::from_f64(2.0).unwrap(),
            maintain_pitch: false,
        })
        .unwrap();
    session
        .apply(Command::AddEffect {
            item,
            effect: project::effect_definition("blur")
                .unwrap()
                .instance("blur1"),
        })
        .unwrap();
    session
        .apply(Command::AddMask {
            item,
            mask: project::Mask::new("mask1", project::MaskShape::Rectangle),
        })
        .unwrap();
    session
        .apply(Command::ToggleKeyframe {
            item,
            path: project::BuiltinPath::Opacity.into(),
            component: None,
            time: Tick(2),
            value: project::ParamValue::Scalar(0.5),
        })
        .unwrap();
    item
}

#[test]
fn replace_media_keeps_every_attribute_and_plays_the_new_media_from_its_start() {
    let mut session = session();
    session
        .apply(Command::AddAsset {
            asset: asset_with(3, "media-long.mp4", Tick(20_000), true),
        })
        .unwrap();
    let item = dressed_clip(&mut session);
    let original = item_of(&session, item);
    assert_eq!(original.duration().unwrap(), Tick(2_000), "2x of 4000");
    assert!(!original.effects.is_empty() && !original.masks.is_empty());
    let before = session.project().clone();
    let history = session.history_len();
    for asset in [AssetId(2), AssetId(3)] {
        session
            .apply(Command::ReplaceMedia {
                items: vec![item],
                asset,
            })
            .unwrap();
        let replaced = item_of(&session, item);
        assert_eq!(replaced.source_asset, Some(asset), "asset reference");
        assert_eq!(
            replaced.source_range,
            SourceRange::new(Tick::ZERO, Tick(4_000)).unwrap(),
            "source in point zero, 4000 source ticks fill 2000 output ticks at 2x"
        );
        assert_eq!(replaced.start, original.start, "timeline start");
        assert_eq!(replaced.duration().unwrap(), original.duration().unwrap());
        assert_eq!(replaced.retime, original.retime, "speed, pitch, reverse");
        assert_eq!(replaced.content, original.content, "volume and params");
        assert_eq!(replaced.effects, original.effects, "effects");
        assert_eq!(replaced.masks, original.masks, "masks");
        assert_eq!(replaced.animations, original.animations, "keyframes");
        assert_eq!(
            replaced,
            project::Item {
                source_asset: Some(asset),
                source_range: SourceRange::new(Tick::ZERO, Tick(4_000)).unwrap(),
                ..original.clone()
            },
            "nothing else changes"
        );
        assert_eq!(session.history_len(), (history.0 + 1, 0), "one undo step");
        session.undo().unwrap();
        assert_eq!(session.project(), &before, "undo restores the exact project");
    }
}

#[test]
fn replace_media_replaces_a_clip_and_its_extracted_audio_together_in_one_step() {
    let mut session = session();
    let video = insert_video_range(
        &mut session,
        AssetId(1),
        SourceRange::new(Tick(200), Tick(1_200)).unwrap(),
    );
    let created = session
        .apply(Command::ExtractAudio { items: vec![video] })
        .unwrap()
        .created;
    let audio = created
        .iter()
        .find_map(|id| match id {
            engine::CreatedId::Item(id) => Some(*id),
            _ => None,
        })
        .unwrap();
    let before = session.project().clone();
    let history = session.history_len();
    session
        .apply(Command::ReplaceMedia {
            items: vec![video, audio],
            asset: AssetId(2),
        })
        .unwrap();
    for id in [video, audio] {
        let replaced = item_of(&session, id);
        assert_eq!(replaced.source_asset, Some(AssetId(2)));
        assert_eq!(
            replaced.source_range,
            SourceRange::new(Tick::ZERO, Tick(1_000)).unwrap()
        );
    }
    assert_eq!(session.history_len(), (history.0 + 1, 0), "one undo step");
    session.undo().unwrap();
    assert_eq!(session.project(), &before);
}

#[test]
fn relink_moves_the_record_path_and_keeps_every_clip_on_it() {
    let mut session = session();
    let item = insert_video_range(
        &mut session,
        AssetId(1),
        SourceRange::new(Tick::ZERO, Tick(5_000)).unwrap(),
    );
    let history = session.history_len();
    // Same identity and content hash on another path.
    session
        .apply(Command::RelinkAsset {
            asset: asset(1, "media-moved.mp4"),
        })
        .unwrap();
    let record = session
        .project()
        .data()
        .assets
        .iter()
        .find(|record| record.id() == AssetId(1))
        .unwrap();
    assert_eq!(record.path(), std::env::temp_dir().join("media-moved.mp4"));
    assert_eq!(session.history_len(), (history.0 + 1, 0), "one undo step");
    let items: Vec<_> = session
        .project()
        .data()
        .scene()
        .timeline
        .main
        .items
        .iter()
        .map(|i| (i.id, i.source_asset))
        .collect();
    assert_eq!(items, vec![(item, Some(AssetId(1)))]);
    // Another content hash and an unknown asset refuse with no change.
    let relinked = session.project().clone();
    let other_hash = testkit::source::asset(1, std::env::temp_dir().join("media-moved.mp4"))
        .format("mp4")
        .duration(Tick(10_000))
        .hash([9; 32])
        .build();
    for (other, expected) in [
        (other_hash, "Invalid(\"relink content differs\")"),
        (asset(9, "media-moved.mp4"), "NoAsset(AssetId(9))"),
    ] {
        let error = session
            .apply(Command::RelinkAsset { asset: other })
            .unwrap_err();
        assert_eq!(format!("{error:?}"), expected);
        assert_eq!(session.project(), &relinked);
    }
}

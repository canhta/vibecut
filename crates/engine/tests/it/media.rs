#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
use engine::{Command, EditError, NewItem, Placement, Session};
use media::{
    AssetData, AssetId, AudioInfo, ColorInfo, ContentHash, SourceAsset, SourceProbe, SourceStream,
    StreamKind, TimeBase, VideoInfo,
};
use project::{ItemParams, Project, ProjectId};
use time::{FrameRate, SourceRange, Tick};

fn asset(id: u64, path: &str) -> SourceAsset {
    SourceAsset::try_from(AssetData {
        ephemeral: false,
        id: AssetId(id),
        path: std::env::temp_dir().join(path),
        content_hash: ContentHash([id as u8; 32]),
        probe: SourceProbe {
            format: "mp4".into(),
            duration: Some(Tick(10_000)),
            streams: vec![
                SourceStream {
                    index: 0,
                    kind: StreamKind::Video,
                    codec: "h264".into(),
                    time_base: TimeBase::new(1, 1_000).unwrap(),
                    duration: Some(Tick(10_000)),
                    start: None,
                    frame_rate: Some(FrameRate::FPS_30),
                    frame_count: None,
                    variable_frame_rate: None,
                    video: Some(VideoInfo {
                        width: 1920,
                        height: 1080,
                        color: ColorInfo::default(),
                        bit_depth: Some(8),
                    }),
                    audio: None,
                },
                SourceStream {
                    index: 1,
                    kind: StreamKind::Audio,
                    codec: "aac".into(),
                    time_base: TimeBase::new(1, 1_000).unwrap(),
                    duration: Some(Tick(10_000)),
                    start: None,
                    frame_rate: None,
                    frame_count: None,
                    variable_frame_rate: None,
                    video: None,
                    audio: Some(AudioInfo {
                        sample_rate: 48_000,
                        channels: 2,
                        initial_padding: 0,
                        trailing_padding: 0,
                    }),
                },
            ],
        },
    })
    .unwrap()
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

fn insert_video(session: &mut Session, asset: AssetId) -> project::ItemId {
    let main = session.project().data().scene().timeline.main.id;
    let outcome = session
        .apply(Command::InsertItem {
            placement: Placement::Track { track: main },
            item: NewItem {
                animations: Default::default(),
                source_asset: Some(asset),
                source_range: SourceRange::new(Tick::ZERO, Tick(1_000)).unwrap(),
                start: Tick::ZERO,
                content: ItemParams::Video(project::VideoParams {
                    source_audio_enabled: true,
                }),
            },
        })
        .unwrap();
    match outcome.created.first() {
        Some(engine::CreatedId::Item(id)) => *id,
        _ => panic!("item"),
    }
}

#[test]
fn unused_asset_removal_is_undoable_and_restores_the_exact_record() {
    let mut session = session();
    let original = session.project().data().assets.clone();
    let history = session.history_len();
    session
        .execute("media.remove", serde_json::json!({"assets":[1]}))
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
        .execute("media.remove", serde_json::json!({"assets":[1,2]}))
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
        .execute("scene.create", serde_json::json!({"name":"Second"}))
        .unwrap()
        .created
        .first()
        .copied()
    {
        Some(engine::CreatedId::Scene(id)) => id,
        _ => panic!("scene"),
    };
    session
        .execute("scene.switch", serde_json::json!({"scene":scene}))
        .unwrap();
    let item = insert_video(&mut session, AssetId(2));
    session
        .execute("scene.switch", serde_json::json!({"scene":0}))
        .unwrap();
    match session
        .execute("media.remove", serde_json::json!({"assets":[2]}))
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
        let error = session
            .execute("media.remove", serde_json::json!({"assets":assets}))
            .unwrap_err();
        match (expected, error) {
            ("empty", EditError::Invalid("media removal list")) => {}
            ("duplicate", EditError::Invalid("duplicate asset ID")) => {}
            ("unknown", EditError::NoAsset(AssetId(99))) => {}
            ("oversized", EditError::Limit) => {}
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
        .execute("media.remove", serde_json::json!({"assets":[1]}))
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

#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
use crate::support::control::{request, write_png};
use engine::control::{Caller, ControlResult, Policy, ResponseValue};
use engine::{Command, CreatedId, Session};
use media::AssetId;
use project::{
    Item, ItemId, ItemParams, OverlayParams, Project, ProjectId, Track, TrackId, TrackKind,
};
use serde_json::{Value, json};
use std::{path::Path, sync::Arc};
use time::{FrameRate, SourceRange, TICKS_PER_SECOND, Tick};

struct Allow;
impl Policy for Allow {
    fn authorize(&self, _: &Command, _: &Project) -> engine::control::Result<()> {
        Ok(())
    }
}

fn secs(n: i64) -> i64 {
    n * TICKS_PER_SECOND
}

/// A 64x64 still PNG host on video track 1 (items 1 and 2), an overlay on track 2 (item 3).
fn session(dir: &Path) -> Session {
    let png = dir.join("host.png");
    write_png(&png);
    let mut data = Project::empty(ProjectId(1), "Freeze".into())
        .unwrap()
        .into_data();
    data.canvas.width = 64;
    data.canvas.height = 64;
    data.canvas.fps = FrameRate::FPS_30;
    data.assets
        .push(media::SourceAsset::import(AssetId(5), png).unwrap());
    data.scene_mut().timeline.insert_track(Track {
        id: TrackId(1),
        name: "Video".into(),
        kind: TrackKind::Video,
        muted: false,
        hidden: false,
        locked: false,
        items: vec![
            testkit::item::video(ItemId(1))
                .asset(AssetId(5))
                .source(SourceRange::new(Tick::ZERO, Tick(secs(2))).unwrap())
                .start(Tick(0))
                .build(),
            testkit::item::video(ItemId(2))
                .asset(AssetId(5))
                .source(SourceRange::new(Tick::ZERO, Tick(secs(1))).unwrap())
                .start(Tick(secs(2)))
                .build(),
        ],
    });
    data.scene_mut().timeline.insert_track(Track {
        id: TrackId(2),
        name: "Overlay".into(),
        kind: TrackKind::Video,
        muted: false,
        hidden: false,
        locked: false,
        items: vec![Item {
            content: ItemParams::Overlay(OverlayParams {
                color: project::Color([0, 0, 0, 255]),
            }),
            source_asset: None,
            source_range: SourceRange::new(Tick::ZERO, Tick(secs(1))).unwrap(),
            ..video(3, 0, secs(1))
        }],
    });
    Session::new(Project::new(data).unwrap(), 10).unwrap()
}

fn caller() -> Caller<'static> {
    Caller {
        principal: "test",
        external: false,
        policy: &Allow,
        fonts: None,
        sounds: None,
        music: None,
        stickers: None,
        brand: None,
        media: None,
        chapters: None,
        captions: None,
        vision: None,
    }
}

fn runner() -> engine::jobs::JobRunner {
    engine::jobs::JobRunner::new(
        engine::jobs::RunnerConfig {
            concurrency: [1, 1, 1],
            queue: 4,
            retained: 4,
            network_attempts: 1,
            backoff: std::time::Duration::from_millis(1),
            progress_interval: std::time::Duration::from_millis(10),
        },
        engine::ByteBudget::new(512 << 20).unwrap(),
        Arc::new(engine::jobs::SystemClock::default()),
    )
    .unwrap()
}

fn send(s: &mut Session, id: &str, params: Value, key: &str) -> engine::control::Response {
    let mut jobs = runner();
    let req = request(s, id, params, key);
    s.control_request(req, caller(), &mut jobs).unwrap()
}

fn freeze_params(item: u64, at: i64, duration: i64, dir: &Path) -> Value {
    json!({"item": item, "at": at, "duration": duration, "dir": dir})
}

/// (start, duration) of every item on track 1, in output order.
fn track_one(s: &Session) -> Vec<(Tick, Tick)> {
    let track = s
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .find(|track| track.id == TrackId(1))
        .unwrap();
    track
        .items
        .iter()
        .map(|item| (item.start, item.duration().unwrap()))
        .collect()
}

fn asset_path(s: &Session, id: AssetId) -> std::path::PathBuf {
    s.project()
        .data()
        .assets
        .iter()
        .find(|asset| asset.id() == id)
        .unwrap()
        .path()
        .to_owned()
}

fn still_item(s: &Session, id: ItemId) -> Item {
    s.project()
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| track.items.iter())
        .find(|item| item.id == id)
        .cloned()
        .unwrap()
}

fn freeze_files(dir: &Path) -> usize {
    std::fs::read_dir(dir)
        .unwrap()
        .filter(|entry| {
            entry
                .as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("freeze-")
        })
        .count()
}

#[test]
fn freeze_holds_the_frame_between_split_halves_and_ripples_later_items() {
    let root = tempfile::tempdir().unwrap();
    let mut s = session(root.path());
    let history = s.history_len();
    let before = project::encode(s.project(), false).unwrap();
    let response = send(
        &mut s,
        "timeline.freeze_frame",
        freeze_params(1, secs(1), secs(3), root.path()),
        "freeze-split",
    );
    let ResponseValue::Result(ControlResult::Edit { outcome }) = response.value else {
        panic!("freeze refused");
    };
    assert_eq!(
        track_one(&s),
        vec![
            (Tick::ZERO, Tick(secs(1))),
            (Tick(secs(1)), Tick(secs(3))),
            (Tick(secs(4)), Tick(secs(1))),
            (Tick(secs(5)), Tick(secs(1))),
        ],
        "left half, held still, right half, then the later item rippled by the hold"
    );
    let CreatedId::Item(still) = outcome.created[0] else {
        panic!("still item id");
    };
    assert_eq!(still_item(&s, still).start, Tick(secs(1)));
    assert_eq!(s.history_len(), (history.0 + 1, 0), "one undo step");
    s.undo().unwrap();
    assert_eq!(project::encode(s.project(), false).unwrap(), before);
}

#[test]
fn freeze_still_pixels_equal_the_render_frame_at_the_playhead_and_is_named_by_hash() {
    let root = tempfile::tempdir().unwrap();
    let mut s = session(root.path());
    // Half a second in, the black overlay covers the host, so the composite is a new picture
    // and not the host file's own bytes (an identical frame would reuse the host asset).
    let at = secs(1) / 2;
    let rendered_before = engine::snapshot::image(s.project(), Tick(at)).unwrap().png;
    let response = send(
        &mut s,
        "timeline.freeze_frame",
        freeze_params(1, at, secs(3), root.path()),
        "freeze-pixels",
    );
    let ResponseValue::Result(ControlResult::Edit { outcome }) = response.value else {
        panic!("freeze refused");
    };
    let CreatedId::Item(still) = outcome.created[0] else {
        panic!("still item id");
    };
    let asset = still_item(&s, still).source_asset.unwrap();
    let path = asset_path(&s, asset);
    let published = std::fs::read(&path).unwrap();
    assert_eq!(
        published, rendered_before,
        "pixels equal the playhead render"
    );
    let hash = media::ContentHash::from_bytes(&published);
    let hex: String = hash.0.iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(
        path.file_name().unwrap().to_string_lossy(),
        format!("freeze-{hex}.png")
    );
}

#[test]
fn freezing_the_same_frame_again_reuses_its_asset() {
    let root = tempfile::tempdir().unwrap();
    let mut s = session(root.path());
    let response = send(
        &mut s,
        "timeline.freeze_frame",
        freeze_params(1, secs(1), secs(3), root.path()),
        "freeze-first",
    );
    let ResponseValue::Result(ControlResult::Edit { outcome }) = response.value else {
        panic!("first freeze refused");
    };
    let CreatedId::Item(still) = outcome.created[0] else {
        panic!("still item id");
    };
    let assets = s.project().data().assets.len();
    // The still shows the same single frame, so its render hashes to the same PNG.
    let response = send(
        &mut s,
        "timeline.freeze_frame",
        freeze_params(still.0, secs(2), secs(1), root.path()),
        "freeze-again",
    );
    assert!(matches!(
        response.value,
        ResponseValue::Result(ControlResult::Edit { .. })
    ));
    assert_eq!(s.project().data().assets.len(), assets, "asset reused");
    assert_eq!(freeze_files(root.path()), 1, "one published frame");
}

#[test]
fn freeze_refusals_change_nothing() {
    let root = tempfile::tempdir().unwrap();
    let mut s = session(root.path());
    let history = s.history_len();
    let before = project::encode(s.project(), false).unwrap();
    let revision = s.revision();
    for (params, key) in [
        (
            json!({"item": 1, "at": secs(1), "duration": secs(3), "dir": "relative-out"}),
            "refuse-relative",
        ),
        (freeze_params(1, secs(1), 0, root.path()), "refuse-zero"),
        (freeze_params(1, 0, secs(3), root.path()), "refuse-at-start"),
        (
            freeze_params(1, secs(2), secs(3), root.path()),
            "refuse-at-end",
        ),
        (
            freeze_params(3, secs(1) / 2, secs(3), root.path()),
            "refuse-non-video",
        ),
        (
            freeze_params(99, secs(1), secs(3), root.path()),
            "refuse-missing",
        ),
    ] {
        let response = send(&mut s, "timeline.freeze_frame", params, key);
        assert!(
            matches!(response.value, ResponseValue::Error(_)),
            "{key}: {response:?}"
        );
        assert_eq!(
            project::encode(s.project(), false).unwrap(),
            before,
            "{key}"
        );
        assert_eq!(s.history_len(), history, "{key}");
        assert_eq!(s.revision(), revision, "{key}");
    }
    assert_eq!(freeze_files(root.path()), 0, "refusals publish nothing");
    send(
        &mut s,
        "track.set_locked",
        json!({"track": 1, "locked": true}),
        "lock-track",
    );
    let locked = project::encode(s.project(), false).unwrap();
    let response = send(
        &mut s,
        "timeline.freeze_frame",
        freeze_params(1, secs(1), secs(3), root.path()),
        "refuse-locked",
    );
    assert!(matches!(response.value, ResponseValue::Error(_)));
    assert_eq!(project::encode(s.project(), false).unwrap(), locked);
}

#[test]
fn frozen_still_survives_reopen_with_a_relink_state_when_its_file_is_gone() {
    let root = tempfile::tempdir().unwrap();
    let mut s = session(root.path());
    let response = send(
        &mut s,
        "timeline.freeze_frame",
        freeze_params(1, secs(1), secs(3), root.path()),
        "freeze-reopen",
    );
    let ResponseValue::Result(ControlResult::Edit { outcome }) = response.value else {
        panic!("freeze refused");
    };
    let CreatedId::Item(still) = outcome.created[0] else {
        panic!("still item id");
    };
    let asset = still_item(&s, still).source_asset.unwrap();
    let path = asset_path(&s, asset);
    let bytes = project::encode(s.project(), false).unwrap();
    let loaded = project::decode(&bytes).unwrap();
    assert_eq!(
        project::encode(&loaded.project, false).unwrap(),
        bytes,
        "round trip"
    );
    std::fs::remove_file(&path).unwrap();
    let reopened = project::decode(&bytes).unwrap();
    let record = reopened
        .project
        .data()
        .assets
        .iter()
        .find(|record| record.id() == asset)
        .unwrap();
    assert_eq!(
        record.path(),
        path.as_path(),
        "relink keeps the recorded path"
    );
    assert!(!record.path().exists(), "missing file is the relink state");
}

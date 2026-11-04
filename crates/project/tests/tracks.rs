#![allow(clippy::unwrap_used)]
use project::{Error, Project, ProjectId, Track, TrackId, TrackKind};
fn empty() -> Project {
    Project::empty(ProjectId(1), "Tracks".into()).unwrap()
}
fn track(id: u64, kind: TrackKind) -> Track {
    Track {
        id: TrackId(id),
        name: "Overlay".into(),
        kind,
        muted: false,
        hidden: false,
        locked: false,
        items: vec![],
    }
}
#[test]
fn layout_order_and_flags_roundtrip() {
    let mut d = empty().into_data();
    for (id, kind) in [
        (1, TrackKind::Video),
        (2, TrackKind::Text),
        (3, TrackKind::Sticker),
        (4, TrackKind::Effect),
        (5, TrackKind::Filter),
        (6, TrackKind::Adjustment),
        (7, TrackKind::Audio),
        (8, TrackKind::Audio),
    ] {
        d.timeline.insert_track(track(id, kind));
    }
    d.timeline.main.muted = true;
    d.timeline.main.hidden = true;
    d.timeline.magnet = false;
    assert_eq!(
        d.timeline.iter().map(|t| t.id.0).collect::<Vec<_>>(),
        [7, 8, 0, 1, 2, 3, 4, 5, 6]
    );
    assert_eq!(
        d.timeline.iter().rev().map(|t| t.id.0).collect::<Vec<_>>(),
        [6, 5, 4, 3, 2, 1, 0, 8, 7]
    );
    let project = Project::new(d).unwrap();
    assert_eq!(
        project::decode(&project::encode(&project, false).unwrap())
            .unwrap()
            .project,
        project
    );
}
#[test]
fn main_and_group_invariants_refuse_invalid_current_files() {
    let d = empty().into_data();
    let mut bad = d.clone();
    bad.timeline.main.kind = TrackKind::Audio;
    assert!(Project::new(bad).is_err());
    let mut bad = d.clone();
    bad.timeline.audio.push(track(1, TrackKind::Text));
    assert!(Project::new(bad).is_err());
    let mut bad = d.clone();
    bad.timeline.overlays.push(track(1, TrackKind::Audio));
    assert!(Project::new(bad).is_err());
    let mut bad = d.clone();
    bad.timeline.insert_track(track(0, TrackKind::Video));
    assert!(Project::new(bad).is_err());
    let mut json = serde_json::to_value(d).unwrap();
    json["timeline"].as_object_mut().unwrap().remove("main");
    assert!(serde_json::from_value::<Project>(json).is_err());
}
#[test]
fn before_new_kinds_schema_is_typed_too_old() {
    // A complete pre-layout file, including the old flat vector and old kind.
    let old=br#"{"format":"vibecut.project","schema_version":3,"generator":"test","project":{"id":"1","name":"Old","canvas":{"width":1920,"height":1080,"fps":{"num":30,"den":1},"background":[0,0,0,255]},"assets":[],"timeline":{"tracks":[{"id":1,"name":"Old overlay","kind":"overlay","muted":false,"hidden":false,"locked":false,"items":[]}]},"bookmarks":[],"settings":{"audio_sample_rate":48000}}}"#;
    assert!(matches!(
        project::decode(old),
        Err(Error::TooOld {
            found: 3,
            supported: 5
        })
    ));
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("before-track-kinds.vibecut");
    std::fs::write(&path, old).unwrap();
    assert!(matches!(
        project::load(&path),
        Err(Error::TooOld {
            found: 3,
            supported: 5
        })
    ));
    assert_eq!(std::fs::read(path).unwrap(), old);
    // Schema 4 on main had effects with a flat timeline, before grouped tracks.
    let previous = std::str::from_utf8(old)
        .unwrap()
        .replace("schema_version\":3", "schema_version\":4");
    assert!(matches!(
        project::decode(previous.as_bytes()),
        Err(Error::TooOld {
            found: 4,
            supported: 5
        })
    ));
}

#[test]
fn generated_video_and_caption_kind_validation() {
    use project::{CaptionParams, Color, Item, ItemId, ItemParams, OverlayParams};
    use time::{SourceRange, Tick};
    let mut data = empty().into_data();
    let color = ItemParams::Overlay(OverlayParams {
        color: Color([0, 0, 255, 255]),
    });
    assert_eq!(color.kind(), TrackKind::Video);
    assert_eq!(
        ItemParams::Caption(CaptionParams {
            cues: captions::CueTrack { cues: vec![] }
        })
        .kind(),
        TrackKind::Text
    );
    data.timeline.main.items.push(Item {
        id: ItemId(1),
        effects: Vec::new(),
        animations: Default::default(),
        source_asset: None,
        source_range: SourceRange::new(Tick::ZERO, Tick(10)).unwrap(),
        start: Tick::ZERO,
        content: color,
    });
    let valid = Project::new(data).unwrap();
    for kind in [
        TrackKind::Sticker,
        TrackKind::Text,
        TrackKind::Effect,
        TrackKind::Filter,
        TrackKind::Adjustment,
        TrackKind::Audio,
    ] {
        let mut wrong = valid.clone().into_data();
        let mut target = track(1, kind);
        target.items = std::mem::take(&mut wrong.timeline.main.items);
        wrong.timeline.insert_track(target);
        assert!(
            Project::new(wrong).is_err(),
            "{kind:?} refuses video content"
        );
    }
}

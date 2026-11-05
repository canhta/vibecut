#![allow(clippy::unwrap_used, reason = "validated test fixtures and assertions")]
use captions::{Cue, CueText, CueTrack, LanguageTag};
use media::{
    AssetAvailability, AssetData, AssetId, AudioInfo, ColorInfo, ContentHash, SourceAsset,
    SourceProbe, SourceStream, StreamKind, TimeBase, VideoInfo,
};
use project::*;
use std::{collections::BTreeMap, fs, path::PathBuf};
use time::{FrameRate, SourceRange, Tick};

fn save_fixture(path: &std::path::Path, project: &Project) -> project::Result<SaveReceipt> {
    WriterLease::acquire(path)?.save(project)
}
fn range(start: i64, end: i64) -> SourceRange {
    SourceRange::new(Tick(start), Tick(end)).unwrap()
}
fn empty() -> Project {
    Project::empty(ProjectId(1), "Dự án Việt Nam".into()).unwrap()
}
fn source(path: PathBuf) -> SourceAsset {
    SourceAsset::try_from(AssetData {
        ephemeral: false,
        id: AssetId(3),
        path,
        content_hash: ContentHash([1; 32]),
        probe: SourceProbe {
            format: "mp4".into(),
            duration: Some(Tick(1000)),
            streams: vec![
                SourceStream {
                    index: 0,
                    kind: StreamKind::Video,
                    codec: "h264".into(),
                    time_base: TimeBase::new(1, 30000).unwrap(),
                    duration: Some(Tick(1000)),
                    start: Some(Tick(0)),
                    frame_rate: Some(FrameRate::FPS_29_97),
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
                    time_base: TimeBase::new(1, 48000).unwrap(),
                    duration: None,
                    start: Some(Tick(25)),
                    frame_rate: None,
                    frame_count: None,
                    variable_frame_rate: None,
                    video: None,
                    audio: Some(AudioInfo {
                        sample_rate: 48000,
                        channels: 2,
                        initial_padding: 1024,
                        trailing_padding: 0,
                    }),
                },
            ],
        },
    })
    .unwrap()
}
fn item(id: u64, start: i64) -> Item {
    Item {
        blend_mode: Default::default(),
        masks: Vec::new(),
        effects: Vec::new(),
        animations: Default::default(),
        id: ItemId(id),
        source_asset: None,
        source_range: range(0, 10),
        start: Tick(start),
        content: ItemParams::Text(TextParams {
            content: "Tiếng Việt".into(),
            font_size: 32.0,
            color: Color([255; 4]),
            ..Default::default()
        }),
    }
}
fn track(items: Vec<Item>) -> Track {
    Track {
        id: TrackId(2),
        name: "Chữ".into(),
        kind: TrackKind::Text,
        muted: false,
        hidden: false,
        locked: false,
        items,
    }
}
fn populated(path: PathBuf) -> Project {
    let mut data = empty().into_data();
    data.canvas.fps = FrameRate::FPS_29_97;
    data.assets.push(source(path));
    let cue = Cue {
        id: "cue-1".into(),
        range: range(0, 10),
        source_lang: LanguageTag::parse("vi").unwrap(),
        texts: BTreeMap::from([(
            LanguageTag::parse("en").unwrap(),
            CueText {
                text: "Hello".into(),
                edited: true,
                translation_ref: Some("translation-1".into()),
            },
        )]),
    };
    let contents = [
        ItemParams::Video(VideoParams {
            source_audio_enabled: true,
        }),
        ItemParams::Audio(AudioParams {
            gain_db: -6.0,
            muted: true,
        }),
        item(0, 0).content,
        ItemParams::Caption(CaptionParams {
            cues: CueTrack { cues: vec![cue] },
        }),
        ItemParams::Overlay(OverlayParams {
            color: Color([40, 50, 60, 255]),
        }),
    ];
    for (i, content) in contents.into_iter().enumerate() {
        let source_asset = matches!(
            content,
            ItemParams::Video(_) | ItemParams::Audio(_) | ItemParams::Caption(_)
        )
        .then_some(AssetId(3));
        data.scene_mut().timeline.insert_track(Track {
            id: TrackId(i as u64 + 1),
            name: "Track".into(),
            kind: content.kind(),
            muted: true,
            hidden: true,
            locked: true,
            items: vec![Item {
                blend_mode: Default::default(),
                masks: Vec::new(),
                effects: Vec::new(),
                animations: Default::default(),
                id: ItemId(i as u64),
                source_asset,
                source_range: range(0, 10),
                start: Tick(5),
                content,
            }],
        });
    }
    for (index, kind) in [
        TrackKind::Sticker,
        TrackKind::Effect,
        TrackKind::Filter,
        TrackKind::Adjustment,
    ]
    .into_iter()
    .enumerate()
    {
        data.scene_mut().timeline.insert_track(Track {
            id: TrackId(index as u64 + 6),
            name: "Empty overlay".into(),
            kind,
            muted: false,
            hidden: true,
            locked: false,
            items: vec![],
        });
    }
    data.scene_mut().timeline.main.muted = true;
    data.scene_mut().timeline.main.hidden = true;
    data.scene_mut().timeline.magnet = false;
    data.scene_mut().bookmarks.push(Bookmark {
        id: BookmarkId(1),
        start: Tick(5),
        duration: Tick(10),
        note: "Ghi chú".into(),
        color: Color([1, 2, 3, 255]),
    });
    Project::new(data).unwrap()
}
#[test]
fn all_kinds_roundtrip_missing_unicode_media_and_caption_identity() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("nguồn phim Việt Nam.mp4");
    let project = populated(missing.clone());
    for pretty in [false, true] {
        let bytes = encode(&project, pretty).unwrap();
        let loaded = decode(&bytes).unwrap();
        assert_eq!(loaded.project, project);
        assert_eq!(loaded.schema_version, SCHEMA_VERSION);
        assert_eq!(loaded.project.duration(), Tick(15));
        let asset = loaded.project.data().assets.first().unwrap();
        assert_eq!(asset.path(), missing);
        assert_eq!(asset.availability().unwrap(), AssetAvailability::Missing);
        assert_eq!(asset.probe().streams.get(1).unwrap().start, Some(Tick(25)));
        assert_eq!(
            serde_json::from_slice::<Project>(&serde_json::to_vec(&project).unwrap()).unwrap(),
            project
        );
    }
    let path = dir.path().join("dự án của tôi.vibecut");
    save_fixture(&path, &project).unwrap();
    assert_eq!(load(&path).unwrap().project, project);
}
#[test]
fn invalid_candidates_are_refused_without_changing_original() {
    let initial = empty();
    let cases: Vec<Vec<Item>> = vec![
        vec![item(1, -1)],
        vec![item(1, 0), item(2, 9)],
        vec![item(1, 20), item(2, 0)],
        vec![item(1, 0), item(1, 10)],
        vec![item(1, i64::MAX)],
    ];
    for items in cases {
        let mut d = initial.clone().into_data();
        d.scene_mut().timeline.insert_track(track(items));
        assert!(Project::new(d).is_err());
    }
    let mut d = initial.clone().into_data();
    d.scene_mut()
        .timeline
        .insert_track(track(vec![item(1, 0), item(2, 10)]));
    assert_eq!(Project::new(d).unwrap().duration(), Tick(20));
    assert_eq!(initial.duration(), Tick::ZERO);
}
#[test]
fn model_trust_boundaries() {
    let dir = tempfile::tempdir().unwrap();
    let p = populated(dir.path().join("a.mp4"));
    let base = p.into_data();
    let mut cases = Vec::new();
    let mut d = base.clone();
    d.scene_mut()
        .timeline
        .audio
        .first_mut()
        .unwrap()
        .items
        .first_mut()
        .unwrap()
        .blend_mode = BlendMode::Screen;
    cases.push(d);
    let mut d = base.clone();
    d.canvas.width = 0;
    cases.push(d);
    for (width, height) in [(0, 360), (640, 0), (u32::MAX, u32::MAX)] {
        let mut d = base.clone();
        d.original_canvas_size = Some(project::CanvasSize { width, height });
        cases.push(d);
    }
    let mut d = base.clone();
    d.canvas.fps = FrameRate::new(17, 1).unwrap();
    cases.push(d);
    let mut d = base.clone();
    d.settings.audio_sample_rate = 0;
    cases.push(d);
    let mut d = base.clone();
    d.assets.push(d.assets.first().unwrap().clone());
    cases.push(d);
    let mut d = base.clone();
    let duplicate = d.scene().timeline.overlays.first().unwrap().clone();
    d.scene_mut().timeline.insert_track(duplicate);
    cases.push(d);
    let mut d = base.clone();
    d.scene_mut()
        .timeline
        .iter_mut()
        .flat_map(|track| &mut track.items)
        .find(|item| item.id == ItemId(0))
        .unwrap()
        .source_asset = Some(AssetId(999));
    cases.push(d);
    let mut d = base.clone();
    d.scene_mut()
        .timeline
        .iter_mut()
        .find(|track| {
            track
                .items
                .iter()
                .any(|item| matches!(item.content, ItemParams::Video(_)))
        })
        .unwrap()
        .items
        .first_mut()
        .unwrap()
        .source_range = range(0, 1001);
    cases.push(d);
    let mut d = base.clone();
    d.scene_mut().bookmarks.first_mut().unwrap().duration = Tick(-1);
    cases.push(d);
    let mut d = base.clone();
    d.scene_mut()
        .timeline
        .audio
        .first_mut()
        .unwrap()
        .items
        .first_mut()
        .unwrap()
        .content = ItemParams::Audio(AudioParams {
        gain_db: f32::NAN,
        muted: false,
    });
    cases.push(d);
    let mut d = base.clone();
    let title = d
        .scene_mut()
        .timeline
        .iter_mut()
        .flat_map(|track| &mut track.items)
        .find(|item| matches!(item.content, ItemParams::Text(_)))
        .unwrap();
    if let ItemParams::Text(params) = &mut title.content {
        params.font_size = f32::INFINITY;
    }
    cases.push(d);
    let mut d = base.clone();
    d.scene_mut().timeline.main.kind = TrackKind::Audio;
    cases.push(d);
    let mut d = base.clone();
    d.scene_mut().timeline.main.id = TrackId(1);
    cases.push(d);
    for (audio_group, wrong_kind) in [(true, TrackKind::Text), (false, TrackKind::Audio)] {
        let mut d = base.clone();
        let wrong = Track {
            id: TrackId(100),
            name: "Wrong group".into(),
            kind: wrong_kind,
            muted: false,
            hidden: false,
            locked: false,
            items: vec![],
        };
        if audio_group {
            d.scene_mut().timeline.audio.push(wrong);
        } else {
            d.scene_mut().timeline.overlays.push(wrong);
        }
        cases.push(d);
    }
    for kind in [
        TrackKind::Sticker,
        TrackKind::Effect,
        TrackKind::Filter,
        TrackKind::Adjustment,
    ] {
        let mut d = base.clone();
        d.scene_mut()
            .timeline
            .overlays
            .iter_mut()
            .find(|track| {
                track
                    .items
                    .iter()
                    .any(|item| matches!(item.content, ItemParams::Overlay(_)))
            })
            .unwrap()
            .kind = kind;
        cases.push(d);
    }
    for d in cases {
        assert!(Project::new(d).is_err());
    }
}
#[test]
fn schema_errors_precede_interpreting_an_unknown_project_body() {
    for (version, body, old) in [
        (0, "null", true),
        (
            3,
            r#"{"id":"1","name":"Old","canvas":{"width":1920,"height":1080,"fps":{"num":30,"den":1},"background":[0,0,0,255]},"assets":[],"timeline":{"tracks":[{"id":1,"name":"Old overlay","kind":"overlay","muted":false,"hidden":false,"locked":false,"items":[]}]},"bookmarks":[],"settings":{"audio_sample_rate":48000}}"#,
            true,
        ),
        (4, "null", true),
        (5, "null", true),
        (6, "null", true),
        (7, "null", true),
        (8, "null", true),
        (SCHEMA_VERSION + 1, "null", false),
    ] {
        let bytes = format!(
            r#"{{"format":"vibecut.project","schema_version":{version},"generator":"test","project":{body}}}"#
        );
        let error = decode(bytes.as_bytes()).unwrap_err();
        assert!(if old {
            matches!(error, Error::TooOld { found, supported } if found == version && supported == SCHEMA_VERSION)
        } else {
            matches!(error, Error::TooNew { .. })
        });
    }
    let mut value: serde_json::Value =
        serde_json::from_slice(&encode(&empty(), false).unwrap()).unwrap();
    value["format"] = "other".into();
    assert!(matches!(
        decode(&serde_json::to_vec(&value).unwrap()),
        Err(Error::Invalid(_))
    ));
}
#[test]
fn hostile_json_is_bounded_and_errors_redact_text() {
    let huge = vec![b' '; (16 << 20) + 1];
    assert!(matches!(decode(&huge), Err(Error::TooLarge(_))));
    let deep = format!("{}0{}", "[".repeat(65), "]".repeat(65));
    assert!(matches!(decode(deep.as_bytes()), Err(Error::TooLarge(_))));
    let array = format!("[{}0]", "0,".repeat(200001));
    assert!(matches!(decode(array.as_bytes()), Err(Error::TooLarge(_))));
    let text = format!("\"{}\"", "x".repeat((1 << 20) + 1));
    assert!(matches!(decode(text.as_bytes()), Err(Error::TooLarge(_))));
    for bytes in [
        b"{}".as_slice(),
        b"null",
        b"{",
        br#"{"format":"PRIVATE USER TEXT"}"#,
    ] {
        let e = decode(bytes).unwrap_err();
        assert!(!format!("{e:?} {e}").contains("PRIVATE"));
    }
    let mut bytes = encode(&empty(), false).unwrap();
    bytes.extend_from_slice(b"[]");
    assert!(decode(&bytes).is_err());
    let mut value: serde_json::Value =
        serde_json::from_slice(&encode(&empty(), false).unwrap()).unwrap();
    value["project"]["canvas"]["fps"]["num"] = 0.into();
    assert!(decode(&serde_json::to_vec(&value).unwrap()).is_err());
}
#[test]
fn save_replacement_and_rotation_preserve_unrelated_files() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Một dự án.vibecut");
    let initial = empty();
    save_fixture(&path, &initial).unwrap();
    let updated = Project::empty(ProjectId(2), "New".into()).unwrap();
    save_fixture(&path, &updated).unwrap();
    assert_eq!(load(&path).unwrap().project, updated);
    let a = autosave::save(&path, &initial, 2).unwrap();
    let recovery = autosave::directory(&path);
    fs::write(recovery.join("leave.txt"), b"keep").unwrap();
    fs::create_dir(recovery.join("snapshot-00000000000000000099.vibecut")).unwrap();
    let b = autosave::save(&path, &updated, 2).unwrap();
    let c = autosave::save(&path, &initial, 2).unwrap();
    assert!(!a.exists());
    assert!(b.exists() && c.exists());
    assert_eq!(load(&c).unwrap().project, initial);
    assert_eq!(autosave::list(&path).unwrap(), vec![b, c]);
    assert_eq!(fs::read(recovery.join("leave.txt")).unwrap(), b"keep");
    assert!(autosave::save(&path, &initial, 0).is_err());
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 3);
}
#[test]
fn failed_publication_keeps_existing_directory_and_cleans_staging() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("target");
    fs::create_dir(&target).unwrap();
    fs::write(target.join("child"), b"good").unwrap();
    assert!(save_fixture(&target, &empty()).is_err());
    assert_eq!(fs::read(target.join("child")).unwrap(), b"good");
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
}
#[cfg(unix)]
#[test]
fn permissions_symlinks_and_unrepresentable_paths() {
    use std::os::unix::{
        ffi::OsStringExt,
        fs::{PermissionsExt, symlink},
    };
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("private.vibecut");
    fs::write(&path, b"old").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    save_fixture(&path, &empty()).unwrap();
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let link = dir.path().join("link");
    symlink(&path, &link).unwrap();
    assert!(atomic_write(&link, b"refused").is_err());
    assert!(fs::symlink_metadata(link).unwrap().file_type().is_symlink());
    let bad = dir.path().join(std::ffi::OsString::from_vec(vec![0xff]));
    let data = AssetData {
        ephemeral: false,
        id: AssetId(1),
        path: bad,
        content_hash: ContentHash([0; 32]),
        probe: SourceProbe {
            format: "unknown".into(),
            duration: None,
            streams: vec![],
        },
    };
    assert!(SourceAsset::try_from(data).is_err());
    let recovery = autosave::directory(&path);
    symlink(dir.path(), &recovery).unwrap();
    assert!(autosave::save(&path, &empty(), 2).is_err());
}
#[test]
fn relink_checks_content_and_preserves_identity() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("old");
    let b = dir.path().join("new Việt Nam");
    fs::write(&a, b"same").unwrap();
    fs::write(&b, b"same").unwrap();
    let mut data: AssetData = source(a.clone()).into();
    data.content_hash = ContentHash::read(&a).unwrap();
    let asset = SourceAsset::try_from(data).unwrap();
    let relinked = asset.relink(b.clone()).unwrap();
    assert_eq!(relinked.id(), asset.id());
    assert_eq!(relinked.content_hash(), asset.content_hash());
    fs::write(&b, b"different").unwrap();
    assert!(asset.relink(b).is_err());
}

proptest::proptest! {
    #[test]
    fn adjacent_integer_timing_roundtrips_without_drift(
        durations in proptest::collection::vec(1_i64..10000, 0..100),
        rate_index in 0_usize..FrameRate::COMMON.len(),
    ) {
        let mut data = empty().into_data(); data.canvas.fps = FrameRate::COMMON[rate_index];
        let mut end = 0;
        let mut items = Vec::new();
        for (id, duration) in durations.into_iter().enumerate() {
            let mut next = item(id as u64, end);
            next.source_range = range(0, duration); end += duration; items.push(next);
        }
        data.scene_mut().timeline.insert_track(track(items)); let project = Project::new(data).unwrap();
        proptest::prop_assert_eq!(project.duration(), Tick(end));
        proptest::prop_assert_eq!(decode(&encode(&project, false).unwrap()).unwrap().project, project);
    }
}

#[test]
fn encoding_limit_failure_never_replaces_saved_project_or_rotates_snapshots() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("project.vibecut");
    let initial = empty();
    save_fixture(&path, &initial).unwrap();
    let recovery = autosave::save(&path, &initial, 1).unwrap();
    let mut data = initial.clone().into_data();
    for id in 0..20 {
        let mut track = track(Vec::new());
        track.id = TrackId(id + 1);
        track.name = "a".repeat(900000).into();
        data.scene_mut().timeline.insert_track(track);
    }
    let oversized = Project::new(data).unwrap();
    assert!(matches!(
        save_fixture(&path, &oversized),
        Err(Error::TooLarge(_))
    ));
    assert!(autosave::save(&path, &oversized, 1).is_err());
    assert_eq!(load(&path).unwrap().project, initial);
    assert_eq!(autosave::list(&path).unwrap(), vec![recovery]);
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 3);
}

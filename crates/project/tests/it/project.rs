#![allow(clippy::unwrap_used, reason = "validated test fixtures and assertions")]
use captions::{Cue, CueStyleId, CueText, CueTrack, LanguageTag};
use media::{AssetAvailability, AssetData, AssetId, ContentHash, SourceAsset, TimeBase};
use project::*;
use std::{collections::BTreeMap, fs, path::PathBuf};
use time::{FrameRate, SourceRange, Tick};

fn save_fixture(path: &std::path::Path, project: &Project) -> project::Result<SaveReceipt> {
    WriterLease::acquire(path)?.save(project)
}

#[test]
fn caption_bank_validates_every_entry_names_closed_fields_and_schema() {
    let style: captions::CueStyle = serde_json::from_value(serde_json::json!({
        "font_family":"Inter", "font_size_ratio":0.05, "primary_color":[255,255,255,180],
        "outline_color":[0,0,0,255], "background_color":[0,0,0,0], "border_style":1,
        "alignment":{"numpad":2,"horizontal":"center","vertical":"bottom"},
        "margin_left_ratio":0.05,"margin_right_ratio":0.05,"margin_vertical_ratio":0.05,
        "bold":true,"italic":false,"underline":false,"strikeout":false
    }))
    .unwrap();
    let mut data = Project::empty(ProjectId(99), "Bank".into())
        .unwrap()
        .into_data();
    data.caption_presets = (0..64)
        .map(|id| SavedCaptionPreset {
            id: CaptionPresetId(id),
            name: format!("Look {id}").into(),
            style: style.clone(),
        })
        .collect();
    let project = Project::new(data.clone()).unwrap();
    assert_eq!(
        decode(&encode(&project, false).unwrap()).unwrap().project,
        project
    );
    assert_eq!(
        project.fork().unwrap().data().caption_presets,
        data.caption_presets
    );
    assert_eq!(
        project
            .for_scene(project::SceneId(0))
            .unwrap()
            .data()
            .caption_presets,
        data.caption_presets
    );
    for case in ["65", "id", "name", "nfd", "style", "nan"] {
        let mut invalid = data.clone();
        match case {
            "65" => invalid.caption_presets.push(SavedCaptionPreset {
                id: CaptionPresetId(65),
                name: "Extra".into(),
                style: style.clone(),
            }),
            "id" => invalid.caption_presets[1].id = CaptionPresetId(0),
            "name" => invalid.caption_presets[1].name = "Look 0".into(),
            "nfd" => {
                invalid.caption_presets[0].name = "Tiếng Việt".into();
                invalid.caption_presets[1].name = "Tie\u{302}\u{301}ng Vie\u{323}\u{302}t".into();
            }
            "style" => invalid.caption_presets[0].style.font_family = " ".into(),
            "nan" => invalid.caption_presets[0].style.font_size_ratio = f32::NAN,
            _ => unreachable!(),
        }
        assert!(Project::new(invalid).is_err(), "{case}");
    }
    for (name, valid) in [
        ("".into(), false),
        (" ".into(), false),
        (" leading".into(), false),
        ("trailing\u{a0}".into(), false),
        ("a\nb".into(), false),
        ("a\0b".into(), false),
        ("x".repeat(64), true),
        ("x".repeat(65), false),
        ("😀".repeat(64), true),
        ("😀".repeat(65), false),
        ("👨‍👩‍👧‍👦".repeat(10), true),
        ("👨‍👩‍👧‍👦".repeat(11), false),
    ] {
        let mut candidate = data.clone();
        candidate.caption_presets[0].name = name.into();
        assert_eq!(Project::new(candidate).is_ok(), valid);
    }
    let mut case_sensitive = data.clone();
    case_sensitive.caption_presets[0].name = "Look".into();
    case_sensitive.caption_presets[1].name = "look".into();
    Project::new(case_sensitive).unwrap(); // Equal style values under distinct names are valid.
    let encoded =
        serde_json::from_slice::<serde_json::Value>(&encode(&project, false).unwrap()).unwrap();
    for case in [
        "missing",
        "unknown",
        "entry-unknown",
        "style-unknown",
        "name-type",
        "style-missing",
    ] {
        let mut value = encoded.clone();
        match case {
            "missing" => {
                value["project"]
                    .as_object_mut()
                    .unwrap()
                    .remove("caption_presets");
            }
            "unknown" => value["project"]["extra"] = 0.into(),
            "entry-unknown" => value["project"]["caption_presets"][0]["extra"] = 0.into(),
            "style-unknown" => value["project"]["caption_presets"][0]["style"]["extra"] = 0.into(),
            "name-type" => value["project"]["caption_presets"][0]["name"] = 5.into(),
            "style-missing" => {
                value["project"]["caption_presets"][0]
                    .as_object_mut()
                    .unwrap()
                    .remove("style");
            }
            _ => unreachable!(),
        }
        assert!(
            decode(&serde_json::to_vec(&value).unwrap()).is_err(),
            "{case}"
        );
    }
    for (version, old) in [(SCHEMA_VERSION - 1, true), (SCHEMA_VERSION + 1, false)] {
        let mut value = encoded.clone();
        value["schema_version"] = version.into();
        value["project"] = serde_json::Value::Null;
        let error = decode(&serde_json::to_vec(&value).unwrap()).unwrap_err();
        assert!(if old {
            matches!(error, Error::TooOld { .. })
        } else {
            matches!(error, Error::TooNew { .. })
        });
    }
}
pub(crate) fn range(start: i64, end: i64) -> SourceRange {
    SourceRange::new(Tick(start), Tick(end)).unwrap()
}
/// `crop.x` / `crop.y` stepping channels (tick, value) plus a held `crop.auto` flag.
fn crop_channels(x: &[(i64, f64)], y: &[(i64, f64)]) -> Animations {
    let scalar = |keys: &[(i64, f64)]| {
        let keys = keys
            .iter()
            .map(|(t, v)| {
                let mut key = Keyframe::new(Tick(*t), ParamValue::Scalar(*v));
                key.segment = Segment::Step;
                key
            })
            .collect();
        Param::Value(
            Channel::new(
                ParamValue::Scalar(0.0),
                keys,
                Extrapolation::Hold,
                Extrapolation::Hold,
            )
            .unwrap(),
        )
    };
    let mut animations = Animations::default();
    for (path, param) in [
        (BuiltinPath::CropX, scalar(x)),
        (BuiltinPath::CropY, scalar(y)),
        (
            BuiltinPath::CropAuto,
            Param::new(ParamValue::Bool(true)).unwrap(),
        ),
    ] {
        animations.set(AnimationPath::from(path), param).unwrap();
    }
    animations
}
pub(crate) fn empty() -> Project {
    Project::empty(ProjectId(1), "Dự án Việt Nam".into()).unwrap()
}
pub(crate) fn source(path: PathBuf) -> SourceAsset {
    testkit::source::asset(3, path)
        .format("mp4")
        .duration(Tick(1000))
        .hash([1; 32])
        .stream(
            testkit::source::video()
                .time_base(TimeBase::new(1, 30000).unwrap())
                .duration(Tick(1000))
                .start(Tick(0))
                .frame_rate(FrameRate::FPS_29_97)
                .build(),
        )
        .stream(
            testkit::source::audio()
                .index(1)
                .time_base(TimeBase::new(1, 48000).unwrap())
                .start(Tick(25))
                .padding(1024, 0)
                .build(),
        )
        .build()
}
fn item(id: u64, start: i64) -> Item {
    Item {
        hidden: false,
        blend_mode: Default::default(),
        masks: vec![Mask::new(
            "glyphs",
            MaskShape::Text {
                content: "Mask".into(),
                font_size: 20.0,
                font: "Lobster".into(),
                weight: 600,
                italic: true,
                letter_spacing: 2.0,
                line_height: 30.0,
            },
        )],
        retime: Default::default(),
        effects: Vec::new(),
        animations: Default::default(),
        id: ItemId(id),
        source_asset: None,
        source_range: range(0, 10),
        start: Tick(start),
        content: ItemParams::Text(TextParams {
            content: "Tiếng Việt".into(),
            font: "Be Vietnam Pro".into(),
            font_size: 32.0,
            weight: 700,
            italic: true,
            letter_spacing: -1.5,
            line_height: 52.0,
            color: Color([255; 4]),
            align: project::TextAlign::Left,
            decoration: project::TextDecoration::Underline,
            background: Some(project::TextBackground {
                color: Color([10, 20, 30, 40]),
                padding_x: 14.0,
                padding_y: 9.0,
                offset_x: -30.5,
                offset_y: 12.0,
                radius: 20.0,
            }),
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
        word_timing: captions::CueWordTiming::Unverified,
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
        word_ranges: Vec::new(),
        style: None,
    };
    let contents = [
        ItemParams::Video(VideoParams {
            fades: Default::default(),
            source_audio_enabled: true,
            volume: 0.0,
            muted: false,
            crop: Some(Crop {
                x: 0.1,
                y: 0.2,
                width: 0.5,
                height: 0.5,
            }),
        }),
        ItemParams::Audio(AudioParams {
            fades: project::Fades {
                fade_in: project::Fade {
                    duration: Tick(3),
                    offset: Tick(1),
                    curve: project::FadeCurve::Linear,
                },
                fade_out: project::Fade {
                    duration: Tick(4),
                    offset: Tick(2),
                    curve: project::FadeCurve::Exponential,
                },
            },
            volume: -6.0,
            muted: true,
            source: Default::default(),
        }),
        item(0, 0).content,
        ItemParams::Caption(CaptionParams {
            word_highlight: false,
            cues: CueTrack {
                styles: Vec::new(),
                cues: vec![cue],
            },
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
                hidden: false,
                blend_mode: Default::default(),
                retime: Default::default(),
                masks: Vec::new(),
                effects: Vec::new(),
                animations: if matches!(content, ItemParams::Video(_)) {
                    crop_channels(&[(0, 0.1), (10, 0.5)], &[(0, 0.0), (10, 0.5)])
                } else {
                    Default::default()
                },
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
    let mut d = base.clone();
    let caption = d
        .scene_mut()
        .timeline
        .iter_mut()
        .flat_map(|track| &mut track.items)
        .find_map(|item| match &mut item.content {
            ItemParams::Caption(params) => Some(params),
            _ => None,
        })
        .unwrap();
    caption.cues.cues[0].style = Some(CueStyleId(0));
    cases.push(d);
    for (width, height) in [(0, 360), (640, 0), (u32::MAX, u32::MAX)] {
        let mut d = base.clone();
        d.original_canvas_size = Some(project::CanvasSize { width, height });
        cases.push(d);
    }
    for (width, height) in [(0, 360), (640, 0), (u32::MAX, u32::MAX)] {
        let mut d = base.clone();
        d.canvas.last_custom_size = project::CanvasSize { width, height };
        cases.push(d);
    }
    let mut d = base.clone();
    d.canvas.preset = Some(project::AspectPreset::Original);
    d.original_canvas_size = None;
    cases.push(d);
    let mut d = base.clone();
    d.canvas.fps = FrameRate::new(17, 1).unwrap();
    cases.push(d);
    let mut d = base.clone();
    d.canvas.fps = FrameRate::new(1, i64::MAX).unwrap();
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
        fades: Default::default(),
        volume: f32::NAN,
        muted: false,
        source: Default::default(),
    });
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
        fades: project::Fades {
            fade_in: project::Fade {
                duration: Tick(-1),
                offset: Tick::ZERO,
                curve: project::FadeCurve::default(),
            },
            fade_out: project::Fade::default(),
        },
        volume: 0.0,
        muted: false,
        source: Default::default(),
    });
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
        fades: project::Fades {
            fade_in: project::Fade::default(),
            fade_out: project::Fade {
                duration: Tick(5),
                offset: Tick(6),
                curve: project::FadeCurve::default(),
            },
        },
        volume: 0.0,
        muted: false,
        source: Default::default(),
    });
    cases.push(d);
    type Corrupt = fn(&mut TextParams);
    let text_style: [Corrupt; 16] = [
        |p| p.background.as_mut().unwrap().padding_x = -1.0,
        |p| p.background.as_mut().unwrap().padding_y = 128.5,
        |p| p.background.as_mut().unwrap().radius = f32::NAN,
        |p| p.background.as_mut().unwrap().offset_x = project::TextBackground::MAX_OFFSET + 1.0,
        |p| p.background.as_mut().unwrap().offset_y = f32::NEG_INFINITY,
        |p| p.font_size = f32::INFINITY,
        |p| p.font = "  ".into(),
        |p| p.weight = 0,
        |p| p.weight = 901,
        |p| p.letter_spacing = f32::NAN,
        |p| p.letter_spacing = project::MAX_LETTER_SPACING + 1.0,
        |p| p.line_height = -1.0,
        |p| p.line_height = f32::INFINITY,
        |p| p.line_height = project::MAX_LINE_HEIGHT + 1.0,
        |p| p.font = "x".repeat(257).into(),
        |p| p.letter_spacing = f32::NEG_INFINITY,
    ];
    for corrupt in text_style {
        let mut d = base.clone();
        let title = d
            .scene_mut()
            .timeline
            .iter_mut()
            .flat_map(|track| &mut track.items)
            .find(|item| matches!(item.content, ItemParams::Text(_)))
            .unwrap();
        if let ItemParams::Text(params) = &mut title.content {
            corrupt(params);
        }
        cases.push(d);
    }
    // Text masks share the same typography limits.
    type CorruptMask = fn(&mut project::Mask);
    let text_masks: [CorruptMask; 3] = [
        |m| {
            if let project::MaskShape::Text { weight, .. } = &mut m.shape {
                *weight = 0;
            }
        },
        |m| {
            if let project::MaskShape::Text { letter_spacing, .. } = &mut m.shape {
                *letter_spacing = f32::NAN;
            }
        },
        |m| {
            if let project::MaskShape::Text { font, .. } = &mut m.shape {
                *font = "".into();
            }
        },
    ];
    for corrupt in text_masks {
        let mut d = base.clone();
        let item = d
            .scene_mut()
            .timeline
            .iter_mut()
            .flat_map(|track| &mut track.items)
            .find(|item| matches!(item.content, ItemParams::Text(_)))
            .unwrap();
        let mut mask = project::Mask::new("text", project::MaskShape::text("Mask", 24.0));
        corrupt(&mut mask);
        item.masks.push(mask);
        cases.push(d);
    }
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
    type CorruptCrop = fn(&mut project::Crop);
    let crops: [CorruptCrop; 7] = [
        |c| c.width = 0.0,
        |c| c.height = -0.1,
        |c| c.x = 0.75,
        |c| c.y = 0.75,
        |c| c.x = -0.01,
        |c| c.width = f64::NAN,
        |c| c.height = f64::INFINITY,
    ];
    for corrupt in crops {
        let mut d = base.clone();
        let item = d
            .scene_mut()
            .timeline
            .iter_mut()
            .flat_map(|track| &mut track.items)
            .find(|item| matches!(item.content, ItemParams::Video(_)))
            .unwrap();
        if let ItemParams::Video(params) = &mut item.content {
            let mut crop = project::Crop {
                x: 0.25,
                y: 0.25,
                width: 0.5,
                height: 0.5,
            };
            corrupt(&mut crop);
            params.crop = Some(crop);
        }
        cases.push(d);
    }
    // Crop channels need a static crop and stay inside `[0, 1 - size]`.
    for (crop, x, y) in [
        (None, vec![(0, 0.1)], vec![(0, 0.2)]),
        (Some(0.5), vec![(0, 0.6)], vec![(0, 0.2)]),
        (Some(0.5), vec![(0, 0.1)], vec![(0, -0.01)]),
        (Some(0.5), vec![(0, 0.1), (5, 0.5001)], vec![(0, 0.2)]),
    ] {
        let mut d = base.clone();
        let item = d
            .scene_mut()
            .timeline
            .iter_mut()
            .flat_map(|track| &mut track.items)
            .find(|item| matches!(item.content, ItemParams::Video(_)))
            .unwrap();
        if let ItemParams::Video(params) = &mut item.content {
            params.crop = crop.map(|size| Crop {
                x: 0.0,
                y: 0.0,
                width: size,
                height: size,
            });
        }
        item.animations = crop_channels(&x, &y);
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
fn aspect_presets_use_the_reference_pixel_sizes_and_resolve_original() {
    assert_eq!(
        AspectPreset::Landscape.size(),
        Some(CanvasSize {
            width: 1920,
            height: 1080
        })
    );
    assert_eq!(
        AspectPreset::Standard.size(),
        Some(CanvasSize {
            width: 1440,
            height: 1080
        })
    );
    assert_eq!(
        AspectPreset::Wide.size(),
        Some(CanvasSize {
            width: 1920,
            height: 960
        })
    );
    assert_eq!(
        AspectPreset::Portrait.size(),
        Some(CanvasSize {
            width: 1080,
            height: 1920
        })
    );
    assert_eq!(
        AspectPreset::Square.size(),
        Some(CanvasSize {
            width: 1080,
            height: 1080
        })
    );
    assert_eq!(
        AspectPreset::Tall.size(),
        Some(CanvasSize {
            width: 1080,
            height: 1440
        })
    );
    let original = CanvasSize {
        width: 640,
        height: 360,
    };
    assert_eq!(AspectPreset::Original.size(), None);
    assert_eq!(
        AspectPreset::Original.resolved(Some(original)),
        Some(original)
    );
    assert_eq!(AspectPreset::Original.resolved(None), None);
}
#[test]
fn a_pre_change_file_is_refused_without_losing_user_data() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pre-change.vibecut");
    let bytes = r#"{"format":"vibecut.project","schema_version":19,"generator":"test","project":{"name":"User project"}}"#;
    fs::write(&path, bytes).unwrap();
    assert!(matches!(
        load(&path),
        Err(Error::TooOld {
            found: 19,
            supported: SCHEMA_VERSION
        })
    ));
    assert_eq!(fs::read(&path).unwrap(), bytes.as_bytes());
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
        (9, "null", true),
        (10, "null", true),
        (SCHEMA_VERSION - 1, "null", true),
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
fn fades_without_an_offset_load_as_uncut_fades() {
    // A fade without `offset` is uncut.
    let fades: Fades = serde_json::from_str(
        r#"{"fade_in":{"duration":30,"curve":"linear"},"fade_out":{"duration":20,"curve":"equal_power"}}"#,
    )
    .unwrap();
    assert_eq!(fades.fade_in.offset, Tick::ZERO);
    assert_eq!(fades.fade_in.window(), Tick(30));
    assert_eq!(fades.fade_out.window(), Tick(20));
}
#[test]
fn hostile_json_is_bounded_and_errors_redact_text() {
    let huge = vec![b' '; (16 << 20) + 1];
    assert!(matches!(decode(&huge), Err(Error::TooLarge(_))));
    let deep = format!("{}0{}", "[".repeat(65), "]".repeat(65));
    assert!(matches!(decode(deep.as_bytes()), Err(Error::TooLarge(_))));
    let array = format!("[{}0]", "0,".repeat(MAX_SEPARATORS + 1));
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
        probe: media::SourceProbe {
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

#[test]
fn graphic_save_load_refuses_hostile_appearance_and_preserves_authoritative_channels() {
    let root = testkit::fixture::dir("graphic-project");
    let path = root.join("Hình.vibecut");
    for graphic in [GraphicParams::rectangle(), GraphicParams::ellipse()] {
        let mut data = empty().into_data();
        let mut graphic_item = item(70, 5);
        graphic_item.masks.clear();
        graphic_item.content = ItemParams::Graphic(graphic.clone());
        graphic_item
            .animations
            .set(
                AnimationPath::new("params.fill").unwrap(),
                Param::new(ParamValue::Color([0.1, 0.2, 0.3, 0.4])).unwrap(),
            )
            .unwrap();
        data.scene_mut().timeline.main.items.push(graphic_item);
        let p = Project::new(data.clone()).unwrap();
        save_fixture(&path, &p).unwrap();
        assert_eq!(load(&path).unwrap().project, p);
        let value = serde_json::to_value(&graphic).unwrap();
        for field in value["params"].as_object().unwrap().keys() {
            let mut bad = value.clone();
            bad["params"].as_object_mut().unwrap().remove(field);
            assert!(
                serde_json::from_value::<GraphicParams>(bad).is_err(),
                "missing {field}"
            );
        }
        for (key, value) in [
            ("unknown", serde_json::json!(0)),
            ("strokeAlign", serde_json::json!("diagonal")),
            ("fill", serde_json::json!([0, 0, 0])),
        ] {
            let mut bad = serde_json::to_value(&graphic).unwrap();
            bad["params"][key] = value;
            assert!(
                serde_json::from_value::<GraphicParams>(bad).is_err(),
                "{key}"
            );
        }
        assert!(serde_json::from_str::<GraphicParams>(r#"{"definitionId":"rectangle","params":{"fill":[0,0,0,0],"fill":[255,255,255,255],"stroke":[0,0,0,255],"strokeWidth":0,"strokeAlign":"center","cornerRadius":0}}"#).is_err());
        for (name, param) in [
            (
                "params.strokeWidth",
                Param::new(ParamValue::Scalar(64.01)).unwrap(),
            ),
            (
                "params.strokeWidth",
                Param::new(ParamValue::Text("2".into())).unwrap(),
            ),
            (
                "params.fill",
                Param::new(ParamValue::Text("#fff".into())).unwrap(),
            ),
            (
                "params.strokeAlign",
                Param::new(ParamValue::Text("diagonal".into())).unwrap(),
            ),
            (
                "params.unknown",
                Param::new(ParamValue::Scalar(0.)).unwrap(),
            ),
        ] {
            let mut bad = data.clone();
            bad.scene_mut().timeline.main.items[0]
                .animations
                .set(AnimationPath::new(name).unwrap(), param)
                .unwrap();
            assert!(Project::new(bad).is_err(), "{name}");
        }
        let mut bad = data.clone();
        bad.scene_mut().timeline.main.items[0].source_asset = Some(media::AssetId(99));
        assert!(Project::new(bad).is_err());
        for width in [-1., 64.001, f64::INFINITY, f64::NAN] {
            let mut bad = data.clone();
            if let ItemParams::Graphic(
                GraphicParams::Rectangle { stroke_width, .. }
                | GraphicParams::Ellipse { stroke_width, .. },
            ) = &mut bad.scene_mut().timeline.main.items[0].content
            {
                *stroke_width = width;
            }
            assert!(Project::new(bad).is_err());
        }
        let bytes = encode(&p, false).unwrap();
        let mut envelope: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        envelope["schema_version"] = serde_json::json!(SCHEMA_VERSION - 1);
        envelope["project"] = serde_json::json!({"hostile":"body"});
        assert!(matches!(
            decode(&serde_json::to_vec(&envelope).unwrap()),
            Err(Error::TooOld { .. })
        ));
        envelope["schema_version"] = serde_json::json!(SCHEMA_VERSION + 1);
        assert!(matches!(
            decode(&serde_json::to_vec(&envelope).unwrap()),
            Err(Error::TooNew { .. })
        ));
    }
    let ellipse = serde_json::json!({"definitionId":"ellipse","params":{"fill":[255,255,255,255],"stroke":[0,0,0,255],"strokeWidth":0,"strokeAlign":"center","cornerRadius":0}});
    assert!(serde_json::from_value::<GraphicParams>(ellipse).is_err());
}

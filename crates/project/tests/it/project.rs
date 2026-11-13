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
fn named_preset_banks_validate_every_entry_names_closed_fields_and_schema() {
    let style = captions::CaptionStyle {
        font: "Inter".into(),
        font_style: "SemiBold".into(),
        size: 54.0,
        color: [255, 255, 255, 180],
        outline: Some(captions::CaptionOutline {
            color: [0, 0, 0, 255],
            width: 2.0,
        }),
        background: None,
        position: captions::CaptionPosition::Bottom,
        safe_area_percent: 5.0,
        max_lines: 2,
        word_highlight_color: [255, 215, 0],
    };
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
            "style" => invalid.caption_presets[0].style.size = 1.0,
            "nan" => invalid.caption_presets[0].style.size = f32::NAN,
            _ => panic!("unknown case {case}"),
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
    // A text bank uses the same closed naming and identity rules over text styles.
    let mut styled = TextParams::new("Giữ chữ");
    styled.stroke = Some(project::TextStroke {
        color: Color([255, 0, 0, 255]),
        width: 3.0,
    });
    styled.shadow = Some(project::TextShadow {
        color: Color([0, 0, 0, 128]),
        offset_x: 4.0,
        offset_y: 5.0,
        blur: 2.0,
    });
    styled.glow = true;
    let text_style = TextStyle::from_params(&styled);
    data.text_presets = (0..64)
        .map(|id| SavedTextPreset {
            id: TextPresetId(id),
            name: format!("Style {id}").into(),
            style: text_style.clone(),
        })
        .collect();
    let project = Project::new(data.clone()).unwrap();
    assert_eq!(
        decode(&encode(&project, false).unwrap()).unwrap().project,
        project
    );
    assert_eq!(
        project.fork().unwrap().data().text_presets,
        data.text_presets
    );
    assert_eq!(
        project
            .for_scene(project::SceneId(0))
            .unwrap()
            .data()
            .text_presets,
        data.text_presets
    );
    for case in [
        "65", "id", "name", "nfd", "font", "size", "box", "stroke", "shadow",
    ] {
        let mut invalid = data.clone();
        match case {
            "65" => invalid.text_presets.push(SavedTextPreset {
                id: TextPresetId(65),
                name: "Extra".into(),
                style: text_style.clone(),
            }),
            "id" => invalid.text_presets[1].id = TextPresetId(0),
            "name" => invalid.text_presets[1].name = "Style 0".into(),
            "nfd" => {
                invalid.text_presets[0].name = "Tiếng Việt".into();
                invalid.text_presets[1].name = "Tie\u{302}\u{301}ng Vie\u{323}\u{302}t".into();
            }
            "font" => invalid.text_presets[0].style.font = " ".into(),
            "size" => invalid.text_presets[0].style.font_size = f32::NAN,
            "box" => {
                invalid.text_presets[0].style.background = Some(TextBackground {
                    padding_x: -1.0,
                    ..Default::default()
                });
            }
            "stroke" => {
                invalid.text_presets[0]
                    .style
                    .stroke
                    .get_or_insert_with(Default::default)
                    .width = project::TextStroke::MAX_WIDTH + 1.0;
            }
            "shadow" => {
                invalid.text_presets[0]
                    .style
                    .shadow
                    .get_or_insert_with(Default::default)
                    .blur = f32::NAN;
            }
            _ => panic!("unknown case {case}"),
        }
        assert!(Project::new(invalid).is_err(), "{case}");
    }
    let encoded_text =
        serde_json::from_slice::<serde_json::Value>(&encode(&project, false).unwrap()).unwrap();
    for case in [
        "missing",
        "entry-unknown",
        "style-unknown",
        "name-type",
        "style-missing",
    ] {
        let mut value = encoded_text.clone();
        match case {
            "missing" => {
                value["project"]
                    .as_object_mut()
                    .unwrap()
                    .remove("text_presets");
            }
            "entry-unknown" => value["project"]["text_presets"][0]["extra"] = 0.into(),
            "style-unknown" => value["project"]["text_presets"][0]["style"]["extra"] = 0.into(),
            "name-type" => value["project"]["text_presets"][0]["name"] = 5.into(),
            "style-missing" => {
                value["project"]["text_presets"][0]
                    .as_object_mut()
                    .unwrap()
                    .remove("style");
            }
            _ => panic!("unknown case {case}"),
        }
        assert!(
            decode(&serde_json::to_vec(&value).unwrap()).is_err(),
            "{case}"
        );
    }
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
            _ => panic!("unknown case {case}"),
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
#[test]
fn caption_selection_references_overrides_and_schema_round_trip() {
    let style = captions::CaptionStyle::default();
    let mut data = populated(std::env::temp_dir().join("caption-selection")).into_data();
    data.caption_presets = vec![SavedCaptionPreset {
        id: CaptionPresetId(7),
        name: "Look".into(),
        style: style.clone(),
    }];
    for item in data
        .scene_mut()
        .timeline
        .iter_mut()
        .flat_map(|track| &mut track.items)
    {
        if let ItemParams::Caption(params) = &mut item.content {
            params.preset = Some(CaptionPresetId(7));
            params.overrides = captions::CaptionOverrides {
                size: Some(72.0),
                outline: Some(None),
                max_lines: Some(3),
                ..Default::default()
            };
        }
    }
    let project = Project::new(data.clone()).unwrap();
    let bytes = encode(&project, false).unwrap();
    assert_eq!(decode(&bytes).unwrap().project, project);

    let mut unknown = data.clone();
    for item in unknown
        .scene_mut()
        .timeline
        .iter_mut()
        .flat_map(|track| &mut track.items)
    {
        if let ItemParams::Caption(params) = &mut item.content {
            params.preset = Some(CaptionPresetId(8));
        }
    }
    assert!(Project::new(unknown).is_err());

    let mut orphan = data.clone();
    for item in orphan
        .scene_mut()
        .timeline
        .iter_mut()
        .flat_map(|track| &mut track.items)
    {
        if let ItemParams::Caption(params) = &mut item.content {
            params.preset = None;
        }
    }
    assert!(Project::new(orphan).is_err());

    let mut hostile = data.clone();
    for item in hostile
        .scene_mut()
        .timeline
        .iter_mut()
        .flat_map(|track| &mut track.items)
    {
        if let ItemParams::Caption(params) = &mut item.content {
            params.overrides.size = Some(f32::NAN);
        }
    }
    assert!(Project::new(hostile).is_err());
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
    testkit::item::item(
        ItemId(id),
        ItemParams::Text(TextParams {
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
            case: project::TextCase::Title,
            stroke: Some(project::TextStroke {
                color: Color([200, 10, 20, 255]),
                width: 3.0,
            }),
            shadow: Some(project::TextShadow {
                color: Color([0, 0, 0, 128]),
                offset_x: 4.0,
                offset_y: 5.0,
                blur: 2.0,
            }),
            glow: true,
            background: Some(project::TextBackground {
                color: Color([10, 20, 30, 40]),
                padding_x: 14.0,
                padding_y: 9.0,
                offset_x: -30.5,
                offset_y: 12.0,
                radius: 20.0,
            }),
        }),
    )
    .source(range(0, 10))
    .start(Tick(start))
    .masks(vec![Mask::new(
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
    )])
    .build()
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
        style: None,
    };
    let contents = [
        ItemParams::Video(VideoParams {
            source_audio_enabled: true,
            crop: Some(Crop {
                x: 0.1,
                y: 0.2,
                width: 0.5,
                height: 0.5,
            }),
            fit: FitMode::Stretch,
            reduce_noise: true,
            ..VideoParams::plain()
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
            ..AudioParams::default()
        }),
        item(0, 0).content,
        ItemParams::Caption(CaptionParams {
            emphasis: Default::default(),
            word_highlight: false,
            preset: None,
            overrides: captions::CaptionOverrides::default(),
            cues: CueTrack {
                styles: Vec::new(),
                cues: vec![cue],
            },
        }),
        ItemParams::Overlay(OverlayParams {
            color: Color([40, 50, 60, 255]),
        }),
        ItemParams::Sticker(StickerParams::new("flags:vn", 640, 480)),
    ];
    for (i, content) in contents.into_iter().enumerate() {
        let source_asset = matches!(
            content,
            ItemParams::Video(_) | ItemParams::Audio(_) | ItemParams::Caption(_)
        )
        .then_some(AssetId(3));
        let video = matches!(content, ItemParams::Video(_));
        data.scene_mut().timeline.insert_track(Track {
            id: TrackId(i as u64 + 1),
            name: "Track".into(),
            kind: content.kind(),
            muted: true,
            hidden: true,
            locked: true,
            items: vec![
                testkit::item::item(ItemId(i as u64), content)
                    .source_asset(source_asset)
                    .source(range(0, 10))
                    .start(Tick(5))
                    .animations(if video {
                        crop_channels(&[(0, 0.1), (10, 0.5)], &[(0, 0.0), (10, 0.5)])
                    } else {
                        Default::default()
                    })
                    .build(),
            ],
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
            id: TrackId(index as u64 + 7),
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
fn reflection_channels_hold_step_booleans_on_visual_items_and_survive_save() {
    let horizontal = AnimationPath::new("transform.flipX").unwrap();
    let vertical = AnimationPath::new("transform.flipY").unwrap();
    assert_eq!(horizontal.builtin(), Some(BuiltinPath::TransformFlipX));
    assert_eq!(vertical.builtin(), Some(BuiltinPath::TransformFlipY));
    for axis in FlipAxis::ALL {
        assert_eq!(
            AnimationPath::from(axis.builtin()).builtin(),
            Some(axis.builtin())
        );
    }
    // Reflection holds a Boolean; scale magnitudes stay numeric.
    let mut animations = Animations::default();
    assert!(
        animations
            .set(
                horizontal.clone(),
                Param::new(ParamValue::Scalar(1.0)).unwrap()
            )
            .is_err()
    );
    assert!(
        animations
            .set(
                vertical.clone(),
                Param::new(ParamValue::Bool(true)).unwrap()
            )
            .is_ok()
    );
    assert!(
        FlipAxis::Vertical
            .value_at(&animations, Tick::ZERO)
            .unwrap()
    );
    assert!(
        !FlipAxis::Horizontal
            .value_at(&animations, Tick::ZERO)
            .unwrap()
    );
    // A fresh key is Linear and a discrete channel must hold.
    let mut param = Param::new(ParamValue::Bool(false)).unwrap();
    assert!(
        param
            .channel_mut(None)
            .unwrap()
            .put(Keyframe::new(Tick::ZERO, ParamValue::Bool(true)))
            .is_err()
    );
    param
        .channel_mut(None)
        .unwrap()
        .put({
            let mut key = Keyframe::new(Tick::ZERO, ParamValue::Bool(true));
            key.segment = project::Segment::Step;
            key
        })
        .unwrap();
    assert!(animations.set(horizontal.clone(), param).is_ok());

    let dir = tempfile::tempdir().unwrap();
    let base = populated(dir.path().join("clip.mp4"));
    let unacceptable = |ids: &[u64]| {
        let mut data = base.data().clone();
        for id in ids {
            let item = data
                .scene_mut()
                .timeline
                .iter_mut()
                .flat_map(|track| &mut track.items)
                .find(|item| item.id == ItemId(*id))
                .unwrap();
            item.animations
                .set(
                    horizontal.clone(),
                    Param::new(ParamValue::Bool(true)).unwrap(),
                )
                .unwrap();
        }
        Project::new(data)
    };
    // Audio and captions own no reflected layer; video, text and overlay do.
    for id in [1, 3] {
        assert!(matches!(
            unacceptable(&[id]),
            Err(Error::Invalid("flip on unsupported item"))
        ));
    }
    for id in [0, 2, 4] {
        assert!(unacceptable(&[id]).is_ok());
    }
    for pretty in [false, true] {
        let mut data = base.data().clone();
        for id in [0, 2, 4] {
            let item = data
                .scene_mut()
                .timeline
                .iter_mut()
                .flat_map(|track| &mut track.items)
                .find(|item| item.id == ItemId(id))
                .unwrap();
            for axis in FlipAxis::ALL {
                item.animations
                    .set(
                        AnimationPath::from(axis.builtin()),
                        Param::new(ParamValue::Bool(axis == FlipAxis::Horizontal)).unwrap(),
                    )
                    .unwrap();
            }
        }
        let project = Project::new(data).unwrap();
        let loaded = decode(&encode(&project, pretty).unwrap()).unwrap();
        assert_eq!(loaded.schema_version, SCHEMA_VERSION);
        assert_eq!(loaded.project, project);
        for id in [0, 2, 4] {
            let item = loaded
                .project
                .data()
                .scene()
                .timeline
                .iter()
                .flat_map(|track| &track.items)
                .find(|item| item.id == ItemId(id))
                .unwrap();
            assert!(
                FlipAxis::Horizontal
                    .value_at(&item.animations, Tick::ZERO)
                    .unwrap()
            );
            assert!(
                !FlipAxis::Vertical
                    .value_at(&item.animations, Tick::ZERO)
                    .unwrap()
            );
        }
    }
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
        volume: f32::NAN,
        ..AudioParams::default()
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
        ..AudioParams::default()
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
        ..AudioParams::default()
    });
    cases.push(d);
    type Corrupt = fn(&mut TextParams);
    let text_style: [Corrupt; 19] = [
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
        |p| {
            p.stroke.get_or_insert_with(Default::default).width =
                project::TextStroke::MAX_WIDTH + 1.0;
        },
        |p| p.shadow.get_or_insert_with(Default::default).offset_x = f32::NAN,
        |p| {
            p.shadow.get_or_insert_with(Default::default).blur =
                project::TextShadow::MAX_BLUR + 1.0;
        },
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
    // Sticker rows: a mask, malformed ids, sizes outside 1..=16384 and a media reference.
    for edit in 0..6 {
        let mut candidate = base.clone();
        let item = candidate
            .scene_mut()
            .timeline
            .iter_mut()
            .flat_map(|track| &mut track.items)
            .find(|item| matches!(item.content, ItemParams::Sticker(_)))
            .unwrap();
        match (edit, &mut item.content) {
            (0, _) => item.masks.push(Mask::new("m", MaskShape::Rectangle)),
            (1, ItemParams::Sticker(p)) => p.sticker = "flags".into(),
            (2, ItemParams::Sticker(p)) => p.sticker = "flags:a:b".into(),
            (3, ItemParams::Sticker(p)) => p.width = 0,
            (4, ItemParams::Sticker(p)) => p.height = StickerParams::MAX_SIDE + 1,
            (_, _) => item.source_asset = Some(AssetId(3)),
        }
        cases.push(candidate);
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

fn music_record() -> LibraryMusic {
    LibraryMusic {
        id: "track-1".into(),
        title: "Night Walk".into(),
        author: "Example Artist".into(),
        licence: "CC BY 4.0".into(),
        licence_url: "https://creativecommons.org/licenses/by/4.0/".into(),
        attribution: "Night Walk by Example Artist".into(),
        source_url: "https://source.example/track-1".into(),
        audio_url: "https://cdn.example/track-1.ogg".into(),
        duration_ms: 1,
    }
}

fn music_candidate(music: LibraryMusic) -> ProjectData {
    let mut data = empty().into_data();
    data.assets
        .push(source(std::env::temp_dir().join("music-source.mp4")));
    data.scene_mut().timeline.insert_track(Track {
        id: TrackId(2),
        name: "Music".into(),
        kind: TrackKind::Audio,
        muted: false,
        hidden: false,
        locked: false,
        items: vec![
            testkit::item::item(
                ItemId(1),
                ItemParams::Audio(AudioParams {
                    source: AudioSource::Music { music },
                    ..AudioParams::default()
                }),
            )
            .asset(AssetId(3))
            .source(range(0, 10))
            .build(),
        ],
    });
    data
}

#[test]
fn music_ingestion_rejects_hostile_provenance() {
    use serde_json::json;
    let mut cases = vec![
        ("id", json!("")),
        ("id", json!("x".repeat(129))),
        ("id", json!("bad/id")),
        ("id", json!("é")),
        ("licence", json!("CC BY-NC 4.0")),
        ("licence", json!("CC BY-SA 4.0")),
        ("licence", json!("CC BY-ND 4.0")),
        ("licence", json!("Public Domain Mark 1.0")),
        ("licence", json!("Unknown")),
        ("licence", json!("CC BY")),
        ("licence", json!("")),
        (
            "licence_url",
            json!("https://creativecommons.org/licenses/by-nc/4.0/"),
        ),
        (
            "licence_url",
            json!("https://creativecommons.org/publicdomain/zero/1.0/"),
        ),
        (
            "licence_url",
            json!("https://creativecommons.org/licenses/by/3.0/"),
        ),
        ("duration_ms", json!(0)),
        ("duration_ms", json!(86_400_001)),
    ];
    for field in ["title", "author", "attribution"] {
        for value in [
            String::new(),
            "x".repeat(4097),
            "line\nfeed".into(),
            "control\u{85}".into(),
        ] {
            cases.push((field, json!(value)));
        }
    }
    for field in ["source_url", "audio_url"] {
        for value in [
            "http://example.com/audio",
            "https://",
            "https:///audio",
            "https://?query=1",
            "https://secret-user-marker@example.com/audio",
            "https://user:password@example.com/audio",
            "https://bad..example/audio",
            "https://-bad.example/audio",
            "https://[invalid]/audio",
            "https://example.com:99999/audio",
            "https://example.com:/audio",
            "https://example.com/a b",
            "https://example.com/\\audio",
            "https://example.com/\u{85}",
            "https://example.com/%zz",
        ] {
            cases.push((field, json!(value)));
        }
        cases.push((
            field,
            json!(format!("https://example.com/{}", "x".repeat(8192))),
        ));
    }
    let mut accepted = Vec::new();
    for (index, (field, value)) in cases.into_iter().enumerate() {
        let mut raw = serde_json::to_value(music_record()).unwrap();
        raw[field] = value;
        let music: LibraryMusic = serde_json::from_value(raw).unwrap();
        let candidate = music_candidate(music);
        let body = serde_json::to_value(&candidate).unwrap();
        let envelope = json!({
            "format": FORMAT_ID, "schema_version": SCHEMA_VERSION,
            "generator": "music-test", "project": body,
        });
        let model = Project::new(candidate);
        let serde = serde_json::from_value::<Project>(body);
        let decoded = decode(&serde_json::to_vec(&envelope).unwrap());
        for (path, passed) in [
            ("model", model.is_ok()),
            ("serde", serde.is_ok()),
            ("envelope", decoded.is_ok()),
        ] {
            if passed {
                accepted.push(format!("{index}:{field}:{path}"));
            }
        }
        for error in [
            model.err().map(|e| e.to_string()),
            serde.err().map(|e| e.to_string()),
            decoded.err().map(|e| e.to_string()),
        ]
        .into_iter()
        .flatten()
        {
            assert!(!error.contains("secret-user-marker"));
            assert!(!error.contains("password"));
        }
    }
    assert!(
        accepted.is_empty(),
        "malformed provenance admitted: {accepted:?}"
    );
}

#[test]
fn music_schema_refuses_pre_music_and_future_before_interpreting_body() {
    for (version, old) in [(SCHEMA_VERSION - 1, true), (SCHEMA_VERSION + 1, false)] {
        let raw = serde_json::json!({
            "format": FORMAT_ID, "schema_version": version,
            "generator": "music-test", "project": null,
        });
        let error = decode(&serde_json::to_vec(&raw).unwrap()).unwrap_err();
        assert!(
            match error {
                Error::TooOld { found, supported } => {
                    old && found == version && supported == SCHEMA_VERSION
                }
                Error::TooNew { found, supported } => {
                    !old && found == version && supported == SCHEMA_VERSION
                }
                _ => false,
            },
            "wrong schema refusal for {version}"
        );
    }
}

#[test]
fn music_licences_require_the_decided_global_identity_and_deed() {
    // Expected identities are the canonical URLs published on the CC deeds.
    let identities = [
        (
            "CC0 1.0",
            "https://creativecommons.org/publicdomain/zero/1.0/",
        ),
        ("CC BY 1.0", "https://creativecommons.org/licenses/by/1.0/"),
        ("CC BY 2.0", "https://creativecommons.org/licenses/by/2.0/"),
        ("CC BY 2.5", "https://creativecommons.org/licenses/by/2.5/"),
        ("CC BY 3.0", "https://creativecommons.org/licenses/by/3.0/"),
        ("CC BY 4.0", "https://creativecommons.org/licenses/by/4.0/"),
    ];
    for (name, deed) in identities {
        for spelling in [deed, deed.strip_suffix('/').unwrap()] {
            let identity = MusicLicence::parse(name, spelling).unwrap();
            assert_eq!(identity.display_name(), name);
            assert_eq!(identity.deed_url(), deed);
            let mut music = music_record();
            music.licence = name.into();
            music.licence_url = spelling.into();
            let expected = music.clone();
            let project = Project::new(music_candidate(music)).unwrap();
            let loaded = decode(&encode(&project, false).unwrap()).unwrap();
            let serialized = serde_json::to_value(loaded.project).unwrap();
            assert_eq!(
                serialized
                    .pointer("/scenes/0/timeline/audio/0/items/0/content/params/source/music")
                    .unwrap(),
                &serde_json::to_value(expected).unwrap(),
            );
        }
        for (other, wrong_deed) in identities {
            if other != name {
                assert!(matches!(
                    MusicLicence::parse(name, wrong_deed),
                    Err(Error::Invalid(_))
                ));
            }
        }
    }
    for name in [
        "CC BY-NC 4.0",
        "CC BY-SA 4.0",
        "CC BY-ND 4.0",
        "Public Domain Mark 1.0",
        "Unknown",
        "CC BY",
        "CC0",
        "CC BY 5.0",
        "CC BY 3.0 US",
        "cc by 4.0",
        " CC BY 4.0",
    ] {
        assert!(matches!(
            MusicLicence::parse(name, identities[5].1),
            Err(Error::Unsupported(_))
        ));
    }
    for deed in [
        "http://creativecommons.org/licenses/by/4.0/",
        "https://creativecommons.org.evil.example/licenses/by/4.0/",
        "https://user@creativecommons.org/licenses/by/4.0/",
        "https://creativecommons.org:443/licenses/by/4.0/",
        "https://creativecommons.org/licenses/by/4.0/?q=1",
        "https://creativecommons.org/licenses/by/4.0/#deed",
        "https://creativecommons.org/licenses/by/4.0//",
        "https://creativecommons.org/licenses/by/4.0/deed.en",
        "https://creativecommons.org/licenses/by/3.0/us/",
        "https://creativecommons.org/licenses/by/4.0/../3.0/",
        "https://creativecommons.org/licenses/%62y/4.0/",
        "https://CREATIVECOMMONS.ORG/licenses/by/4.0/",
        "",
    ] {
        assert!(matches!(
            MusicLicence::parse("CC BY 4.0", deed),
            Err(Error::Invalid(_))
        ));
    }
    assert!(matches!(
        MusicLicence::parse(&"x".repeat(4097), identities[5].1),
        Err(Error::TooLarge(_))
    ));
    assert!(matches!(
        MusicLicence::parse("CC BY 4.0", &"x".repeat(8193)),
        Err(Error::TooLarge(_))
    ));
}

#[test]
fn music_provenance_round_trips_exactly_at_admission_bounds() {
    for duration_ms in [1, 86_400_000] {
        let mut music = music_record();
        music.id = "-_.:".repeat(32).into();
        music.title = "é".repeat(2048).into();
        music.author = "a".repeat(4096).into();
        music.attribution = "b".repeat(4096).into();
        music.licence_url = "https://creativecommons.org/licenses/by/4.0".into();
        for url in [&mut music.source_url, &mut music.audio_url] {
            *url = format!(
                "https://example.com/{}",
                "x".repeat(8192 - "https://example.com/".len())
            );
        }
        music.duration_ms = duration_ms;
        music.validate().unwrap();
        let project = Project::new(music_candidate(music)).unwrap();
        let bytes = encode(&project, false).unwrap();
        let loaded = decode(&bytes).unwrap();
        assert_eq!(loaded.schema_version, SCHEMA_VERSION);
        assert_eq!(loaded.project, project);
    }
    for url in [
        "https://example.com:443/a%20b?q=1#part",
        "https://127.0.0.1/audio",
        "https://[::1]:8443/audio",
        "https://localhost/audio",
    ] {
        let mut music = music_record();
        music.source_url = url.into();
        music.audio_url = url.into();
        Project::new(music_candidate(music)).unwrap();
    }
    for (field, value) in [
        ("id", "x".repeat(129)),
        ("title", "é".repeat(2049)),
        ("author", "a".repeat(4097)),
        ("attribution", "b".repeat(4097)),
        (
            "source_url",
            format!(
                "https://example.com/{}",
                "x".repeat(8193 - "https://example.com/".len())
            ),
        ),
        (
            "audio_url",
            format!(
                "https://example.com/{}",
                "x".repeat(8193 - "https://example.com/".len())
            ),
        ),
    ] {
        let mut raw = serde_json::to_value(music_record()).unwrap();
        raw[field] = value.into();
        let music: LibraryMusic = serde_json::from_value(raw).unwrap();
        assert!(
            matches!(music.validate(), Err(Error::TooLarge(_))),
            "{field}"
        );
    }
}

#[test]
fn music_serde_normalizes_provenance_text_and_rejects_unknown_fields() {
    let mut raw = serde_json::to_value(music_candidate(music_record())).unwrap();
    let music = raw
        .pointer_mut("/scenes/0/timeline/audio/0/items/0/content/params/source/music")
        .unwrap();
    for field in ["title", "author", "attribution"] {
        music[field] = "Tie\u{302}\u{301}ng Vie\u{323}\u{302}t".into();
    }
    let project: Project = serde_json::from_value(raw.clone()).unwrap();
    let serialized = serde_json::to_value(&project).unwrap();
    let stored = serialized
        .pointer("/scenes/0/timeline/audio/0/items/0/content/params/source/music")
        .unwrap();
    for field in ["title", "author", "attribution"] {
        assert_eq!(stored[field], "Tiếng Việt");
    }
    assert_eq!(
        decode(&encode(&project, false).unwrap()).unwrap().project,
        project
    );
    raw.pointer_mut("/scenes/0/timeline/audio/0/items/0/content/params/source/music")
        .unwrap()["licence_identity"] = "second shape".into();
    assert!(serde_json::from_value::<Project>(raw).is_err());
}
#[test]
fn music_credits_cover_used_project_items_and_dedupe_full_records() {
    let music = music_record();
    let mut other = music.clone();
    other.id = "track-2".into();
    other.attribution = "Other track".into();
    let mut data = music_candidate(music.clone());
    let mut second = data.scene().timeline.audio[0].items[0].clone();
    second.id = ItemId(2);
    second.start = Tick(10);
    second.hidden = true;
    let ItemParams::Audio(params) = &mut second.content else {
        panic!("audio item");
    };
    params.source = AudioSource::Music {
        music: other.clone(),
    };
    data.scene_mut().timeline.audio[0].items.push(second);
    let mut duplicate = data.scene().timeline.audio[0].items[0].clone();
    duplicate.id = ItemId(3);
    duplicate.start = Tick(20);
    data.scene_mut().timeline.audio[0].items.push(duplicate);
    let project = Project::new(data).unwrap();
    assert_eq!(
        project.music_credits().unwrap(),
        vec![music.attribution_record(), other.attribution_record()]
    );
}

#[test]
fn music_credits_refuse_more_than_one_mib_without_partial_output() {
    let mut data = music_candidate(music_record());
    let base = data.scene().timeline.audio[0].items[0].clone();
    for index in 0..90u64 {
        let mut item = base.clone();
        item.id = ItemId(index + 2);
        item.start = Tick(i64::try_from(index + 1).unwrap() * 10);
        let ItemParams::Audio(params) = &mut item.content else {
            panic!("audio item");
        };
        let AudioSource::Music { music } = &mut params.source else {
            panic!("music source");
        };
        music.attribution = "a".repeat(4096).into();
        music.source_url = format!("https://example.com/{index}/{}", "x".repeat(8000));
        data.scene_mut().timeline.audio[0].items.push(item);
    }
    let project = Project::new(data).unwrap();
    assert!(matches!(
        project.music_credits(),
        Err(Error::TooLarge("music credits"))
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
    for graphic in [
        GraphicParams::rectangle(),
        GraphicParams::ellipse(),
        GraphicParams::polygon(),
        GraphicParams::star(),
    ] {
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
                | GraphicParams::Ellipse { stroke_width, .. }
                | GraphicParams::Polygon { stroke_width, .. }
                | GraphicParams::Star { stroke_width, .. },
            ) = &mut bad.scene_mut().timeline.main.items[0].content
            {
                *stroke_width = width;
            }
            assert!(Project::new(bad).is_err());
        }
        // Radial geometry: sides/points stay in 3..=12, depth in 1..=99, only on their own definition.
        for (name, value) in [
            ("params.sides", 2.),
            ("params.sides", 13.),
            ("params.points", 2.),
            ("params.points", 13.),
            ("params.depth", 0.5),
            ("params.depth", 100.),
        ] {
            let mut bad = data.clone();
            bad.scene_mut().timeline.main.items[0]
                .animations
                .set(
                    AnimationPath::new(name).unwrap(),
                    Param::new(ParamValue::Scalar(value)).unwrap(),
                )
                .unwrap();
            assert!(Project::new(bad).is_err(), "{name} {value}");
        }
        for (name, own) in [
            (
                "params.sides",
                matches!(graphic, GraphicParams::Polygon { .. }),
            ),
            (
                "params.points",
                matches!(graphic, GraphicParams::Star { .. }),
            ),
            (
                "params.depth",
                matches!(graphic, GraphicParams::Star { .. }),
            ),
        ] {
            let mut good = data.clone();
            good.scene_mut().timeline.main.items[0]
                .animations
                .set(
                    AnimationPath::new(name).unwrap(),
                    Param::new(ParamValue::Scalar(6.)).unwrap(),
                )
                .unwrap();
            assert_eq!(Project::new(good).is_ok(), own, "{name}");
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

fn cubic(points: [[f64; 2]; 4], t: f64) -> [f64; 2] {
    let u = 1.0 - t;
    let [w0, w1, w2, w3] = [u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t];
    let [(x0, y0), (x1, y1), (x2, y2), (x3, y3)] = points.map(|[x, y]| (x, y));
    [
        w0 * x0 + w1 * x1 + w2 * x2 + w3 * x3,
        w0 * y0 + w1 * y1 + w2 * y2 + w3 * y3,
    ]
}

#[test]
fn freeform_path_constructors_and_edits_keep_cubic_geometry() {
    let ellipse = MaskPath::ellipse();
    assert!(ellipse.closed);
    assert_eq!(ellipse.vertices.len(), 4);
    let k = 0.5 * 4.0 / 3.0 * (std::f64::consts::PI / 8.0).tan();
    let anchors = [[0.5, 0.0], [1.0, 0.5], [0.5, 1.0], [0.0, 0.5]];
    let outs = [[k, 0.0], [0.0, k], [-k, 0.0], [0.0, -k]];
    for ((vertex, p), out) in ellipse.vertices.iter().zip(anchors).zip(outs) {
        assert_eq!(vertex.p, p);
        assert_eq!(vertex.t_out, out);
        assert_eq!(vertex.t_in, [-out[0], -out[1]]);
    }
    let rectangle = MaskPath::rectangle();
    assert!(rectangle.closed);
    assert_eq!(rectangle.vertices.len(), 4);
    assert!(
        rectangle
            .vertices
            .iter()
            .all(|v| v.t_in == [0.0; 2] && v.t_out == [0.0; 2])
    );
    let polygon = MaskPath::polygon(&[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]]).unwrap();
    assert!(polygon.editable().is_ok());
    for points in [
        vec![[0.0, 0.0], [1.0, 0.0]],
        vec![[0.0, 0.0], [f64::NAN, 0.0], [1.0, 1.0]],
        vec![[0.0, 0.0], [101.0, 0.0], [1.0, 1.0]],
    ] {
        assert!(MaskPath::polygon(&points).is_err());
    }
    assert!(
        MaskPath {
            closed: false,
            ..ellipse.clone()
        }
        .editable()
        .is_err()
    );

    // The de Casteljau split keeps both halves on the original cubic.
    let mut source = MaskPath::rectangle();
    source.vertices[0] = MaskVertex {
        p: [0.0, 0.0],
        t_in: [0.0; 2],
        t_out: [0.0, 1.0],
    };
    source.vertices[1] = MaskVertex {
        p: [1.0, 0.0],
        t_in: [0.0, 1.0],
        t_out: [0.0; 2],
    };
    let split = source.split(0, 0.5).unwrap();
    assert_eq!(split.vertices.len(), 5);
    let inserted = &split.vertices[1];
    assert_eq!(inserted.p, [0.5, 0.75]);
    assert_eq!(source.vertices[0].t_out, [0.0, 1.0], "source unchanged");
    assert_eq!(split.vertices[0].t_out, [0.0, 0.5]);
    assert_eq!(inserted.t_in, [-0.25, 0.0]);
    assert_eq!(inserted.t_out, [0.25, 0.0]);
    assert_eq!(split.vertices[2].t_in, [0.0, 0.5]);
    let controls = [[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]];
    let left = [[0.0, 0.0], [0.0, 0.5], [0.25, 0.75], [0.5, 0.75]];
    let right = [[0.5, 0.75], [0.75, 0.75], [1.0, 0.5], [1.0, 0.0]];
    for u in [0.0, 0.25, 0.5, 1.0] {
        assert_eq!(cubic(left, u), cubic(controls, 0.5 * u));
        assert_eq!(cubic(right, u), cubic(controls, 0.5 + 0.5 * u));
    }
    for (segment, t) in [(9, 0.5), (0, 0.0), (0, 1.0), (0, f64::NAN)] {
        assert!(source.split(segment, t).is_err(), "{segment} {t}");
    }

    let removed = split.without(&[1]).unwrap();
    let mut expected = split.clone();
    expected.vertices.remove(1);
    assert_eq!(removed, expected, "survivors keep their handles");
    for points in [vec![], vec![0, 0], vec![7], vec![0, 1, 2]] {
        assert!(removed.without(&points).is_err(), "{points:?}");
    }

    // Pasted anchors land after the given anchor, in order, with their handles.
    let copied = [ellipse.vertices[1].clone(), ellipse.vertices[3].clone()];
    let pasted = ellipse.with_inserted(1, &copied).unwrap();
    assert_eq!(pasted.vertices.len(), 6);
    assert_eq!(pasted.vertices[1], ellipse.vertices[1]);
    assert_eq!(pasted.vertices[2], copied[0]);
    assert_eq!(pasted.vertices[3], copied[1]);
    assert_eq!(pasted.vertices[4], ellipse.vertices[2]);
    assert!(pasted.closed);
    let mut far = copied.clone();
    far[0].p = [101.0, 0.0];
    for (after, vertices) in [(9, &copied[..]), (0, &[][..]), (0, &far[..])] {
        assert!(ellipse.with_inserted(after, vertices).is_err());
    }
    let open = MaskPath {
        closed: false,
        ..ellipse.clone()
    };
    assert!(open.with_inserted(0, &copied).is_err());
}

#[test]
fn freeform_path_round_trips_through_the_writer_without_flattening() {
    let dir = tempfile::tempdir().unwrap();
    let mut data = empty().into_data();
    let mut glyphs = item(1, 0);
    let mut path = MaskPath::ellipse();
    path.vertices[1].t_out = [0.0, 0.5];
    glyphs.masks[0].shape = MaskShape::Path { path: path.clone() };
    data.scene_mut().timeline.insert_track(track(vec![glyphs]));
    let project = Project::new(data).unwrap();
    let bytes = encode(&project, false).unwrap();
    let loaded = decode(&bytes).unwrap().project;
    assert_eq!(loaded, project);
    let shape = &loaded
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| track.items.iter())
        .next()
        .unwrap()
        .masks[0]
        .shape;
    assert!(matches!(shape, MaskShape::Path { .. }));
    let MaskShape::Path { path: stored } = shape else {
        return;
    };
    assert_eq!(
        stored, &path,
        "stored points, tangents and closed survive exactly"
    );
    assert_eq!(stored.vertices.len(), 4, "no flattening at save");
    let file = dir.path().join("mask.vibecut");
    save_fixture(&file, &project).unwrap();
    assert_eq!(load(&file).unwrap().project, project);
}

fn close(actual: [f64; 2], expected: [f64; 2]) -> bool {
    actual
        .iter()
        .zip(expected)
        .all(|(a, e)| (a - e).abs() < 1e-9)
}

#[test]
fn fit_modes_place_portrait_and_landscape_sources_and_reject_unknown_names() {
    let landscape = [1920.0, 1080.0];
    let portrait = [1080.0, 1920.0];
    // (mode, canvas, source px, extent, whether the renderer crops)
    type Row = (FitMode, [f64; 2], [f64; 2], [f64; 2], bool);
    let rows: [Row; 9] = [
        // Portrait into landscape: Fit letterboxes at the canvas height.
        (
            FitMode::Fit,
            landscape,
            [1080.0, 1920.0],
            [0.31640625, 1.0],
            false,
        ),
        // Portrait into landscape: Fill covers the width and overflows the height.
        (
            FitMode::Fill,
            landscape,
            [1080.0, 1920.0],
            [1.0, 3.160493827160494],
            true,
        ),
        // Landscape into portrait, the mirror case.
        (
            FitMode::Fit,
            portrait,
            [1920.0, 1080.0],
            [1.0, 0.31640625],
            false,
        ),
        (
            FitMode::Fill,
            portrait,
            [1920.0, 1080.0],
            [3.160493827160494, 1.0],
            true,
        ),
        // Stretch and Manual ignore the source aspect.
        (
            FitMode::Stretch,
            landscape,
            [1080.0, 1920.0],
            [1.0, 1.0],
            false,
        ),
        (
            FitMode::Manual,
            portrait,
            [1920.0, 1080.0],
            [1.0, 1.0],
            false,
        ),
        // None keeps the pixel size, smaller or larger than the canvas.
        (
            FitMode::None,
            landscape,
            [640.0, 360.0],
            [1.0 / 3.0, 1.0 / 3.0],
            false,
        ),
        (
            FitMode::None,
            landscape,
            [3840.0, 2160.0],
            [2.0, 2.0],
            false,
        ),
        // A source of the canvas aspect fits exactly.
        (FitMode::Fit, landscape, [1280.0, 720.0], [1.0, 1.0], false),
    ];
    for (mode, canvas, source, extent, cover) in rows {
        assert!(
            close(mode.extent(canvas, source).unwrap(), extent),
            "{mode:?}"
        );
        let placed = mode.place(canvas, source).unwrap();
        assert_eq!(placed.cover, cover, "{mode:?}");
        if matches!(mode, FitMode::Fit | FitMode::None) {
            // A centred box of the extent.
            assert!(close([placed.width, placed.height], extent), "{mode:?}");
            let centred = [(1.0 - extent[0]) / 2.0, (1.0 - extent[1]) / 2.0];
            assert!(close([placed.x, placed.y], centred), "{mode:?}");
        } else {
            assert_eq!(
                (placed.x, placed.y, placed.width, placed.height),
                (0.0, 0.0, 1.0, 1.0),
                "{mode:?}"
            );
        }
    }
    for source in [[0.0, 10.0], [10.0, -1.0], [f64::NAN, 10.0]] {
        assert!(FitMode::Fit.place(landscape, source).is_none());
    }
    assert!(FitMode::Fit.place([0.0, 1080.0], [10.0, 10.0]).is_none());

    // The persisted spelling is the id, and every other name is refused.
    for mode in FitMode::ALL {
        let value = serde_json::to_value(mode).unwrap();
        assert_eq!(value, serde_json::json!(mode.id()));
        assert_eq!(serde_json::from_value::<FitMode>(value).unwrap(), mode);
    }
    for name in ["zoom", "Fit", "", "contain", "cover"] {
        assert!(serde_json::from_value::<FitMode>(serde_json::json!(name)).is_err());
    }
}

#[test]
fn fit_box_reads_the_asset_stream_and_the_crop_and_ignores_other_items() {
    let asset = testkit::source::asset(3, std::env::temp_dir().join("fit-box-source"))
        .format("mp4")
        .duration(Tick(1000))
        .stream(
            testkit::source::video()
                .time_base(TimeBase::new(1, 30000).unwrap())
                .duration(Tick(1000))
                .start(Tick(0))
                .frame_rate(FrameRate::FPS_29_97)
                .size(1280, 720)
                .build(),
        )
        .build();
    let mut data = empty().into_data();
    data.canvas.width = 1000;
    data.canvas.height = 1000;
    let mut video = item(1, 0);
    video.source_asset = Some(AssetId(3));
    video.content = ItemParams::Video(VideoParams {
        source_audio_enabled: true,
        fit: FitMode::Fit,
        ..VideoParams::plain()
    });
    let assets = [asset];
    // 16:9 inside a square canvas: a 1000 x 562.5 box centred vertically.
    let fitted = fit_box(&data.canvas, &assets, &video).unwrap();
    assert!(close([fitted.width, fitted.height], [1.0, 0.5625]));
    assert!(close([fitted.x, fitted.y], [0.0, 0.21875]));
    assert!(!fitted.cover);
    // A crop to the left half changes the aspect the fit uses (640 x 720).
    if let ItemParams::Video(params) = &mut video.content {
        params.crop = Some(Crop {
            x: 0.0,
            y: 0.0,
            width: 0.5,
            height: 1.0,
        });
    }
    let cropped = fit_box(&data.canvas, &assets, &video).unwrap();
    assert!(close([cropped.width, cropped.height], [8.0 / 9.0, 1.0]));
    // An unknown source fills the canvas as Fill does; other items have no box.
    video.source_asset = Some(AssetId(99));
    let unknown = fit_box(&data.canvas, &assets, &video).unwrap();
    assert!(unknown.cover && (unknown.width, unknown.height) == (1.0, 1.0));
    assert!(fit_box(&data.canvas, &assets, &item(2, 0)).is_none());
}

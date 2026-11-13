#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
use crate::support::*;
use render::*;
use time::{Tick, TimeRange};
struct Pixels;
struct Quadrants;
impl SourceProvider for Quadrants {
    fn status(
        &mut self,
        _: &media::SourceAsset,
        _: bool,
        _: &dyn ExecutionHooks,
    ) -> Result<Option<UnavailableReason>> {
        Ok(None)
    }
    fn video(
        &mut self,
        _: &media::SourceAsset,
        _: project::ItemId,
        _: Tick,
        h: &dyn ExecutionHooks,
    ) -> Result<Option<Image>> {
        let mut bytes = Vec::new();
        for y in 0..8 {
            for x in 0..8 {
                let pixel = match (x < 4, y < 4) {
                    (true, true) => [255, 0, 0, 255],
                    (false, true) => [0, 255, 0, 255],
                    (true, false) => [0, 0, 255, 255],
                    (false, false) => [255, 255, 0, 255],
                };
                bytes.extend_from_slice(&pixel);
            }
        }
        Ok(Some(Image::from_rgba8(8, 8, &bytes, false, 13, h)?))
    }
    fn audio(
        &mut self,
        _: &media::SourceAsset,
        _: project::ItemId,
        _: i64,
        n: usize,
        rate: u32,
        h: &dyn ExecutionHooks,
    ) -> Result<AudioBlock> {
        AudioBlock::new(rate, 2, n, MemoryKind::Decoded, h)
    }
}
fn crop_scene(
    format: &str,
    crop: Option<project::Crop>,
    transform: &[(&str, f64)],
) -> project::Project {
    let mut animations = project::Animations::default();
    for (path, value) in transform {
        animations
            .set(
                project::AnimationPath::new(*path).unwrap(),
                project::Param::new(project::ParamValue::Scalar(*value)).unwrap(),
            )
            .unwrap();
    }
    crop_scene_animated(format, crop, animations)
}
fn crop_scene_animated(
    format: &str,
    crop: Option<project::Crop>,
    animations: project::Animations,
) -> project::Project {
    let asset = testkit::source::asset(1, std::env::temp_dir().join("render-crop-source"))
        .format(format)
        .duration(tick(1, 1))
        .stream(
            testkit::source::video()
                .codec("rgba")
                .time_base(media::TimeBase::new(1, 30).unwrap())
                .duration(tick(1, 1))
                .start(Tick::ZERO)
                .frame_count(30)
                .variable_frame_rate(false)
                .size(8, 8)
                .build(),
        )
        .build();
    let mut data = project::Project::empty(project::ProjectId(0), "".into())
        .unwrap()
        .into_data();
    data.canvas.width = 64;
    data.canvas.height = 64;
    data.canvas.background = project::Color([0, 0, 0, 255]);
    data.assets.push(asset);
    data.scene_mut().timeline.main.items = vec![project::Item {
        hidden: false,
        blend_mode: Default::default(),
        retime: Default::default(),
        masks: Vec::new(),
        effects: Vec::new(),
        animations,
        id: project::ItemId(0),
        source_asset: Some(media::AssetId(1)),
        source_range: TimeRange::new(Tick::ZERO, tick(1, 1)).unwrap(),
        start: Tick::ZERO,
        content: project::ItemParams::Video(project::VideoParams {
            crop,
            fades: Default::default(),
            source_audio_enabled: false,
            volume: 0.0,
            muted: false,
            cleanup: Default::default(),
            background: ClipBackground::None,
        }),
    }];
    project::Project::new(data).unwrap()
}
fn hash(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325u64, |hash, b| {
        (hash ^ u64::from(*b)).wrapping_mul(0x100000001b3)
    })
}
#[test]
fn item_crop_renders_centred_and_edge_regions_before_transform() {
    if !tools() {
        return;
    }
    let h = Hooks::default();
    let centred = project::Crop {
        x: 0.25,
        y: 0.25,
        width: 0.5,
        height: 0.5,
    };
    let edge = project::Crop {
        x: 0.0,
        y: 0.0,
        width: 0.5,
        height: 0.5,
    };
    for (case, format, crop, transform, expected) in [
        (
            "centred",
            "synthetic",
            Some(centred),
            vec![],
            18372119245977500073u64,
        ),
        (
            "edge",
            "synthetic",
            Some(edge),
            vec![],
            8779314781866312485u64,
        ),
        (
            "rotate-scale",
            "synthetic",
            Some(centred),
            vec![
                ("transform.scaleX", 0.5),
                ("transform.scaleY", 0.5),
                ("transform.rotate", 90.0),
            ],
            2049379840725108052u64,
        ),
        (
            "keyed-position",
            "synthetic",
            Some(edge),
            vec![("crop.x", 0.25), ("crop.y", 0.25)],
            18372119245977500073u64,
        ),
        (
            "image",
            "png_pipe",
            Some(centred),
            vec![],
            18372119245977500073u64,
        ),
    ] {
        let p = crop_scene(format, crop, &transform);
        let mut renderer = Renderer::new(&p, RenderSettings::from_project(&p)).unwrap();
        let frame = renderer
            .render_frame(Tick::ZERO, &mut Quadrants, &h)
            .unwrap();
        let preview = renderer
            .preview_frame(Tick::ZERO, &mut Quadrants, &h)
            .unwrap();
        assert_eq!(frame.pixels(), preview.pixels(), "{case} preview");
        let packed = frame.rgba8(1, &h).unwrap();
        let actual = hash(&packed.bytes);
        let raw = root().join(format!("crop-{case}-golden.rgba"));
        std::fs::write(&raw, &packed.bytes).unwrap();
        let png = root().join(format!("crop-{case}-golden.png"));
        ffmpeg(&[
            "-f".as_ref(),
            "rawvideo".as_ref(),
            "-pixel_format".as_ref(),
            "rgba".as_ref(),
            "-video_size".as_ref(),
            "64x64".as_ref(),
            "-i".as_ref(),
            raw.as_os_str(),
            "-frames:v".as_ref(),
            "1".as_ref(),
            png.as_os_str(),
        ]);
        assert_eq!(actual, expected, "inspected {case} crop golden");
    }
    // A keyed crop position: preview at tick t equals the export frame at t, and the
    // linear midpoint matches the same crop held at that position.
    let key = |t: Tick, v: f64| {
        let mut key = project::Keyframe::new(t, project::ParamValue::Scalar(v));
        key.segment = project::Segment::Linear;
        key
    };
    let mut keyed = project::Animations::default();
    for path in ["crop.x", "crop.y"] {
        let channel = project::Channel::new(
            project::ParamValue::Scalar(0.0),
            vec![key(Tick::ZERO, 0.0), key(tick(1, 1), 0.5)],
            project::Extrapolation::Hold,
            project::Extrapolation::Hold,
        )
        .unwrap();
        keyed
            .set(
                project::AnimationPath::new(path).unwrap(),
                project::Param::Value(channel),
            )
            .unwrap();
    }
    let p = crop_scene_animated("synthetic", Some(edge), keyed);
    let mut renderer = Renderer::new(&p, RenderSettings::from_project(&p)).unwrap();
    let mid = tick(1, 2);
    let frame = renderer.render_frame(mid, &mut Quadrants, &h).unwrap();
    let preview = renderer.preview_frame(mid, &mut Quadrants, &h).unwrap();
    assert_eq!(frame.pixels(), preview.pixels(), "keyed crop preview");
    let held = crop_scene("synthetic", Some(centred), &[]);
    let held = Renderer::new(&held, RenderSettings::from_project(&held))
        .unwrap()
        .render_frame(Tick::ZERO, &mut Quadrants, &h)
        .unwrap();
    assert_eq!(frame.pixels(), held.pixels(), "midpoint equals held crop");
    let none = crop_scene("synthetic", None, &[]);
    let full = crop_scene(
        "synthetic",
        Some(project::Crop {
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
        }),
        &[],
    );
    let render = |p: &project::Project| {
        Renderer::new(p, RenderSettings::from_project(p))
            .unwrap()
            .render_frame(Tick::ZERO, &mut Quadrants, &h)
            .unwrap()
    };
    assert_eq!(render(&none).pixels(), render(&full).pixels());
    let p = crop_scene("synthetic", Some(centred), &[]);
    let mut settings = ExportSettings::from_project(&p);
    settings.include_audio = false;
    settings.mode = ExportMode::Encode;
    settings.quality = 100;
    settings.overwrite = project::atomic::Overwrite::Replace;
    let path = root().join("crop-export-golden.mp4");
    let receipt = export_with_sources(
        &p,
        TimeRange::new(Tick::ZERO, tick(1, 10)).unwrap(),
        &settings,
        &path,
        &mut Quadrants,
        &h,
    )
    .unwrap();
    assert_eq!(receipt.video_frames, 3);
    let exported = rgba(&path);
    let reference = render(&p).rgba8(1, &h).unwrap();
    assert_eq!(exported.len(), reference.bytes.len());
    let mse = exported
        .iter()
        .zip(&reference.bytes)
        .map(|(a, b)| (f64::from(*a) - f64::from(*b)).powi(2))
        .sum::<f64>()
        / exported.len() as f64;
    assert!(mse < 150.0, "encoded crop MSE {mse}");
}

impl SourceProvider for Pixels {
    fn status(
        &mut self,
        _: &media::SourceAsset,
        _: bool,
        _: &dyn ExecutionHooks,
    ) -> Result<Option<UnavailableReason>> {
        Ok(None)
    }
    fn video(
        &mut self,
        _: &media::SourceAsset,
        _: project::ItemId,
        _: Tick,
        h: &dyn ExecutionHooks,
    ) -> Result<Option<Image>> {
        let mut bytes = Vec::new();
        for y in 0..4 {
            for x in 0..4 {
                bytes.extend_from_slice(if y == 0 {
                    &[255, 0, 0, 0]
                } else if x < 2 {
                    &[255, 0, 0, 128]
                } else {
                    &[0, 255, 0, 255]
                });
            }
        }
        Ok(Some(Image::from_rgba8(4, 4, &bytes, false, 13, h)?))
    }
    fn audio(
        &mut self,
        _: &media::SourceAsset,
        _: project::ItemId,
        _: i64,
        n: usize,
        rate: u32,
        h: &dyn ExecutionHooks,
    ) -> Result<AudioBlock> {
        AudioBlock::new(rate, 2, n, MemoryKind::Decoded, h)
    }
}
fn scene() -> project::Project {
    let asset = testkit::source::asset(1, std::env::temp_dir().join("render-synthetic-source"))
        .format("synthetic")
        .duration(tick(1, 1))
        .stream(
            testkit::source::video()
                .codec("rgba")
                .time_base(media::TimeBase::new(1, 30).unwrap())
                .duration(tick(1, 1))
                .start(Tick::ZERO)
                .frame_count(30)
                .variable_frame_rate(false)
                .size(4, 4)
                .build(),
        )
        .build();
    let mut d = project::Project::empty(project::ProjectId(0), "".into())
        .unwrap()
        .into_data();
    d.canvas.width = 256;
    d.canvas.height = 192;
    d.canvas.background = project::Color([20, 40, 80, 255]);
    d.assets.push(asset);
    for (id, kind, content, source_asset) in [
        (
            0,
            project::TrackKind::Video,
            project::ItemParams::Video(project::VideoParams {
                fades: Default::default(),
                source_audio_enabled: false,
                volume: 0.0,
                muted: false,
                crop: None,
                cleanup: Default::default(),
                background: ClipBackground::None,
            }),
            Some(media::AssetId(1)),
        ),
        (
            1,
            project::TrackKind::Text,
            project::ItemParams::Text(project::TextParams {
                content: "Tiếng Việt 中文".into(),
                font_size: 36.0,
                color: project::Color([255, 255, 255, 224]),
                ..Default::default()
            }),
            None,
        ),
    ] {
        d.scene_mut().timeline.insert_track(project::Track {
            id: project::TrackId(id + 1),
            name: "".into(),
            kind,
            muted: false,
            hidden: false,
            locked: false,
            items: vec![project::Item {
                hidden: false,
                blend_mode: Default::default(),
                retime: Default::default(),
                masks: Vec::new(),
                effects: Vec::new(),
                animations: Default::default(),
                id: project::ItemId(id),
                source_asset,
                source_range: TimeRange::new(Tick::ZERO, tick(1, 1)).unwrap(),
                start: Tick::ZERO,
                content,
            }],
        });
    }
    project::Project::new(d).unwrap()
}
#[test]
fn transparent_edge_half_opacity_scaled_text_and_color_survive_encoded_decode() {
    if !tools() {
        return;
    }
    for case in ["plain", "masked", "graded", "keyed"] {
        let masked = case == "masked";
        let mut data = scene().into_data();
        if case == "graded" {
            // Preview and export must agree on a graded clip within the same MSE bound.
            let item = data
                .scene_mut()
                .timeline
                .iter_mut()
                .flat_map(|t| &mut t.items)
                .find(|i| i.id == project::ItemId(0))
                .unwrap();
            item.effects.push(
                project::effect_definition("color_grade")
                    .unwrap()
                    .instance("color_grade")
                    .with("brightness", project::EffectValue::Scalar(30.))
                    .unwrap()
                    .with("contrast", project::EffectValue::Scalar(20.))
                    .unwrap()
                    .with("saturation", project::EffectValue::Scalar(-20.))
                    .unwrap()
                    .with("hue", project::EffectValue::Scalar(10.))
                    .unwrap(),
            );
        }
        if case == "keyed" {
            // Preview and export must agree on a keyed clip: the synthetic source's
            // right half is opaque green and loses its alpha.
            let item = data
                .scene_mut()
                .timeline
                .iter_mut()
                .flat_map(|t| &mut t.items)
                .find(|i| i.id == project::ItemId(0))
                .unwrap();
            item.effects.push(
                project::effect_definition("chroma_key")
                    .unwrap()
                    .instance("chroma_key")
                    .with(
                        "key_color",
                        project::EffectValue::Color(project::Color([0, 255, 0, 255])),
                    )
                    .unwrap()
                    .with("intensity", project::EffectValue::Scalar(40.))
                    .unwrap()
                    .with("shadow", project::EffectValue::Scalar(0.))
                    .unwrap(),
            );
        }
        if masked {
            let item = data
                .scene_mut()
                .timeline
                .iter_mut()
                .flat_map(|t| &mut t.items)
                .find(|i| i.id == project::ItemId(0))
                .unwrap();
            let mut mask = project::Mask::new("ellipse", project::MaskShape::Ellipse);
            mask.width = 0.8;
            mask.height = 0.8;
            mask.feather = 8.;
            mask.stroke.width = 6.;
            mask.stroke.color = project::Color([255, 255, 255, 128]);
            mask.stroke.align = project::StrokeAlign::Outside;
            item.masks.push(mask);
            item.effects.push(
                project::effect_definition("blur")
                    .unwrap()
                    .instance("blur")
                    .with("intensity", project::EffectValue::Scalar(100.))
                    .unwrap(),
            );
        }
        let p = project::Project::new(data).unwrap();
        let h = Hooks::default();
        let mut settings = ExportSettings::from_project(&p);
        settings.render.width = 128;
        settings.render.height = 96;
        settings.include_audio = false;
        settings.mode = ExportMode::Encode;
        settings.quality = 100;
        settings.overwrite = project::atomic::Overwrite::Replace;
        settings.render.placements.insert(
            project::ItemId(0),
            Placement {
                destination: Rect {
                    x: 0.1,
                    y: 0.1,
                    width: 0.8,
                    height: 0.65,
                },
                fit: Fit::Stretch,
                opacity: 0.5,
                ..Default::default()
            },
        );
        settings.render.placements.insert(
            project::ItemId(1),
            Placement {
                destination: Rect {
                    x: 0.1,
                    y: 0.25,
                    width: 0.8,
                    height: 0.6,
                },
                fit: Fit::Stretch,
                opacity: 0.75,
                ..Default::default()
            },
        );
        let mut renderer = Renderer::new(&p, settings.render.clone()).unwrap();
        let reference = renderer.render_frame(Tick::ZERO, &mut Pixels, &h).unwrap();
        let bytes = reference.rgba8(1, &h).unwrap();
        let checksum = bytes.bytes.iter().fold(0xcbf29ce484222325u64, |hash, b| {
            (hash ^ u64::from(*b)).wrapping_mul(0x100000001b3)
        });
        let expected_checksum = match case {
            "masked" => 0xba9c55b24c28e586,
            "graded" => 0x16fda543ff7d7fd8,
            "keyed" => 0x2dba6f3a5e314e17,
            _ => 0x9748c49eea14a2bf,
        };
        assert_eq!(
            checksum, expected_checksum,
            "visually inspected {case} compositor golden"
        );
        let png = root().join(format!("{case}-compositor-golden.png"));
        let raw = root().join(format!("{case}-compositor-golden.rgba"));
        std::fs::write(&raw, &bytes.bytes).unwrap();
        ffmpeg(&[
            "-f".as_ref(),
            "rawvideo".as_ref(),
            "-pixel_format".as_ref(),
            "rgba".as_ref(),
            "-video_size".as_ref(),
            "128x96".as_ref(),
            "-i".as_ref(),
            raw.as_os_str(),
            "-frames:v".as_ref(),
            "1".as_ref(),
            png.as_os_str(),
        ]);
        let path = root().join(format!("{case}-compositor-golden.mp4"));
        let receipt = export_with_sources(
            &p,
            TimeRange::new(Tick::ZERO, tick(1, 10)).unwrap(),
            &settings,
            &path,
            &mut Pixels,
            &Hooks::default(),
        )
        .unwrap();
        assert_eq!(receipt.video_frames, 3);
        let actual = rgba(&path);
        assert_eq!(actual.len(), bytes.bytes.len());
        let mse = actual
            .iter()
            .zip(&bytes.bytes)
            .map(|(a, b)| (f64::from(*a) - f64::from(*b)).powi(2))
            .sum::<f64>()
            / actual.len() as f64;
        assert!(mse < 150.0, "encoded compositor MSE {mse}");
        // Transparent pixels retain the exact background; half-opacity red over it is
        // independently checked in linear light before the lossy encoding boundary.
        let background = reference.pixels()[0];
        let transparent = reference.pixels()[12 * 128 + 20];
        assert_eq!(background, transparent);
        if case == "plain" {
            let red = reference.pixels()[65 * 128 + 25];
            let alpha = 128.0 / 255.0 * 0.5;
            assert!((red[0] - (alpha + background[0] * (1.0 - alpha))).abs() < 1e-5);
            assert_eq!(red[3], 1.0);
        }
        let repeated = renderer.render_frame(tick(1, 30), &mut Pixels, &h).unwrap();
        assert_eq!(repeated.pixels(), reference.pixels());
    }
}

#[test]
fn text_fade_samples_survive_encoded_decode_without_a_new_golden() {
    assert!(tools());
    let mut data = scene().into_data();
    let title = data
        .scene_mut()
        .timeline
        .iter_mut()
        .flat_map(|t| &mut t.items)
        .find(|i| i.id == project::ItemId(1))
        .unwrap();
    let project::ItemParams::Text(text) = &mut title.content else {
        panic!("title")
    };
    text.content = "Tiếng Việt\nEnglish".into();
    text.color.0[3] = 160;
    text.background = Some(project::TextBackground {
        color: project::Color([20, 80, 120, 128]),
        ..Default::default()
    });
    let mut opacity = project::Param::new(project::ParamValue::Scalar(0.6)).unwrap();
    for (t, v) in [
        (0, 0.0),
        (76_204_800_000, 0.6),
        (177_811_200_000, 0.6),
        (254_016_000_000, 0.0),
    ] {
        opacity
            .channel_mut(None)
            .unwrap()
            .put(project::Keyframe::new(
                Tick(t),
                project::ParamValue::Scalar(v),
            ))
            .unwrap();
    }
    title
        .animations
        .set(project::BuiltinPath::Opacity.into(), opacity)
        .unwrap();
    let p = project::Project::new(data).unwrap();
    let mut settings = ExportSettings::from_project(&p);
    settings.render.width = 256;
    settings.render.height = 192;
    settings.include_audio = false;
    settings.mode = ExportMode::Encode;
    settings.quality = 100;
    settings.overwrite = project::atomic::Overwrite::Replace;
    for n in [0, 4, 15, 26] {
        let at = time::FrameRate::FPS_30.tick_of(n).unwrap();
        let h = Hooks::default();
        let expected = Renderer::new(&p, settings.render.clone())
            .unwrap()
            .render_frame(at, &mut Pixels, &h)
            .unwrap()
            .video_rgba8(1, &h)
            .unwrap()
            .bytes;
        let path = root().join(format!("text-fade-{n}.mp4"));
        export_with_sources(
            &p,
            TimeRange::new(at, at.checked_add(tick(1, 30)).unwrap()).unwrap(),
            &settings,
            &path,
            &mut Pixels,
            &Hooks::default(),
        )
        .unwrap();
        let actual = rgba(&path);
        assert_eq!(actual.len(), expected.len());
        for rows in [0..192, 80..192] {
            let range = rows.start * 256 * 4..rows.end * 256 * 4;
            let mse = actual[range.clone()]
                .iter()
                .zip(&expected[range])
                .map(|(a, b)| (f64::from(*a) - f64::from(*b)).powi(2))
                .sum::<f64>()
                / ((rows.end - rows.start) * 256 * 4) as f64;
            assert!(mse < 150.0, "fade frame {n} MSE {mse}");
        }
    }
}

/// Every decoded RGBA frame of `path`, in display order.
fn all_frames(path: &std::path::Path, frame_len: usize) -> Vec<Vec<u8>> {
    let output = tool_command("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(path)
        .args(["-f", "rawvideo", "-pix_fmt", "rgba", "-"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
        .stdout
        .chunks_exact(frame_len)
        .map(<[u8]>::to_vec)
        .collect()
}

#[test]
fn chroma_key_intensity_keyframes_interpolate_in_preview_and_export() {
    if !tools() {
        return;
    }
    let span = tick(1, 10);
    let path = project::AnimationPath::new("effects.chroma_key.params.intensity").unwrap();
    let keys = vec![
        project::Keyframe::new(Tick::ZERO, project::ParamValue::Scalar(0.0)),
        project::Keyframe::new(span, project::ParamValue::Scalar(100.0)),
    ];
    let mut data = project::Project::empty(project::ProjectId(9), "key anim".into())
        .unwrap()
        .into_data();
    data.canvas.width = 128;
    data.canvas.height = 96;
    data.canvas.background = project::Color([0, 0, 0, 255]);
    let mut animations = project::Animations::default();
    animations
        .set(
            path.clone(),
            project::Param::Value(
                project::Channel::new(
                    project::ParamValue::Scalar(0.0),
                    keys,
                    project::Extrapolation::Hold,
                    project::Extrapolation::Hold,
                )
                .unwrap(),
            ),
        )
        .unwrap();
    data.scene_mut().timeline.main.items.push(project::Item {
        hidden: false,
        blend_mode: Default::default(),
        retime: Default::default(),
        masks: Vec::new(),
        effects: vec![
            project::effect_definition("chroma_key")
                .unwrap()
                .instance("chroma_key")
                .with(
                    "key_color",
                    project::EffectValue::Color(project::Color([0, 255, 0, 255])),
                )
                .unwrap()
                .with("intensity", project::EffectValue::Scalar(0.0))
                .unwrap()
                .with("shadow", project::EffectValue::Scalar(0.0))
                .unwrap(),
        ],
        animations,
        id: project::ItemId(1),
        source_asset: None,
        source_range: TimeRange::new(Tick::ZERO, span).unwrap(),
        start: Tick::ZERO,
        content: project::ItemParams::Overlay(project::OverlayParams {
            color: project::Color([120, 204, 102, 255]),
        }),
    });
    let p = project::Project::new(data).unwrap();
    let h = Hooks::default();
    let mut settings = ExportSettings::from_project(&p);
    settings.render.width = 128;
    settings.render.height = 96;
    settings.include_audio = false;
    settings.mode = ExportMode::Encode;
    settings.quality = 100;
    settings.overwrite = project::atomic::Overwrite::Replace;
    let mut renderer = Renderer::new(&p, settings.render.clone()).unwrap();
    let ticks = [Tick::ZERO, tick(1, 30), tick(2, 30)];
    let previews: Vec<_> = ticks
        .iter()
        .map(|t| renderer.render_frame(*t, &mut Empty, &h).unwrap())
        .collect();
    // The interpolated intensity raises keying, so the overlay keeps shrinking.
    let green: Vec<f32> = previews.iter().map(|image| image.pixels()[0][1]).collect();
    assert!(
        green[0] > green[1] && green[1] > green[2],
        "preview alphas {green:?}"
    );
    let mid = path.clone();
    let animated = p
        .data()
        .scene()
        .timeline
        .main
        .items
        .first()
        .and_then(|item| item.animations.value_at(&mid, tick(1, 30)).unwrap());
    assert!(
        matches!(animated, Some(project::ParamValue::Scalar(v)) if v > 0.0 && v < 100.0),
        "midpoint intensity {animated:?}"
    );
    let output = root().join("chroma-key-anim.mp4");
    let receipt = export_with_sources(
        &p,
        TimeRange::new(Tick::ZERO, span).unwrap(),
        &settings,
        &output,
        &mut Empty,
        &h,
    )
    .unwrap();
    assert_eq!(receipt.video_frames, 3);
    let frames = all_frames(&output, 128 * 96 * 4);
    assert_eq!(frames.len(), previews.len());
    for (index, (frame, preview)) in frames.iter().zip(&previews).enumerate() {
        let expected = preview.rgba8(1, &h).unwrap();
        let mse = frame
            .iter()
            .zip(&expected.bytes)
            .map(|(a, b)| (f64::from(*a) - f64::from(*b)).powi(2))
            .sum::<f64>()
            / frame.len() as f64;
        assert!(mse < 150.0, "frame {index} preview/export MSE {mse}");
    }
}

#[test]
fn blur_background_stays_below_lower_timeline_layers() {
    let mut d = scene().into_data();
    // Put an opaque blue overlay below an inset video. A global background
    // prepared while visiting the video must not erase that overlay.
    d.scene_mut().timeline.overlays.remove(0);
    d.scene_mut().timeline.overlays.push(project::Track {
        id: project::TrackId(3),
        name: "".into(),
        kind: project::TrackKind::Video,
        muted: false,
        hidden: false,
        locked: false,
        items: vec![project::Item {
            hidden: false,
            blend_mode: Default::default(),
            retime: Default::default(),
            masks: Vec::new(),
            effects: Vec::new(),
            animations: Default::default(),
            id: project::ItemId(2),
            source_asset: None,
            source_range: TimeRange::new(Tick::ZERO, tick(1, 1)).unwrap(),
            start: Tick::ZERO,
            content: project::ItemParams::Overlay(project::OverlayParams {
                color: project::Color([0, 0, 255, 255]),
            }),
        }],
    });
    let p = project::Project::new(d).unwrap();
    let mut s = RenderSettings::from_project(&p);
    s.background_blur = Some(3);
    s.placements.insert(
        project::ItemId(0),
        Placement {
            destination: Rect {
                x: 0.25,
                y: 0.25,
                width: 0.5,
                height: 0.5,
            },
            ..Default::default()
        },
    );
    let h = Hooks::default();
    let image = Renderer::new(&p, s)
        .unwrap()
        .render_frame(Tick::ZERO, &mut Pixels, &h)
        .unwrap();
    assert_eq!(image.pixels()[0], [0.0, 0.0, 1.0, 1.0]);
    assert!(image.pixels()[100 * 256 + 160][1] > 0.0);
}

#[test]
fn separable_sampling_preserves_fractional_crops_and_transparent_borders() {
    let mut d = scene().into_data();
    d.scene_mut().timeline.overlays.remove(0);
    let p = project::Project::new(d).unwrap();
    let h = Hooks::default();
    // Baselines captured from the scalar public renderer before optimization.
    // Include subpixel crops, partially clipped destinations and one-pixel axes.
    for (width, height, destination, crop, fit, expected) in [
        (
            13,
            17,
            Rect::default(),
            project::Crop::default(),
            Fit::Cover,
            0x9406922a3f194af2u64,
        ),
        (
            13,
            17,
            Rect::default(),
            project::Crop::default(),
            Fit::Contain,
            0x453c26aa637a4267,
        ),
        (
            13,
            17,
            Rect::default(),
            project::Crop::default(),
            Fit::Stretch,
            0x50c6e38ada4d2c67,
        ),
        (
            1,
            17,
            Rect::default(),
            project::Crop::default(),
            Fit::Stretch,
            0x8aac0b8393e47abd,
        ),
        (
            13,
            1,
            Rect::default(),
            project::Crop::default(),
            Fit::Stretch,
            0x30994fcd45211ea0,
        ),
        (
            13,
            17,
            Rect {
                x: -0.2,
                y: 0.12,
                width: 0.91,
                height: 1.12,
            },
            project::Crop {
                x: 0.13,
                y: 0.21,
                width: 0.19,
                height: 0.17,
            },
            Fit::Stretch,
            0x4d4b3e2da6efada6,
        ),
        (
            13,
            17,
            Rect {
                x: 0.12,
                y: -0.2,
                width: 1.12,
                height: 0.91,
            },
            project::Crop {
                x: 0.01,
                y: 0.17,
                width: 0.71,
                height: 0.81,
            },
            Fit::Cover,
            0x150b8c47fba009b1,
        ),
        (
            2,
            2,
            Rect::default(),
            project::Crop::default(),
            Fit::Stretch,
            0x365eecd45ead1427,
        ),
    ] {
        let mut settings = RenderSettings::from_project(&p);
        settings.width = width;
        settings.height = height;
        settings.placements.insert(
            project::ItemId(0),
            Placement {
                destination,
                crop,
                fit,
                opacity: 0.73,
            },
        );
        let mut renderer = Renderer::new(&p, settings).unwrap();
        let frame = renderer.render_frame(Tick::ZERO, &mut Pixels, &h).unwrap();
        let preview = renderer.preview_frame(Tick::ZERO, &mut Pixels, &h).unwrap();
        assert_eq!(frame.pixels(), preview.pixels());
        let packed = frame.rgba8(1, &h).unwrap();
        let hash = packed.bytes.iter().fold(0xcbf29ce484222325u64, |hash, b| {
            (hash ^ u64::from(*b)).wrapping_mul(0x100000001b3)
        });
        assert_eq!(hash, expected, "{width}x{height} {fit:?}");
    }
}

#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
mod common;
use common::*;
use render::*;
use time::{FrameRate, Tick, TimeRange};
struct Pixels;
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
    let asset = media::SourceAsset::try_from(media::AssetData {
        id: media::AssetId(1),
        path: std::env::temp_dir().join("render-synthetic-source"),
        content_hash: media::ContentHash([0; 32]),
        probe: media::SourceProbe {
            format: "synthetic".into(),
            duration: Some(tick(1, 1)),
            streams: vec![media::SourceStream {
                index: 0,
                kind: media::StreamKind::Video,
                codec: "rgba".into(),
                time_base: media::TimeBase::new(1, 30).unwrap(),
                duration: Some(tick(1, 1)),
                start: Some(Tick::ZERO),
                frame_rate: Some(FrameRate::FPS_30),
                frame_count: Some(30),
                variable_frame_rate: Some(false),
                video: Some(media::VideoInfo {
                    width: 4,
                    height: 4,
                    color: media::ColorInfo::default(),
                    bit_depth: Some(8),
                }),
                audio: None,
            }],
        },
    })
    .unwrap();
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
                source_audio_enabled: false,
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
            }),
            None,
        ),
    ] {
        d.timeline.tracks.push(project::Track {
            id: project::TrackId(id),
            name: "".into(),
            kind,
            muted: false,
            hidden: false,
            locked: false,
            items: vec![project::Item {
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
    let p = scene();
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
    assert_eq!(
        checksum, 0x9748c49eea14a2bf,
        "visually inspected compositor golden"
    );
    let png = root().join("compositor-golden.png");
    let raw = root().join("compositor-golden.rgba");
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
    let path = root().join("compositor-golden.mp4");
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
    let red = reference.pixels()[65 * 128 + 25];
    let alpha = 128.0 / 255.0 * 0.5;
    assert!((red[0] - (alpha + background[0] * (1.0 - alpha))).abs() < 1e-5);
    assert_eq!(red[3], 1.0);
    let repeated = renderer.render_frame(tick(1, 30), &mut Pixels, &h).unwrap();
    assert_eq!(repeated.pixels(), reference.pixels());
}

#[test]
fn blur_background_stays_below_lower_timeline_layers() {
    let mut d = scene().into_data();
    // Put an opaque blue overlay below an inset video. A global background
    // prepared while visiting the video must not erase that overlay.
    d.timeline.tracks.pop();
    d.timeline.tracks.insert(
        0,
        project::Track {
            id: project::TrackId(2),
            name: "".into(),
            kind: project::TrackKind::Overlay,
            muted: false,
            hidden: false,
            locked: false,
            items: vec![project::Item {
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
        },
    );
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
    d.timeline.tracks.pop();
    let p = project::Project::new(d).unwrap();
    let h = Hooks::default();
    // Baselines captured from the scalar public renderer before optimization.
    // Include subpixel crops, partially clipped destinations and one-pixel axes.
    for (width, height, destination, crop, fit, expected) in [
        (
            13,
            17,
            Rect::default(),
            Rect::default(),
            Fit::Cover,
            0x9406922a3f194af2u64,
        ),
        (
            13,
            17,
            Rect::default(),
            Rect::default(),
            Fit::Contain,
            0x453c26aa637a4267,
        ),
        (
            13,
            17,
            Rect::default(),
            Rect::default(),
            Fit::Stretch,
            0x50c6e38ada4d2c67,
        ),
        (
            1,
            17,
            Rect::default(),
            Rect::default(),
            Fit::Stretch,
            0x8aac0b8393e47abd,
        ),
        (
            13,
            1,
            Rect::default(),
            Rect::default(),
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
            Rect {
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
            Rect {
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
            Rect::default(),
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

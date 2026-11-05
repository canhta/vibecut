#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
mod common;
use common::*;
use render::*;
use std::sync::atomic::Ordering;
use time::{Tick, TimeRange};
#[test]
fn working_pixels_linear_premultiplied_and_hostile_dimensions() {
    let hooks = Hooks::default();
    let image = Image::from_rgba8(1, 1, &[128, 0, 0, 128], true, 13, &hooks).unwrap();
    assert!((image.pixels()[0][0] - 128.0 / 255.0).abs() < 0.00001);
    assert_eq!(image.pixels()[0][3], 128.0 / 255.0);
    assert_eq!(image.rgba8(13, &hooks).unwrap().bytes, [255, 0, 0, 128]);
    let matte = image.video_rgba8(13, &hooks).unwrap();
    assert!((i32::from(matte.bytes[0]) - 188).abs() <= 1);
    assert_eq!(&matte.bytes[1..], &[0, 0, 255]);
    assert!(matches!(
        Image::new(0, 1, [0.0; 4], &hooks),
        Err(Error::Invalid(_))
    ));
    assert!(Image::new(1, 1, [f32::NAN; 4], &hooks).is_err());
    assert!(Image::new(1, 1, [1.0, 0.0, 0.0, 0.5], &hooks).is_err());
    let small = Hooks {
        limit: 1 << 20,
        ..Default::default()
    };
    assert!(matches!(
        Image::new(8192, 8192, [0.0; 4], &small),
        Err(Error::Budget {
            requested: 1073741824,
            ..
        })
    ));
    assert_eq!(small.requested.load(Ordering::SeqCst), 1073741824);
    assert_eq!(small.used.load(Ordering::SeqCst), 0);
}
#[test]
fn track_order_half_opacity_and_half_open_frames() {
    let mut data = solid_project(1, 64, 64, [255, 0, 0, 255]).into_data();
    data.timeline.insert_track(project::Track {
        id: project::TrackId(2),
        name: "".into(),
        kind: project::TrackKind::Video,
        muted: false,
        hidden: false,
        locked: false,
        items: vec![project::Item {
            masks: Vec::new(),
            effects: Vec::new(),
            animations: Default::default(),
            id: project::ItemId(1),
            source_asset: None,
            source_range: TimeRange::new(Tick::ZERO, tick(1, 1)).unwrap(),
            start: Tick::ZERO,
            content: project::ItemParams::Overlay(project::OverlayParams {
                color: project::Color([0, 0, 255, 128]),
            }),
        }],
    });
    let project = project::Project::new(data).unwrap();
    let hooks = Hooks::default();
    let mut renderer = Renderer::new(&project, RenderSettings::from_project(&project)).unwrap();
    let image = renderer
        .render_frame(Tick::ZERO, &mut Empty, &hooks)
        .unwrap();
    let pixel = image.pixels()[0];
    assert!((pixel[0] - (1.0 - 128.0 / 255.0)).abs() < 1e-6);
    assert!((pixel[2] - 128.0 / 255.0).abs() < 1e-6);
    assert_eq!(pixel[3], 1.0);
    let after = renderer
        .render_frame(tick(1, 1), &mut Empty, &hooks)
        .unwrap();
    assert_eq!(after.pixels()[0], [0.0, 0.0, 0.0, 1.0]);
}
fn animated_overlay_project(
    start: Tick,
    duration: Tick,
    keys: Vec<(Tick, f64)>,
) -> project::Project {
    let mut data = project::Project::empty(project::ProjectId(0), "".into())
        .unwrap()
        .into_data();
    data.canvas.width = 64;
    data.canvas.height = 64;
    let path = project::AnimationPath::new("opacity").unwrap();
    let keyframes = keys
        .into_iter()
        .map(|(time, value)| project::Keyframe::new(time, project::ParamValue::Scalar(value)))
        .collect();
    let channel = project::Channel::new(
        project::ParamValue::Scalar(0.0),
        keyframes,
        project::Extrapolation::Hold,
        project::Extrapolation::Hold,
    )
    .unwrap();
    let mut animations = project::Animations::default();
    animations
        .set(path, project::Param::Value(channel))
        .unwrap();
    data.timeline.insert_track(project::Track {
        id: project::TrackId(1),
        name: "".into(),
        kind: project::TrackKind::Video,
        muted: false,
        hidden: false,
        locked: false,
        items: vec![project::Item {
            masks: Vec::new(),
            effects: Vec::new(),
            animations,
            id: project::ItemId(0),
            source_asset: None,
            source_range: time::SourceRange::new(Tick::ZERO, duration).unwrap(),
            start,
            content: project::ItemParams::Overlay(project::OverlayParams {
                color: project::Color([255, 255, 255, 255]),
            }),
        }],
    });
    project::Project::new(data).unwrap()
}

#[test]
fn keyframed_opacity_uses_item_local_time_and_overrides_static_placement() {
    let start = tick(1, 2);
    let project = animated_overlay_project(
        start,
        tick(3, 2),
        vec![(Tick::ZERO, 0.0), (tick(1, 1), 1.0)],
    );
    let hooks = Hooks::default();
    let mut settings = RenderSettings::from_project(&project);
    settings.placements.insert(
        project::ItemId(0),
        Placement {
            opacity: 0.25,
            ..Default::default()
        },
    );
    let mut renderer = Renderer::new(&project, settings).unwrap();
    for (at, opacity) in [(start, 0.0), (tick(1, 1), 0.5), (tick(3, 2), 1.0)] {
        let image = renderer.render_frame(at, &mut Empty, &hooks).unwrap();
        let pixel = image.pixels()[0];
        for channel in &pixel[..3] {
            assert!((channel - opacity as f32).abs() < 1e-6, "{at:?}: {pixel:?}");
        }
        assert_eq!(pixel[3], 1.0);
    }
}

#[test]
fn keyframed_opacity_clamps_to_the_compositor_range() {
    let project = animated_overlay_project(
        Tick::ZERO,
        tick(3, 1),
        vec![(Tick::ZERO, 2.0), (tick(2, 1), -1.0)],
    );
    let hooks = Hooks::default();
    let mut renderer = Renderer::new(&project, RenderSettings::from_project(&project)).unwrap();
    let bright = renderer
        .render_frame(Tick::ZERO, &mut Empty, &hooks)
        .unwrap();
    assert_eq!(bright.pixels()[0], [1.0, 1.0, 1.0, 1.0]);
    let clear = renderer
        .render_frame(tick(2, 1), &mut Empty, &hooks)
        .unwrap();
    assert_eq!(clear.pixels()[0], [0.0, 0.0, 0.0, 1.0]);
}

#[test]
fn empty_audio_is_exact_silence_for_any_block_partition() {
    let p = project::Project::empty(project::ProjectId(0), "".into()).unwrap();
    let h = Hooks::default();
    let whole = mix_audio(&p, -5, 1000, 2, &mut Empty, &h).unwrap();
    assert!(whole.planes().iter().flatten().all(|x| *x == 0.0));
    let mut all = vec![Vec::new(), Vec::new()];
    let mut at = -5;
    for n in [1, 7, 331, 661] {
        let b = mix_audio(&p, at, n, 2, &mut Empty, &h).unwrap();
        for (d, s) in all.iter_mut().zip(b.planes()) {
            d.extend_from_slice(s);
        }
        at += n as i64;
    }
    assert_eq!(whole.planes(), all);
    assert!(mix_audio(&p, i64::MAX, 2, 2, &mut Empty, &h).is_err());
    assert!(mix_audio(&p, 0, 65537, 2, &mut Empty, &h).is_err());
}
#[test]
fn publication_preserves_destination_on_cancel_fail_finalize_and_mux_limit() {
    if !tools() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("existing movie.mp4");
    let p = solid_project(1, 64, 64, [120, 160, 40, 255]);
    for failure in [
        Stage::Render,
        Stage::Finalize,
        Stage::Finalized,
        Stage::Publish,
    ] {
        std::fs::write(&path, b"original").unwrap();
        let h = Hooks {
            event: Some(std::sync::Arc::new(move |s| {
                if s == failure {
                    Err(Error::Io(std::io::Error::other(
                        "injected write/finalize/rename failure",
                    )))
                } else {
                    Ok(())
                }
            })),
            ..Default::default()
        };
        let mut settings = ExportSettings::from_project(&p);
        settings.overwrite = project::atomic::Overwrite::Replace;
        settings.mode = ExportMode::Encode;
        assert!(
            export(
                &p,
                TimeRange::new(Tick::ZERO, tick(1, 1)).unwrap(),
                &settings,
                &path,
                &h
            )
            .is_err()
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"original");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }
    let h = Hooks::default();
    let state = h.state.clone();
    let h = Hooks {
        event: Some(std::sync::Arc::new(move |s| {
            if s == Stage::Finalized {
                state.store(1, Ordering::SeqCst);
            }
            Ok(())
        })),
        ..h
    };
    let mut s = ExportSettings::from_project(&p);
    s.overwrite = project::atomic::Overwrite::Replace;
    s.mode = ExportMode::Encode;
    assert!(matches!(
        export(
            &p,
            TimeRange::new(Tick::ZERO, tick(1, 1)).unwrap(),
            &s,
            &path,
            &h
        ),
        Err(Error::Cancelled)
    ));
    assert_eq!(std::fs::read(&path).unwrap(), b"original");
    assert_eq!(h.used.load(Ordering::SeqCst), 0);
    s.max_bytes = 100;
    assert!(
        export(
            &p,
            TimeRange::new(Tick::ZERO, tick(1, 1)).unwrap(),
            &s,
            &path,
            &Hooks::default()
        )
        .is_err()
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"original");
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    s.resume = true;
    assert!(matches!(
        export(
            &p,
            TimeRange::new(Tick::ZERO, tick(1, 1)).unwrap(),
            &s,
            &path,
            &Hooks::default()
        ),
        Err(Error::Unsupported(_))
    ));
}
#[test]
fn encoded_color_patch_uses_same_compositor_and_has_valid_audio_video() {
    if !tools() {
        return;
    }
    let p = solid_project(1, 64, 64, [70, 160, 210, 255]);
    let h = Hooks::default();
    let mut renderer = Renderer::new(&p, RenderSettings::from_project(&p)).unwrap();
    let frame = renderer.render_frame(Tick::ZERO, &mut Empty, &h).unwrap();
    let expected = frame.rgba8(1, &h).unwrap();
    let path = root().join("color-patch.mp4");
    let mut s = ExportSettings::from_project(&p);
    s.mode = ExportMode::Encode;
    s.overwrite = project::atomic::Overwrite::Replace;
    let receipt = export(
        &p,
        TimeRange::new(Tick::ZERO, tick(1, 1)).unwrap(),
        &s,
        &path,
        &h,
    )
    .unwrap();
    assert_eq!(receipt.video_frames, 30);
    assert_eq!(receipt.audio_samples, 48000);
    let decoded = rgba(&path);
    assert_eq!(decoded.len(), expected.bytes.len());
    let max = decoded
        .iter()
        .zip(&expected.bytes)
        .map(|(a, b)| a.abs_diff(*b))
        .max()
        .unwrap();
    assert!(max <= 6, "encoded pixel max error {max}");
    let info = probe(&path);
    assert_eq!(info["streams"][0]["codec_name"], "h264");
    assert_eq!(info["streams"][1]["codec_name"], "aac");
    assert_eq!(info["streams"][0]["color_transfer"], "bt709");
    // The explicit platform software path retains the same requested settings
    // and perceptual tolerance; hardware/software bytes need not match.
    s.encoder = media::EncodePreference::Software;
    let software = root().join("color-patch-software.mp4");
    std::fs::write(&software, b"software-original").unwrap();
    let started = std::time::Instant::now();
    match export(
        &p,
        TimeRange::new(Tick::ZERO, tick(1, 1)).unwrap(),
        &s,
        &software,
        &Hooks::default(),
    ) {
        Ok(receipt) => {
            let elapsed = started.elapsed();
            assert_eq!(receipt.video_frames, 30);
            assert_eq!(receipt.audio_samples, 48000);
            let info = probe(&software);
            assert_eq!(info["streams"][0]["codec_name"], "h264");
            assert_eq!(info["streams"][1]["codec_name"], "aac");
            let decoded = rgba(&software);
            assert_eq!(decoded.len(), expected.bytes.len());
            let max = decoded
                .iter()
                .zip(&expected.bytes)
                .map(|(a, b)| a.abs_diff(*b))
                .max()
                .unwrap();
            assert!(max <= 6, "software encoded pixel max error {max}");
            eprintln!(
                "OBSERVED software export: 30 frames, 48000 samples, max channel error {max}, elapsed {elapsed:?}"
            );
        }
        Err(Error::Media(media::MediaError::EncoderUnavailable { .. }))
            if !cfg!(target_os = "macos") =>
        {
            assert_eq!(std::fs::read(&software).unwrap(), b"software-original");
            eprintln!(
                "OBSERVED software H.264 unavailable; pixel quality unverified on this platform"
            );
        }
        Err(other) => panic!("unexpected software encoder failure: {other:?}"),
    }
}
#[test]
fn immutable_text_reuse_is_bounded_invalidates_and_matches_uncached_pixels() {
    let mut d = solid_project(1, 128, 96, [20, 30, 40, 255]).into_data();
    let frame = time::FrameRate::FPS_30.frame_duration().unwrap();
    let cases = [
        ("Tiếng Việt", 32.0, [255; 4]),
        ("Tiếng Việt", 32.0, [255; 4]),
        ("Tie\u{0302}\u{0301}ng Vie\u{0323}\u{0302}t", 32.0, [255; 4]),
        ("Tiếng Việt", 42.0, [255, 0, 0, 200]),
        ("Tiếng Việt", 42.0, [255, 0, 0, 200]),
        ("中文", 32.0, [255; 4]),
    ];
    let items = cases
        .into_iter()
        .enumerate()
        .map(|(n, (content, font_size, color))| project::Item {
            masks: Vec::new(),
            effects: Vec::new(),
            animations: Default::default(),
            id: project::ItemId(n as u64 + 1),
            source_asset: None,
            source_range: TimeRange::new(Tick::ZERO, frame).unwrap(),
            start: Tick(frame.0 * n as i64),
            content: project::ItemParams::Text(project::TextParams {
                content: content.into(),
                font_size,
                color: project::Color(color),
            }),
        })
        .collect();
    d.timeline.insert_track(project::Track {
        id: project::TrackId(2),
        name: "".into(),
        kind: project::TrackKind::Text,
        muted: false,
        hidden: false,
        locked: false,
        items,
    });
    let p = project::Project::new(d).unwrap();
    let mut settings = RenderSettings::from_project(&p);
    settings.text_cache_entries = 2;
    settings.text_cache_bytes = 100000;
    settings.placements.insert(
        project::ItemId(5),
        Placement {
            destination: Rect {
                x: 0.1,
                y: 0.72,
                width: 0.5,
                height: 0.25,
            },
            ..Default::default()
        },
    );
    let mut uncached = settings.clone();
    uncached.text_cache_bytes = 0;
    let mut cached = Renderer::new(&p, settings).unwrap();
    let mut reference = Renderer::new(&p, uncached).unwrap();
    let h = Hooks::default();
    for n in 0..6 {
        let at = Tick(frame.0 * n);
        let image = cached.render_frame(at, &mut Empty, &h).unwrap();
        let original = reference.render_frame(at, &mut Empty, &h).unwrap();
        assert_eq!(image.pixels(), original.pixels());
        let stats = cached.text_cache_stats();
        assert!(stats.entries <= 2 && stats.bytes <= 100000);
        if n == 2 {
            assert_eq!(stats.hits, 2);
            assert_eq!(stats.misses, 1);
        }
    }
    let before = cached.text_cache_stats().misses;
    cached.render_frame(Tick::ZERO, &mut Empty, &h).unwrap();
    assert_eq!(cached.text_cache_stats().misses, before + 1);
    cached.clear_text_cache();
    assert_eq!(cached.text_cache_stats().entries, 0);
    drop(cached);
    drop(reference);
    assert_eq!(h.used.load(Ordering::SeqCst), 0);
}
#[test]
fn text_cache_releases_inactive_entries_under_new_frame_pressure() {
    let mut d = solid_project(1, 320, 180, [0, 0, 0, 255]).into_data();
    let dt = tick(1, 10);
    let items = [40.0, 42.0]
        .into_iter()
        .enumerate()
        .map(|(n, font_size)| project::Item {
            masks: Vec::new(),
            effects: Vec::new(),
            animations: Default::default(),
            id: project::ItemId(n as u64 + 1),
            source_asset: None,
            source_range: TimeRange::new(Tick::ZERO, dt).unwrap(),
            start: Tick(dt.0 * n as i64),
            content: project::ItemParams::Text(project::TextParams {
                content: "Tiếng Việt 中文".into(),
                font_size,
                color: project::Color([255; 4]),
            }),
        })
        .collect();
    d.timeline.insert_track(project::Track {
        id: project::TrackId(2),
        name: "".into(),
        kind: project::TrackKind::Text,
        muted: false,
        hidden: false,
        locked: false,
        items,
    });
    let p = project::Project::new(d).unwrap();
    let h = Hooks::default();
    let mut renderer = Renderer::new(&p, RenderSettings::from_project(&p)).unwrap();
    drop(renderer.render_frame(Tick::ZERO, &mut Empty, &h).unwrap());
    let pressure = Hooks {
        limit: 1_600_000,
        ..h.clone()
    };
    let actual = renderer.render_frame(dt, &mut Empty, &pressure).unwrap();
    assert_eq!(renderer.text_cache_stats().entries, 1);
    let mut s = RenderSettings::from_project(&p);
    s.text_cache_bytes = 0;
    let expected = Renderer::new(&p, s)
        .unwrap()
        .render_frame(dt, &mut Empty, &Hooks::default())
        .unwrap();
    assert_eq!(actual.pixels(), expected.pixels());
    drop(actual);
    drop(renderer);
    assert_eq!(h.used.load(Ordering::SeqCst), 0);
}
#[test]
fn caption_expansion_is_refused_before_raster_allocation() {
    let mut d = project::Project::empty(project::ProjectId(0), "".into())
        .unwrap()
        .into_data();
    d.canvas.width = 64;
    d.canvas.height = 64;
    let range = TimeRange::new(Tick::ZERO, tick(1, 1)).unwrap();
    d.assets.push(
        media::SourceAsset::try_from(media::AssetData {
            id: media::AssetId(0),
            path: std::env::temp_dir().join("caption-identity"),
            content_hash: media::ContentHash([0; 32]),
            probe: media::SourceProbe {
                format: "caption identity".into(),
                duration: Some(tick(1, 1)),
                streams: Vec::new(),
            },
        })
        .unwrap(),
    );
    let cues = (0..5000)
        .map(|n| captions::Cue {
            id: format!("cue-{n}").into(),
            range,
            source_lang: captions::LanguageTag::parse("vi").unwrap(),
            texts: std::collections::BTreeMap::new(),
        })
        .collect();
    d.timeline.insert_track(project::Track {
        id: project::TrackId(1),
        name: "".into(),
        kind: project::TrackKind::Text,
        muted: false,
        hidden: false,
        locked: false,
        items: vec![project::Item {
            masks: Vec::new(),
            effects: Vec::new(),
            animations: Default::default(),
            id: project::ItemId(0),
            source_asset: Some(media::AssetId(0)),
            source_range: range,
            start: Tick::ZERO,
            content: project::ItemParams::Caption(project::CaptionParams {
                cues: captions::CueTrack { cues },
            }),
        }],
    });
    let p = project::Project::new(d).unwrap();
    let h = Hooks::default();
    assert!(matches!(
        Renderer::new(&p, RenderSettings::from_project(&p))
            .unwrap()
            .render_frame(Tick::ZERO, &mut Empty, &h),
        Err(Error::Invalid("caption layer expansion"))
    ));
    assert_eq!(h.requested.load(Ordering::SeqCst), 0);
}

#[test]
fn rgba_ingress_observes_cancellation_and_releases_its_reservation() {
    let hooks = Hooks::default();
    hooks.state.store(1, Ordering::SeqCst);
    assert!(matches!(
        Image::from_rgba8(1, 1, &[255; 4], false, 1, &hooks),
        Err(Error::Cancelled)
    ));
    assert_eq!(hooks.used.load(Ordering::SeqCst), 0);
}

#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
use crate::support::*;
use render::*;
use std::sync::atomic::Ordering;
use time::{Tick, TimeRange};

#[test]
fn export_options_refuse_hostile_geometry_rates_versions_and_incompatible_codecs() {
    let p = solid_project(1, 64, 64, [0, 0, 0, 255]);
    let mut original = ExportSettings::from_project(&p);
    OutputOptions::default().apply(&mut original).unwrap();
    assert!(matches!(original.mode, ExportMode::Auto));
    for choice in [
        OutputOptions {
            resolution: Some([64, 64]),
            ..Default::default()
        },
        OutputOptions {
            fps: Some(time::FrameRate::FPS_24),
            ..Default::default()
        },
        OutputOptions {
            bit_rate: Some(100_000),
            ..Default::default()
        },
        OutputOptions {
            codec: VideoCodec::H264,
            ..Default::default()
        },
        OutputOptions {
            profile: media::H264Profile::Main,
            ..Default::default()
        },
    ] {
        let mut settings = ExportSettings::from_project(&p);
        choice.apply(&mut settings).unwrap();
        assert!(matches!(settings.mode, ExportMode::Encode));
    }
    let encoded = serde_json::to_value(OutputOptions::default()).unwrap();
    for field in encoded.as_object().unwrap().keys() {
        let mut incomplete = encoded.clone();
        incomplete.as_object_mut().unwrap().remove(field);
        assert!(
            serde_json::from_value::<OutputOptions>(incomplete).is_err(),
            "missing {field}"
        );
    }
    let mut unknown = encoded;
    unknown["unknown"] = true.into();
    assert!(serde_json::from_value::<OutputOptions>(unknown).is_err());
    for resolution in [[2, 2], [8192, 8192]] {
        OutputOptions {
            resolution: Some(resolution),
            ..Default::default()
        }
        .validate()
        .unwrap();
    }
    for resolution in [
        [0, 64],
        [63, 64],
        [64, 1],
        [u32::MAX, u32::MAX],
        [16384, 16384],
    ] {
        assert!(
            OutputOptions {
                resolution: Some(resolution),
                ..Default::default()
            }
            .ranges(&p)
            .is_err()
        );
    }
    for rate in [0, 999, 1_000_000_001, usize::MAX] {
        assert!(
            OutputOptions {
                bit_rate: Some(rate),
                ..Default::default()
            }
            .validate()
            .is_err()
        );
    }
    for version in [0, 2, u32::MAX] {
        assert!(matches!(
            OutputOptions {
                version,
                ..Default::default()
            }
            .validate(),
            Err(Error::Unsupported(_))
        ));
    }
    for fps in [
        time::FrameRate::new(31, 1).unwrap(),
        time::FrameRate::new(48_000, 1).unwrap(),
        time::FrameRate::new(17, 1_001).unwrap(),
    ] {
        assert!(
            OutputOptions {
                fps: Some(fps),
                ..Default::default()
            }
            .validate()
            .is_err(),
            "{fps}"
        );
    }
    let mut settings = ExportSettings::from_project(&p);
    assert!(
        OutputOptions {
            codec: VideoCodec::Vp9,
            ..Default::default()
        }
        .apply(&mut settings)
        .is_err()
    );
    settings.container = media::Container::WebM;
    for options in [
        OutputOptions {
            codec: VideoCodec::H264,
            ..Default::default()
        },
        OutputOptions {
            profile: media::H264Profile::High,
            ..Default::default()
        },
    ] {
        assert!(options.apply(&mut settings).is_err());
    }
}

#[test]
fn export_refuses_hostile_settings_and_zero_duration_with_typed_errors() {
    let p = solid_project(1, 64, 64, [0, 0, 0, 255]);
    let range = TimeRange::new(Tick::ZERO, tick(1, 1)).unwrap();
    for fps in [
        time::FrameRate::new(17, 1).unwrap(),
        time::FrameRate::new(17, 1_001).unwrap(),
    ] {
        let mut settings = ExportSettings::from_project(&p);
        settings.fps = fps;
        assert!(
            matches!(
                export(
                    &p,
                    range,
                    &settings,
                    std::path::Path::new("never-written.mp4"),
                    &Hooks::default()
                ),
                Err(Error::Invalid("export settings/range"))
            ),
            "{fps}"
        );
    }
    // VP9 and H.264 both refuse odd output axes before any encoder opens.
    for (width, height) in [(65, 64), (64, 65), (3, 64), (64, 3)] {
        let mut settings = ExportSettings::from_project(&p);
        settings.container = media::Container::WebM;
        settings.render.width = width;
        settings.render.height = height;
        assert!(
            matches!(
                export(
                    &p,
                    range,
                    &settings,
                    std::path::Path::new("never-written.webm"),
                    &Hooks::default()
                ),
                Err(Error::Invalid("export settings/range"))
            ),
            "{width}x{height}"
        );
    }
    // Every remaining explicit bound is a typed refusal, never a clamp.
    let rejections: [fn(&mut ExportSettings); 6] = [
        |s| s.bit_rate = 0,
        |s| s.quality = 101,
        |s| s.channels = 3,
        |s| s.max_bytes = 0,
        |s| s.audio_block = 0,
        |s| s.audio_cut_fade_ms = 1_001,
    ];
    for reject in rejections {
        let mut settings = ExportSettings::from_project(&p);
        settings.container = media::Container::WebM;
        reject(&mut settings);
        assert!(matches!(
            export(
                &p,
                range,
                &settings,
                std::path::Path::new("never-written.webm"),
                &Hooks::default()
            ),
            Err(Error::Invalid("export settings/range"))
        ));
    }
    // An empty timeline cannot form a positive range, so it refuses before staging.
    let mut data = project::Project::empty(project::ProjectId(0), "".into())
        .unwrap()
        .into_data();
    data.canvas.width = 64;
    data.canvas.height = 64;
    let empty = project::Project::new(data).unwrap();
    let mut settings = ExportSettings::from_project(&empty);
    settings.container = media::Container::WebM;
    assert!(matches!(
        export(
            &empty,
            range,
            &settings,
            std::path::Path::new("never-written.webm"),
            &Hooks::default()
        ),
        Err(Error::Invalid("export settings/range"))
    ));
}

#[test]
fn export_range_snaps_ticks_refuses_empty_and_resolves_scene_durations_without_edits() {
    let p = solid_project(2, 64, 64, [0, 0, 0, 255]);
    let fps = p.data().canvas.fps;
    let frame = fps.frame_duration().unwrap();
    let mut data = p.clone().into_data();
    let mut other = data.scene().clone();
    other.id = project::SceneId(9);
    other.is_main = false;
    other.name = "Other".into();
    other.timeline.main.id = project::TrackId(8);
    for track in other.timeline.iter_mut() {
        track.id = project::TrackId(track.id.0 + 10);
        for item in &mut track.items {
            item.id = project::ItemId(item.id.0 + 10);
            item.source_range = TimeRange::new(Tick::ZERO, tick(1, 1)).unwrap();
        }
    }
    data.scenes.insert(other).unwrap();
    let p = project::Project::new(data).unwrap();
    let before = project::encode(&p, false).unwrap();
    let options = OutputOptions {
        scene: SceneChoice::All,
        ..Default::default()
    };
    let scenes = options.ranges(&p).unwrap();
    assert_eq!(
        scenes.iter().map(|(_, r)| r.duration()).collect::<Vec<_>>(),
        vec![tick(2, 1), tick(1, 1)]
    );
    assert_eq!(
        options.paths(&p, std::path::Path::new("out.mp4")).unwrap(),
        vec![
            std::path::PathBuf::from("out-scene-0.mp4"),
            "out-scene-9.mp4".into()
        ]
    );
    let selected = OutputOptions {
        scene: SceneChoice::Selected {
            scene: project::SceneId(9),
        },
        ..Default::default()
    };
    assert_eq!(selected.ranges(&p).unwrap()[0].1.duration(), tick(1, 1));
    assert!(
        OutputOptions {
            scene: SceneChoice::Selected {
                scene: project::SceneId(u64::MAX)
            },
            ..Default::default()
        }
        .ranges(&p)
        .is_err()
    );
    for (start, end) in [
        (Tick(-1), frame),
        (frame, frame),
        (frame, Tick::ZERO),
        (Tick::ZERO, tick(3, 1)),
        (Tick(1), Tick(2)),
    ] {
        assert!(
            OutputOptions {
                range: RangeChoice::InOut { start, end },
                ..Default::default()
            }
            .ranges(&p)
            .is_err()
        );
    }
    for range in [
        RangeChoice::InOut {
            start: Tick(frame.0 + 1),
            end: Tick(frame.0 * 3 - 1),
        },
        RangeChoice::Selected {
            start: Tick(frame.0 + 1),
            end: Tick(frame.0 * 3 - 1),
        },
    ] {
        let range = OutputOptions {
            range,
            ..Default::default()
        }
        .ranges(&p)
        .unwrap()[0]
            .1;
        assert_eq!((range.start(), range.end()), (frame, Tick(frame.0 * 3)));
    }
    let bytes = serde_json::to_vec(&options).unwrap();
    assert_eq!(
        serde_json::from_slice::<OutputOptions>(&bytes).unwrap(),
        options
    );
    assert_eq!(project::encode(&p, false).unwrap(), before);
}
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
    data.scene_mut().timeline.insert_track(project::Track {
        id: project::TrackId(2),
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
    data.scene_mut().timeline.insert_track(project::Track {
        id: project::TrackId(1),
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
fn text_item_font_background_and_position_reach_the_compositor() {
    let mut data = solid_project(1, 320, 180, [0, 0, 0, 255]).into_data();
    let mut text = project::TextParams::new("VibeCut");
    text.font_size = 40.0;
    data.scene_mut().timeline.insert_track(project::Track {
        id: project::TrackId(2),
        name: "Text".into(),
        kind: project::TrackKind::Text,
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
            id: project::ItemId(1),
            source_asset: None,
            source_range: time::SourceRange::new(Tick::ZERO, tick(1, 1)).unwrap(),
            start: Tick::ZERO,
            content: project::ItemParams::Text(text.clone()),
        }],
    });
    let baseline = project::Project::new(data).unwrap();
    let hooks = Hooks::default();
    let mut renderer = Renderer::new(&baseline, RenderSettings::from_project(&baseline)).unwrap();
    let plain = renderer
        .render_frame(Tick::ZERO, &mut Empty, &hooks)
        .unwrap();

    for (axis, left, right) in [
        ("x", 0.0, 25.0),
        ("x", 75.0, 100.0),
        ("y", 0.0, 10.0),
        ("y", 90.0, 100.0),
    ] {
        let frames: Vec<_> = [left, right]
            .into_iter()
            .map(|position| {
                let mut data = baseline.clone().into_data();
                let project::ItemParams::Text(text) =
                    &mut data.scene_mut().timeline.overlays[0].items[0].content
                else {
                    panic!("text item")
                };
                if axis == "x" {
                    text.position.x = position;
                } else {
                    text.position.y = position;
                }
                let project = project::Project::new(data).unwrap();
                Renderer::new(&project, RenderSettings::from_project(&project))
                    .unwrap()
                    .render_frame(Tick::ZERO, &mut Empty, &hooks)
                    .unwrap()
            })
            .collect();
        assert_ne!(
            frames[0].pixels(),
            frames[1].pixels(),
            "{axis} {left} vs {right}"
        );
    }

    let mut font_data = baseline.clone().into_data();
    let project::ItemParams::Text(text) =
        &mut font_data.scene_mut().timeline.overlays[0].items[0].content
    else {
        panic!("text item")
    };
    text.font = "Noto Serif".into();
    let font_project = project::Project::new(font_data).unwrap();
    let font_image = Renderer::new(&font_project, RenderSettings::from_project(&font_project))
        .unwrap()
        .render_frame(Tick::ZERO, &mut Empty, &hooks)
        .unwrap();
    assert_ne!(
        plain.pixels(),
        font_image.pixels(),
        "font alone changes glyphs"
    );

    let mut data = baseline.into_data();
    let project::ItemParams::Text(text) =
        &mut data.scene_mut().timeline.overlays[0].items[0].content
    else {
        panic!("text item")
    };
    text.background = Some(project::TextBackground {
        color: project::Color([255, 0, 0, 255]),
        padding_x: 8.0,
        padding_y: 8.0,
        offset_x: 0.0,
        offset_y: 0.0,
        radius: 2.0,
    });
    let styled = project::Project::new(data).unwrap();
    let mut renderer = Renderer::new(&styled, RenderSettings::from_project(&styled)).unwrap();
    let decorated = renderer
        .render_frame(Tick::ZERO, &mut Empty, &hooks)
        .unwrap();

    assert_ne!(plain.pixels(), decorated.pixels());
    let red = decorated
        .pixels()
        .iter()
        .filter(|pixel| pixel[0] > 0.5 && pixel[1] < 0.2 && pixel[2] < 0.2)
        .count();
    assert!(red > 0, "background box paints red pixels");
    let plain_red = plain
        .pixels()
        .iter()
        .filter(|pixel| pixel[0] > 0.5 && pixel[1] < 0.2 && pixel[2] < 0.2)
        .count();
    assert_eq!(plain_red, 0, "plain title has no background box");
}

#[test]
fn text_typography_reaches_the_compositor_and_a_newly_loaded_family_invalidates_cached_text() {
    let mut data = solid_project(1, 320, 180, [0, 0, 0, 255]).into_data();
    let mut text = project::TextParams::new(
        "Vibe
Cut",
    );
    // Two lines must fit the 45 px item box so line height is visible.
    text.font_size = 12.0;
    data.scene_mut().timeline.insert_track(project::Track {
        id: project::TrackId(2),
        name: "Text".into(),
        kind: project::TrackKind::Text,
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
            id: project::ItemId(1),
            source_asset: None,
            source_range: time::SourceRange::new(Tick::ZERO, tick(1, 1)).unwrap(),
            start: Tick::ZERO,
            content: project::ItemParams::Text(text),
        }],
    });
    let baseline = project::Project::new(data).unwrap();
    let hooks = Hooks::default();
    let render = |edit: &dyn Fn(&mut project::TextParams)| {
        let mut data = baseline.clone().into_data();
        let project::ItemParams::Text(text) =
            &mut data.scene_mut().timeline.overlays[0].items[0].content
        else {
            panic!("text item")
        };
        edit(text);
        let project = project::Project::new(data).unwrap();
        Renderer::new(&project, RenderSettings::from_project(&project))
            .unwrap()
            .render_frame(Tick::ZERO, &mut Empty, &hooks)
            .unwrap()
    };
    let plain = render(&|_| {});
    type Edit<'a> = &'a dyn Fn(&mut project::TextParams);
    let typography: [(&str, Edit); 5] = [
        ("bold", &|t| t.weight = 700),
        ("italic", &|t| t.italic = true),
        ("letter spacing", &|t| t.letter_spacing = 6.0),
        ("tight letter spacing", &|t| t.letter_spacing = -2.0),
        ("line height", &|t| t.line_height = 30.0),
    ];
    for (label, edit) in typography {
        assert_ne!(plain.pixels(), render(edit).pixels(), "{label}");
    }

    // A family nobody loaded renders exactly like the default family, every time.
    let missing = |t: &mut project::TextParams| t.font = "Late Sample Family".into();
    assert_eq!(plain.pixels(), render(&missing).pixels());

    // Loading the family changes what the same project renders; a renderer that cached the
    // fallback raster does not serve it afterwards.
    let late = project::Project::new({
        let mut data = baseline.clone().into_data();
        let project::ItemParams::Text(text) =
            &mut data.scene_mut().timeline.overlays[0].items[0].content
        else {
            panic!("text item")
        };
        text.font = "Late Sample Family".into();
        data
    })
    .unwrap();
    let mut renderer = Renderer::new(&late, RenderSettings::from_project(&late)).unwrap();
    let before = renderer
        .render_frame(Tick::ZERO, &mut Empty, &hooks)
        .unwrap();
    assert_eq!(plain.pixels(), before.pixels());
    text::register(text::FontData {
        family: "Late Sample Family".into(),
        style: "Regular".into(),
        weight: 400,
        italic: false,
        index: 0,
        bytes: include_bytes!("../../../../assets/fonts/NotoSerif-Regular.ttf").to_vec(),
    })
    .unwrap();
    let after = renderer
        .render_frame(Tick::ZERO, &mut Empty, &hooks)
        .unwrap();
    assert_ne!(before.pixels(), after.pixels());
    assert_eq!(after.pixels(), render(&missing).pixels());
}

#[test]
fn preview_audio_mix_obeys_item_gain_and_mute() {
    struct ConstantAudio;
    impl SourceProvider for ConstantAudio {
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
            _: &dyn ExecutionHooks,
        ) -> Result<Option<Image>> {
            panic!("unexpected video read")
        }
        fn audio(
            &mut self,
            _: &media::SourceAsset,
            _: project::ItemId,
            _: i64,
            frames: usize,
            rate: u32,
            hooks: &dyn ExecutionHooks,
        ) -> Result<AudioBlock> {
            let mut block = AudioBlock::new(rate, 2, frames, MemoryKind::Decoded, hooks)?;
            for plane in block.planes_mut() {
                plane.fill(1.0);
            }
            Ok(block)
        }
    }

    let duration = tick(1, 1);
    let source = testkit::source::asset(1, std::env::temp_dir().join("vibecut-audio-ui.mp4"))
        .format("mov,mp4")
        .duration(duration)
        .content_hash(media::ContentHash::from_bytes(&[1; 8]))
        .stream(
            testkit::source::audio()
                .time_base(media::TimeBase::new(1, 48_000).unwrap())
                .duration(duration)
                .build(),
        )
        .build();
    let base = |volume, muted| {
        let mut data = project::Project::empty(project::ProjectId(0), "".into())
            .unwrap()
            .into_data();
        data.assets.push(source.clone());
        data.scene_mut().timeline.insert_track(project::Track {
            id: project::TrackId(1),
            name: "Audio".into(),
            kind: project::TrackKind::Audio,
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
                id: project::ItemId(1),
                source_asset: Some(media::AssetId(1)),
                source_range: time::SourceRange::new(Tick::ZERO, duration).unwrap(),
                start: Tick::ZERO,
                content: project::ItemParams::Audio(project::AudioParams {
                    fades: Default::default(),
                    volume,
                    muted,
                    source: Default::default(),
                }),
            }],
        });
        project::Project::new(data).unwrap()
    };
    let hooks = Hooks::default();
    for (volume, muted, expected) in [
        (-3.0, false, 10f32.powf(-3.0 / 20.0) * 0.98),
        (-6.0, false, 10f32.powf(-6.0 / 20.0) * 0.98),
        (0.0, true, 0.0),
        (-6.0, true, 0.0),
    ] {
        let project = base(volume, muted);
        let block = mix_audio(&project, 0, 480, 2, &mut ConstantAudio, &hooks).unwrap();
        assert!(
            (block.peak() - expected).abs() < 1e-6,
            "{volume}dB muted={muted}"
        );
        assert!(
            block
                .planes()
                .iter()
                .flatten()
                .all(|sample| sample.is_finite())
        );
    }
}

#[test]
fn publication_preserves_destination_on_cancel_fail_finalize_and_mux_limit() {
    if !tools() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let p = solid_project(1, 64, 64, [120, 160, 40, 255]);
    for (container, extension) in [
        (media::Container::Mp4, "mp4"),
        (media::Container::Mov, "mov"),
        (media::Container::WebM, "webm"),
    ] {
        // Cancellation before publication leaves no final file and no staged
        // neighbour, for WebM as for MP4/MOV.
        let cancelled = dir.path().join(format!("cancelled.{extension}"));
        let h = Hooks {
            event: Some(std::sync::Arc::new(|stage| {
                if stage == Stage::Render {
                    Err(Error::Cancelled)
                } else {
                    Ok(())
                }
            })),
            ..Default::default()
        };
        let mut settings = ExportSettings::from_project(&p);
        settings.overwrite = project::atomic::Overwrite::Replace;
        settings.mode = ExportMode::Encode;
        settings.container = container;
        assert!(matches!(
            export(
                &p,
                TimeRange::new(Tick::ZERO, tick(1, 1)).unwrap(),
                &settings,
                &cancelled,
                &h
            ),
            Err(Error::Cancelled)
        ));
        assert!(!cancelled.exists());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
        let path = dir.path().join(format!("existing movie.{extension}"));
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
            settings.container = container;
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
        s.container = container;
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
        std::fs::remove_file(path).unwrap();
    }
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
            hidden: false,
            blend_mode: Default::default(),
            retime: Default::default(),
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
                ..Default::default()
            }),
        })
        .collect();
    d.scene_mut().timeline.insert_track(project::Track {
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
            hidden: false,
            blend_mode: Default::default(),
            retime: Default::default(),
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
                ..Default::default()
            }),
        })
        .collect();
    d.scene_mut().timeline.insert_track(project::Track {
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
            ephemeral: false,
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
            word_ranges: Vec::new(),
            style: None,
        })
        .collect();
    d.scene_mut().timeline.insert_track(project::Track {
        id: project::TrackId(1),
        name: "".into(),
        kind: project::TrackKind::Text,
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
            id: project::ItemId(0),
            source_asset: Some(media::AssetId(0)),
            source_range: range,
            start: Tick::ZERO,
            content: project::ItemParams::Caption(project::CaptionParams {
                cues: captions::CueTrack {
                    styles: Vec::new(),
                    cues,
                },
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

#[test]
fn graphic_fill_ring_opacity_and_native_geometry_match_analytic_pixels_without_sources() {
    // Independent true ellipse distance: minimize Euclidean distance on the first
    // quadrant, bracket every local minimum before refinement (not renderer contours).
    fn distance(x: f64, y: f64, a: f64, b: f64) -> f64 {
        let (x, y) = (x.abs(), y.abs());
        let f = |t: f64| (x - a * t.cos()).hypot(y - b * t.sin());
        let mut best = f(0.).min(f(std::f64::consts::FRAC_PI_2));
        for i in 0..64 {
            let mut lo = i as f64 * std::f64::consts::FRAC_PI_2 / 64.;
            let mut hi = (i + 1) as f64 * std::f64::consts::FRAC_PI_2 / 64.;
            for _ in 0..30 {
                let l = lo + (hi - lo) / 3.;
                let r = hi - (hi - lo) / 3.;
                if f(l) < f(r) { hi = r } else { lo = l }
            }
            best = best.min(f((lo + hi) / 2.));
        }
        if (x / a).powi(2) + (y / b).powi(2) <= 1. {
            best
        } else {
            -best
        }
    }
    for ellipse in [false, true] {
        for scale in [1, 2, 4] {
            for aspect in [1., 1.5] {
                for width in [0., 1., 64.] {
                    for align in [
                        project::StrokeAlign::Inside,
                        project::StrokeAlign::Center,
                        project::StrokeAlign::Outside,
                    ] {
                        for radius in if ellipse {
                            vec![0.]
                        } else {
                            vec![0., 25., 50.]
                        } {
                            let hooks = Hooks::default();
                            let mut data = solid_project(1, 128, 128, [0; 4]).into_data();
                            data.canvas.background = project::Color([0; 4]);
                            let item = &mut data.scene_mut().timeline.overlays[0].items[0];
                            item.content = project::ItemParams::Graphic(if ellipse {
                                project::GraphicParams::Ellipse {
                                    fill: project::Color([255, 0, 0, 128]),
                                    stroke: project::Color([0, 0, 255, 128]),
                                    stroke_width: width,
                                    stroke_align: align,
                                }
                            } else {
                                project::GraphicParams::Rectangle {
                                    fill: project::Color([255, 0, 0, 128]),
                                    stroke: project::Color([0, 0, 255, 128]),
                                    stroke_width: width,
                                    stroke_align: align,
                                    corner_radius: radius,
                                }
                            });
                            for (name, c) in
                                [("fill", [1., 0., 0., 0.5]), ("stroke", [0., 0., 1., 0.5])]
                            {
                                item.animations
                                    .set(
                                        project::AnimationPath::new(format!("params.{name}"))
                                            .unwrap(),
                                        project::Param::new(project::ParamValue::Color(c)).unwrap(),
                                    )
                                    .unwrap();
                            }
                            item.animations
                                .set(
                                    project::BuiltinPath::Opacity.into(),
                                    project::Param::new(project::ParamValue::Scalar(0.5)).unwrap(),
                                )
                                .unwrap();
                            let p = project::Project::new(data).unwrap();
                            let mut settings = RenderSettings::from_project(&p);
                            settings.width = 128 * scale;
                            settings.height = 128 * scale;
                            settings.placements.insert(
                                project::ItemId(0),
                                Placement {
                                    destination: Rect {
                                        x: 0.25 + 0.25 / f64::from(settings.width),
                                        y: 0.25,
                                        width: 0.375 * aspect,
                                        height: 0.375,
                                    },
                                    ..Default::default()
                                },
                            );
                            let mut renderer = Renderer::new(&p, settings).unwrap();
                            let frame = renderer.render_frame(Tick(0), &mut Empty, &hooks).unwrap();
                            assert_eq!(
                                frame.pixels(),
                                renderer
                                    .preview_frame(Tick(0), &mut Empty, &hooks)
                                    .unwrap()
                                    .pixels()
                            );
                            let size = 128. * f64::from(scale);
                            let (dw, dh) =
                                (48. * aspect * f64::from(scale), 48. * f64::from(scale));
                            let (cx, cy) = (size * 0.25 + 0.25 + dw / 2., size * 0.25 + dh / 2.);
                            let inset = if align == project::StrokeAlign::Center {
                                width / 2.
                            } else {
                                0.
                            };
                            let (a, b) = (
                                dw * (512. - 2. * inset) / 1024.,
                                dh * (512. - 2. * inset) / 1024.,
                            );
                            let w = width * dw.min(dh) / 512.;
                            let (inside, outside) = match align {
                                project::StrokeAlign::Inside => (w, 0.),
                                project::StrokeAlign::Center => (w / 2., w / 2.),
                                project::StrokeAlign::Outside => (0., w),
                            };
                            let f = |d: f64| (d + 0.5).clamp(0., 1.);
                            for (x, y) in [
                                (cx.floor(), cy.floor()),
                                ((cx + a).floor(), cy.floor()),
                                ((cx + a - w).floor(), cy.floor()),
                                ((cx + a + w).floor(), cy.floor()),
                                ((cx + a * 0.7).floor(), (cy + b * 0.7).floor()),
                                (0., 0.),
                            ] {
                                let (px, py) = (x + 0.5 - cx, y + 0.5 - cy);
                                let d = if ellipse {
                                    distance(px, py, a, b)
                                } else {
                                    let (rx, ry) = (a * radius / 50., b * radius / 50.);
                                    let (qx, qy) = (px.abs() - (a - rx), py.abs() - (b - ry));
                                    if radius > 0. && qx > 0. && qy > 0. {
                                        distance(qx, qy, rx, ry)
                                    } else if radius > 0. {
                                        (a - px.abs()).min(b - py.abs())
                                    } else {
                                        -(qx.max(0.).hypot(qy.max(0.)) + qx.max(qy).min(0.))
                                    }
                                };
                                let fill = 0.5 * f(d);
                                let ring = 0.5 * (f(d + outside) - f(d - inside)).max(0.);
                                let expected = [
                                    fill * (1. - ring) * 0.5,
                                    0.,
                                    ring * 0.5,
                                    (ring + fill * (1. - ring)) * 0.5,
                                ];
                                let pixel = frame.pixels()
                                    [y as usize * frame.width() as usize + x as usize];
                                let tolerance = if ellipse || radius > 0. {
                                    (a.max(b) * 0.00028 + 0.051) * 0.5
                                } else {
                                    3e-6
                                };
                                for (actual, expected) in pixel.into_iter().zip(expected) {
                                    assert!(
                                        (f64::from(actual) - expected).abs() < tolerance,
                                        "ellipse={ellipse} scale={scale} width={width} radius={radius} align={align:?} at{x},{y}: {pixel:?} expected {expected}"
                                    );
                                }
                            }
                            drop(frame);
                            drop(renderer);
                            assert_eq!(hooks.used.load(Ordering::SeqCst), 0);
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn graphic_budget_cancel_and_once_opacity_release_every_reservation() {
    let mut data = solid_project(1, 64, 64, [0; 4]).into_data();
    data.canvas.background = project::Color([0; 4]);
    let item = &mut data.scene_mut().timeline.overlays[0].items[0];
    item.content = project::ItemParams::Graphic(project::GraphicParams::Rectangle {
        fill: project::Color([255, 0, 0, 128]),
        stroke: project::Color([0, 0, 255, 128]),
        stroke_width: 64.,
        stroke_align: project::StrokeAlign::Inside,
        corner_radius: 0.,
    });
    item.animations
        .set(
            project::BuiltinPath::Opacity.into(),
            project::Param::new(project::ParamValue::Scalar(0.5)).unwrap(),
        )
        .unwrap();
    let p = project::Project::new(data).unwrap();
    let settings = RenderSettings::from_project(&p);
    for limit in [1, 65536, 131072, 196608] {
        let h = Hooks {
            limit,
            ..Default::default()
        };
        let mut renderer = Renderer::new(&p, settings.clone()).unwrap();
        assert!(matches!(
            renderer.render_frame(Tick(0), &mut Empty, &h),
            Err(Error::Budget { .. })
        ));
        drop(renderer);
        assert_eq!(h.used.load(Ordering::SeqCst), 0);
    }
    struct Cancel {
        h: Hooks,
        calls: std::sync::atomic::AtomicUsize,
    }
    impl ExecutionHooks for Cancel {
        fn cancelled(&self) -> bool {
            self.calls.fetch_add(1, Ordering::SeqCst) > 40
        }
        fn reserve(&self, kind: MemoryKind, bytes: u64) -> Result<Box<dyn Reservation>> {
            self.h.reserve(kind, bytes)
        }
        fn begin_publish(&self) -> Result<()> {
            self.h.begin_publish()
        }
        fn progress(&self, _: Stage, _: f32) -> Result<()> {
            Ok(())
        }
    }
    let h = Cancel {
        h: Hooks::default(),
        calls: std::sync::atomic::AtomicUsize::new(0),
    };
    let mut renderer = Renderer::new(&p, settings.clone()).unwrap();
    assert!(matches!(
        renderer.render_frame(Tick(0), &mut Empty, &h),
        Err(Error::Cancelled)
    ));
    assert_eq!(h.h.used.load(Ordering::SeqCst), 0);
    let h = Hooks::default();
    let image = renderer.render_frame(Tick(0), &mut Empty, &h).unwrap();
    // At a fully covered inside ring, blue is over red before opacity, not two
    // independently attenuated paints. Static alpha128 is deliberately not 0.5.
    let a = 128. / 255.;
    let expected = [a * (1. - a) * 0.5, 0., a * 0.5, (a + a * (1. - a)) * 0.5];
    for (actual, expected) in image.pixels()[3 * 64 + 32].into_iter().zip(expected) {
        assert!((actual - expected).abs() < 3e-6);
    }
    assert_eq!(renderer.text_cache_stats().entries, 0);
}

#[test]
fn graphic_effect_mask_transform_and_final_opacity_keep_the_shared_stage_order() {
    let mut data = solid_project(1, 64, 64, [0; 4]).into_data();
    data.canvas.background = project::Color([0; 4]);
    let item = &mut data.scene_mut().timeline.overlays[0].items[0];
    item.content = project::ItemParams::Graphic(project::GraphicParams::rectangle());
    item.animations
        .set(
            project::AnimationPath::new("params.fill").unwrap(),
            project::Param::new(project::ParamValue::Color([0.25, 0., 0., 0.5])).unwrap(),
        )
        .unwrap();
    item.effects.push(
        project::effect_definition("color_grade")
            .unwrap()
            .instance("grade")
            .with("brightness", project::EffectValue::Scalar(50.))
            .unwrap(),
    );
    let mut mask = project::Mask::new("half", project::MaskShape::Rectangle);
    mask.opacity = 0.5;
    item.masks.push(mask);
    for (path, v) in [("opacity", 0.5), ("transform.positionX", 8.)] {
        item.animations
            .set(
                project::AnimationPath::new(path).unwrap(),
                project::Param::new(project::ParamValue::Scalar(v)).unwrap(),
            )
            .unwrap();
    }
    let p = project::Project::new(data).unwrap();
    let h = Hooks::default();
    let mut renderer = Renderer::new(&p, RenderSettings::from_project(&p)).unwrap();
    let frame = renderer.render_frame(Tick(0), &mut Empty, &h).unwrap();
    for (actual, expected) in frame.pixels()[32 * 64 + 40]
        .into_iter()
        .zip([0.0625, 0., 0., 0.125])
    {
        assert!((actual - expected).abs() < 3e-6, "{actual} vs {expected}");
    }
    assert_eq!(frame.pixels()[32 * 64 + 2], [0.; 4]);
    assert_eq!(
        frame.pixels(),
        renderer
            .preview_frame(Tick(0), &mut Empty, &h)
            .unwrap()
            .pixels()
    );
}

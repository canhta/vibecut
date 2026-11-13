#![allow(clippy::unwrap_used)]
use crate::support::Hooks;
use render::{
    Error, Image,
    mask::{FlatMask, scale_by},
};
use std::sync::atomic::Ordering;
fn rectangle(feather: f32) -> FlatMask {
    FlatMask {
        pts: vec![[10., 10.], [30., 10.], [30., 30.], [10., 30.]],
        feather,
        expansion: 0.,
        opacity: 1.,
        inverted: false,
    }
}
fn alpha(image: &Image, x: usize, y: usize) -> f32 {
    image.pixels().get(y * image.width() as usize + x).unwrap()[3]
}
#[test]
fn rectangle_feather_invert_and_scale_preserve_working_alpha() {
    let hooks = Hooks::default();
    for scale in [1., 2.] {
        let mut m = rectangle(10. * scale);
        for p in &mut m.pts {
            for v in p {
                *v *= scale;
            }
        }
        let n = (40. * scale) as u32;
        let coverage = m.coverage(n, n, &hooks).unwrap();
        for x in (5. * scale) as usize..(15. * scale) as usize {
            assert!(
                (alpha(&coverage, x, (20. * scale) as usize)
                    + alpha(
                        &coverage,
                        (20. * scale) as usize - x - 1,
                        (20. * scale) as usize
                    )
                    - 1.)
                    .abs()
                    < 1e-6
            );
        }
        m.inverted = true;
        m.opacity = 0.5;
        let inverted = m.coverage(n, n, &hooks).unwrap();
        for (a, b) in coverage.pixels().iter().zip(inverted.pixels()) {
            assert!((a[3] * 0.5 + b[3] - 0.5).abs() < 1e-6);
        }
        let mut image = Image::new(n, n, [0.2, 0.1, 0., 0.5], &hooks).unwrap();
        scale_by(&mut image, &coverage, &hooks).unwrap();
        for (p, c) in image.pixels().iter().zip(coverage.pixels()) {
            assert_eq!(*p, [0.2 * c[3], 0.1 * c[3], 0., 0.5 * c[3]]);
        }
    }
}
#[test]
fn hard_edges_expansion_and_all_stroke_alignments() {
    let h = Hooks::default();
    let m = rectangle(0.);
    let c = m.coverage(40, 40, &h).unwrap();
    assert_eq!(alpha(&c, 10, 20), 1.);
    assert_eq!(alpha(&c, 9, 20), 0.);
    for (inside, outside, lo, hi) in [(4., 0., 10, 14), (2., 2., 8, 12), (0., 4., 6, 10)] {
        let s = m.stroke(40, 40, inside, outside, &h).unwrap();
        for x in 0..20 {
            assert_eq!(
                alpha(&s, x, 20),
                if (lo..hi).contains(&x) { 1. } else { 0. },
                "x={x}, {inside}/{outside}"
            );
        }
    }
    let mut m = m;
    m.expansion = 5.;
    assert_eq!(alpha(&m.coverage(40, 40, &h).unwrap(), 5, 20), 1.);
    m.inverted = true;
    let s = m.stroke(40, 40, 4., 0., &h).unwrap();
    assert_eq!(alpha(&s, 4, 20), 1.);
    assert_eq!(alpha(&s, 5, 20), 0.);
}
#[test]
fn coverage_geometry_refuses_before_mutating_same_area_images() {
    let h = Hooks::default();
    let mut item = Image::new(4, 4, [0.2, 0.1, 0., 0.5], &h).unwrap();
    let wrong = Image::new(2, 8, [1.; 4], &h).unwrap();
    assert!(matches!(
        scale_by(&mut item, &wrong, &h),
        Err(Error::Invalid(_))
    ));
    assert!(item.pixels().iter().all(|p| *p == [0.2, 0.1, 0., 0.5]));
}

#[test]
fn hostile_degenerate_budget_and_cancellation_inputs_refuse() {
    let h = Hooks::default();
    let mut m = rectangle(0.);
    for points in [
        vec![],
        vec![[10., 10.]],
        vec![[10., 10.], [20., 20.], [30., 30.]],
    ] {
        m.pts = points;
        assert!(
            m.coverage(40, 40, &h)
                .unwrap()
                .pixels()
                .iter()
                .all(|p| *p == [0.; 4])
        );
        assert!(
            m.stroke(40, 40, 2., 2., &h)
                .unwrap()
                .pixels()
                .iter()
                .all(|p| *p == [0.; 4])
        );
    }
    m.pts = vec![[10., 10.], [30., 30.], [10., 30.], [30., 10.]];
    let crossing = m.coverage(40, 40, &h).unwrap();
    assert_eq!(alpha(&crossing, 20, 12), 1.);
    assert_eq!(alpha(&crossing, 12, 20), 0.);
    m.pts.clear();
    assert!(
        m.coverage(40, 40, &h)
            .unwrap()
            .pixels()
            .iter()
            .all(|p| *p == [0.; 4])
    );
    m.inverted = true;
    assert!(
        m.coverage(40, 40, &h)
            .unwrap()
            .pixels()
            .iter()
            .all(|p| *p == [1.; 4])
    );
    assert!(
        m.stroke(40, 40, 2., 2., &h)
            .unwrap()
            .pixels()
            .iter()
            .all(|p| *p == [0.; 4])
    );
    for bad in [f32::NAN, f32::INFINITY, -1.] {
        m.feather = bad;
        assert!(m.coverage(40, 40, &h).is_err());
    }
    m = rectangle(0.);
    m.pts.get_mut(0).unwrap()[0] = f32::NAN;
    assert!(m.coverage(40, 40, &h).is_err());
    for (shape, expected) in [
        (project::MaskShape::text(" \n", 12.), 1.),
        (
            project::MaskShape::Path {
                path: project::MaskPath {
                    closed: true,
                    vertices: vec![project::MaskVertex {
                        p: [0.5, 0.5],
                        t_in: [0.3, 0.5],
                        t_out: [0.3, -0.5],
                    }],
                },
            },
            0.,
        ),
    ] {
        let mut mask = project::Mask::new("degenerate", shape);
        mask.stroke.width = 4.;
        let project = masked_scene(40, 40, vec![mask]);
        let image = render::Renderer::new(&project, render::RenderSettings::from_project(&project))
            .unwrap()
            .render_frame(time::Tick::ZERO, &mut crate::support::Empty, &h)
            .unwrap();
        assert!(image.pixels().iter().all(|p| p[3] == expected));
    }
    struct WrongDimensions;
    impl render::effects::EffectCompositor for WrongDimensions {
        fn frame<'a>(
            &'a self,
            _: &'a dyn render::ExecutionHooks,
        ) -> render::Result<Option<Box<dyn render::effects::EffectCompositor + 'a>>> {
            Ok(None)
        }
        fn release_idle(&self) -> render::Result<()> {
            Ok(())
        }
        fn supports(&self, _: &str) -> bool {
            true
        }
        fn apply(
            &self,
            _: &Image,
            _: &[render::effects::EffectPass],
            _: render::effects::EffectInputs<'_>,
            hooks: &dyn render::ExecutionHooks,
        ) -> render::Result<Image> {
            Image::new(1, 1, [1.; 4], hooks)
        }
    }
    let mask = project::Mask::new("glyph", project::MaskShape::text("Mask", 4096.));
    let mut data = masked_scene(40, 40, vec![mask]).into_data();
    data.scene_mut()
        .timeline
        .main
        .items
        .first_mut()
        .unwrap()
        .effects
        .push(project::effect_definition("blur").unwrap().instance("blur"));
    let project = project::Project::new(data).unwrap();
    let mut settings = render::RenderSettings::from_project(&project);
    settings.effects = Some(std::sync::Arc::new(WrongDimensions));
    let limited = Hooks {
        limit: 512 * 1024,
        ..Default::default()
    };
    assert!(matches!(
        render::Renderer::new(&project, settings)
            .unwrap()
            .render_frame(time::Tick::ZERO, &mut crate::support::Empty, &limited),
        Err(Error::Invalid("effect output dimensions"))
    ));
    assert_eq!(limited.used.load(Ordering::SeqCst), 0);
    let h = Hooks {
        limit: 40 * 40 * 16,
        ..Hooks::default()
    };
    assert!(matches!(
        rectangle(0.).coverage(40, 40, &h),
        Err(Error::Budget { .. })
    ));
    assert_eq!(h.used.load(Ordering::SeqCst), 0);
    h.state.store(1, Ordering::SeqCst);
    assert!(matches!(
        rectangle(0.).coverage(40, 40, &h),
        Err(Error::Cancelled)
    ));
    assert_eq!(h.used.load(Ordering::SeqCst), 0);
    struct DuringRow {
        calls: std::sync::atomic::AtomicUsize,
        hooks: Hooks,
    }
    impl render::ExecutionHooks for DuringRow {
        fn cancelled(&self) -> bool {
            self.calls.fetch_add(1, Ordering::SeqCst) + 1 >= 4
        }
        fn reserve(
            &self,
            kind: render::MemoryKind,
            bytes: u64,
        ) -> render::Result<Box<dyn render::Reservation>> {
            render::ExecutionHooks::reserve(&self.hooks, kind, bytes)
        }
    }
    let during = DuringRow {
        calls: std::sync::atomic::AtomicUsize::new(0),
        hooks: Hooks::default(),
    };
    assert!(matches!(
        rectangle(0.).coverage(1280, 1, &during),
        Err(Error::Cancelled)
    ));
    assert_eq!(during.hooks.used.load(Ordering::SeqCst), 0);
}

#[test]
fn builtin_contours_appearance_and_strokes_at_two_resolutions() {
    use project::{Mask, MaskShape, StrokeAlign};
    let h = Hooks::default();
    let kinds = [
        "split",
        "cinematic-bars",
        "rectangle",
        "ellipse",
        "heart",
        "diamond",
        "star",
        "text",
    ];
    let appearances = ["hard", "feather", "invert", "inside", "center", "outside"];
    let mut atlas = vec![0u8; 1024 * 1152 * 3];
    for (variant, appearance) in appearances.into_iter().enumerate() {
        let mut hash = 0xcbf29ce484222325u64;
        for (row, (w, height)) in [(64u32, 48u32), (128, 96)].into_iter().enumerate() {
            for (col, kind) in kinds.into_iter().enumerate() {
                let shape = match kind {
                    "split" => MaskShape::Split,
                    "cinematic-bars" => MaskShape::CinematicBars,
                    "rectangle" => MaskShape::Rectangle,
                    "ellipse" => MaskShape::Ellipse,
                    "heart" => MaskShape::Heart,
                    "diamond" => MaskShape::Diamond,
                    "star" => MaskShape::Star,
                    "text" => MaskShape::text("Mask", height as f32 * 0.4),
                    _ => MaskShape::Rectangle,
                };
                let mut mask = Mask::new("mask", shape);
                mask.width = if kind == "text" {
                    1.
                } else {
                    f64::from(height) * 0.6 / f64::from(w)
                };
                mask.height = if kind == "text" { 1. } else { 0.6 };
                let scale = f64::from(height) / 48.;
                match appearance {
                    "feather" => mask.feather = 8. * scale,
                    "invert" => {
                        mask.feather = 8. * scale;
                        mask.inverted = true;
                    }
                    "inside" | "center" | "outside" => {
                        mask.stroke.width = 6. * scale;
                        mask.stroke.color = project::Color([255, 96, 32, 192]);
                        mask.stroke.align = match appearance {
                            "inside" => StrokeAlign::Inside,
                            "outside" => StrokeAlign::Outside,
                            _ => StrokeAlign::Center,
                        };
                    }
                    _ => {}
                }
                let mut rotated = mask.clone();
                rotated.rotation = 180.;
                let project = masked_scene(w, height, vec![mask]);
                let mut renderer =
                    render::Renderer::new(&project, render::RenderSettings::from_project(&project))
                        .unwrap();
                let image = renderer
                    .render_frame(time::Tick::ZERO, &mut crate::support::Empty, &h)
                    .unwrap();
                if appearance == "hard" {
                    let rotated = masked_scene(w, height, vec![rotated]);
                    let rotated = render::Renderer::new(
                        &rotated,
                        render::RenderSettings::from_project(&rotated),
                    )
                    .unwrap()
                    .render_frame(time::Tick::ZERO, &mut crate::support::Empty, &h)
                    .unwrap();
                    assert!(
                        image
                            .pixels()
                            .iter()
                            .zip(rotated.pixels().iter().rev())
                            .all(|(a, b)| (a[3] - b[3]).abs() < 5e-5),
                        "180 degree geometry for {kind}"
                    );
                }
                let bytes = image.rgba8(13, &h).unwrap();
                for (i, pixel) in bytes.bytes.as_chunks::<4>().0.iter().enumerate() {
                    for b in pixel {
                        hash = (hash ^ u64::from(*b)).wrapping_mul(0x100000001b3);
                    }
                    let x = col * 128 + i % w as usize;
                    let y = variant * 192 + row * 96 + i / w as usize;
                    let index = (y * 1024 + x) * 3;
                    for (dst, src) in atlas
                        .get_mut(index..index + 3)
                        .unwrap()
                        .iter_mut()
                        .zip(pixel)
                    {
                        *dst = (f32::from(*src) * f32::from(pixel.get(3).copied().unwrap()) / 255.)
                            .round() as u8;
                    }
                }
            }
        }
        let expected = [
            0xc4edfacb35063679u64,
            0x789f1e3b186691b2,
            0x74090c98d080b5ef,
            0xd23f669a1a493be3,
            0xbf6a74b65184bbe5,
            0xcc0cf6ad883ae957,
        ];
        assert_eq!(
            hash,
            expected.get(variant).copied().unwrap(),
            "{appearance} built-ins at both resolutions"
        );
    }
    let root = crate::support::root();
    std::fs::write(root.join("mask-contours.rgb"), atlas).unwrap();
    if crate::support::tools() {
        crate::support::ffmpeg(&[
            "-f".as_ref(),
            "rawvideo".as_ref(),
            "-pixel_format".as_ref(),
            "rgb24".as_ref(),
            "-video_size".as_ref(),
            "1024x1152".as_ref(),
            "-i".as_ref(),
            root.join("mask-contours.rgb").as_os_str(),
            "-frames:v".as_ref(),
            "1".as_ref(),
            root.join("mask-contours.png").as_os_str(),
        ]);
    }
}

#[test]
fn glyph_coverage_reuses_text_distance_with_fractional_edges_and_nfc() {
    use render::mask::RasterMask;
    let h = Hooks::default();
    let mut previous = None;
    for content in ["Tiếng Việt", "Tie\u{302}\u{301}ng Vie\u{302}\u{323}t"] {
        let layout = text::layout(
            content,
            &text::LayoutOptions {
                size: 22.,
                max_width: 180.,
                ..Default::default()
            },
        )
        .unwrap();
        let raster = text::render(
            &layout,
            text::Canvas {
                width: 192,
                height: 48,
            },
            (4., 4.),
            &text::Paint::default(),
        )
        .unwrap();
        let image = Image::from_rgba8(192, 48, &raster.pixels, true, 13, &h).unwrap();
        let mask = RasterMask::from_image(&image, &h).unwrap();
        let coverage = mask.coverage(0., 0., 1., false, &h).unwrap();
        for (src, out) in image.pixels().iter().zip(coverage.pixels()) {
            assert!((src[3] - out[3]).abs() < 1e-6);
        }
        let feathered = mask.coverage(5., 0., 0.5, false, &h).unwrap();
        let inverted = mask.coverage(5., 0., 0.5, true, &h).unwrap();
        for (a, b) in feathered.pixels().iter().zip(inverted.pixels()) {
            assert!((a[3] + b[3] - 0.5).abs() < 1e-6);
        }
        if let Some(previous) = previous {
            assert_eq!(coverage.pixels(), previous);
        }
        previous = Some(coverage.pixels().to_vec());
    }
}

#[test]
#[ignore = "manual production-renderer timing and RSS measurement"]
fn mask_coverage_benchmark() {
    use project::{Mask, MaskShape};
    use std::time::Instant;
    for (w, height) in [(1920u32, 1080u32), (3840, 2160)] {
        if std::env::var("VIBECUT_MASK_BENCH_WIDTH")
            .ok()
            .is_some_and(|v| v != w.to_string())
        {
            continue;
        }
        let h = Hooks::default();
        let mut mask = Mask::new("ellipse", MaskShape::Ellipse);
        mask.width = f64::from(height) * 0.6 / f64::from(w);
        mask.height = 0.6;
        let hard = masked_scene(w, height, vec![mask.clone()]);
        let hard = render::Renderer::new(&hard, render::RenderSettings::from_project(&hard))
            .unwrap()
            .render_frame(time::Tick::ZERO, &mut crate::support::Empty, &h)
            .unwrap();
        let area = hard.pixels().iter().map(|p| f64::from(p[3])).sum::<f64>();
        let analytic = std::f64::consts::PI * (f64::from(height) * 0.3).powi(2);
        let area_error = (area - analytic).abs() / analytic;
        assert!(area_error < 0.001, "ellipse area error {area_error}");
        drop(hard);
        mask.feather = 10. * f64::from(height) / 1080.;
        let masked = masked_scene(w, height, vec![mask]);
        let baseline = masked_scene(w, height, vec![]);
        let mut masked_renderer =
            render::Renderer::new(&masked, render::RenderSettings::from_project(&masked)).unwrap();
        let mut baseline_renderer =
            render::Renderer::new(&baseline, render::RenderSettings::from_project(&baseline))
                .unwrap();
        let expected = masked_renderer
            .render_frame(time::Tick::ZERO, &mut crate::support::Empty, &h)
            .unwrap();
        baseline_renderer
            .render_frame(time::Tick::ZERO, &mut crate::support::Empty, &h)
            .unwrap();
        h.peak
            .store(h.used.load(Ordering::SeqCst), Ordering::SeqCst);
        let repetitions = 8;
        let mut baseline_ms = 0.;
        let mut masked_ms = 0.;
        let mut max_error = 0f32;
        for _ in 0..repetitions {
            let start = Instant::now();
            let image = baseline_renderer
                .render_frame(time::Tick::ZERO, &mut crate::support::Empty, &h)
                .unwrap();
            baseline_ms += start.elapsed().as_secs_f64() * 1000.;
            std::hint::black_box(image);
            let start = Instant::now();
            let image = masked_renderer
                .render_frame(time::Tick::ZERO, &mut crate::support::Empty, &h)
                .unwrap();
            masked_ms += start.elapsed().as_secs_f64() * 1000.;
            for (a, b) in image.pixels().iter().zip(expected.pixels()) {
                max_error = max_error.max((a[3] - b[3]).abs());
            }
            std::hint::black_box(image);
        }
        println!(
            "mask renderer ellipse {w}x{height}, {repetitions} alternating warm repetitions, baseline_ms={baseline_ms}, masked_ms={masked_ms}, ratio={}, fps={}, analytic_area_error={area_error}, max_alpha_error={max_error}, peak_charged_bytes={}",
            masked_ms / baseline_ms,
            f64::from(repetitions) * 1000. / masked_ms,
            h.peak.load(Ordering::SeqCst)
        );
        assert_eq!(max_error, 0.);
    }
}

#[test]
fn ordered_modes_and_unclipped_strokes_keep_premultiplied_channels() {
    use project::MaskMode::*;
    use render::mask::{combine, initial};
    let h = Hooks::default();
    let quarter = Image::new(1, 1, [0.25; 4], &h).unwrap();
    for (mode, expected, seed) in [
        (None, 0.6, 1.),
        (Add, 0.7, 0.),
        (Subtract, 0.45, 1.),
        (Intersect, 0.15, 1.),
        (Lighten, 0.6, 0.),
        (Darken, 0.25, 1.),
        (Difference, 0.55, 0.),
    ] {
        assert_eq!(initial(mode), seed);
        let mut acc = Image::new(1, 1, [0.6; 4], &h).unwrap();
        combine(&mut acc, &quarter, mode, &h).unwrap();
        assert!((alpha(&acc, 0, 0) - expected).abs() < 1e-6);
    }
    use project::{Mask, MaskMode, MaskShape, StrokeAlign};
    for (reverse, expected) in [(false, 0.), (true, 0.5)] {
        let mut left = Mask::new("left", MaskShape::Rectangle);
        left.width = 0.5;
        left.height = 0.5;
        left.opacity = 0.5;
        let mut right = Mask::new("right", MaskShape::Rectangle);
        right.width = 0.5;
        right.height = 0.5;
        right.center_x = 0.25;
        right.mode = MaskMode::Subtract;
        let project = masked_scene(
            40,
            40,
            if reverse {
                vec![right, left]
            } else {
                vec![left, right]
            },
        );
        let image = render::Renderer::new(&project, render::RenderSettings::from_project(&project))
            .unwrap()
            .render_frame(time::Tick::ZERO, &mut crate::support::Empty, &h)
            .unwrap();
        assert_eq!(alpha(&image, 25, 20), expected);
    }
    let mut red = Mask::new("red", MaskShape::Rectangle);
    red.feather = 12.;
    red.width = 0.5;
    red.height = 0.5;
    red.stroke.width = 4.;
    red.stroke.align = StrokeAlign::Outside;
    red.stroke.color = project::Color([255, 0, 0, 255]);
    let mut blue = red.clone();
    blue.id = "blue".into();
    blue.mode = MaskMode::Intersect;
    blue.stroke.color = project::Color([0, 0, 255, 128]);
    let project = masked_scene(40, 40, vec![red, blue]);
    let mut settings = render::RenderSettings::from_project(&project);
    settings.placements.insert(
        project::ItemId(1),
        render::Placement {
            opacity: 0.5,
            ..Default::default()
        },
    );
    let image = render::Renderer::new(&project, settings)
        .unwrap()
        .render_frame(time::Tick::ZERO, &mut crate::support::Empty, &h)
        .unwrap();
    let p = image.pixels().get(20 * 40 + 8).unwrap();
    assert!((p[0] - (1. - 128. / 255.) * 0.5).abs() < 1e-6);
    assert!((p[2] - 128. / 255. * 0.5).abs() < 1e-6);
    assert_eq!(p[3], 0.5);
    assert_eq!(alpha(&image, 0, 20), 0.);
    let mask = Mask::new("animated", MaskShape::Rectangle);
    let path = mask.path("opacity").unwrap();
    let mut data = masked_scene(40, 40, vec![mask]).into_data();
    let item = data.scene_mut().timeline.main.items.first_mut().unwrap();
    let frame = |n| time::FrameRate::FPS_30.tick_of(n).unwrap();
    item.start = frame(5);
    let mut param = project::Param::new(project::ParamValue::Scalar(0.2)).unwrap();
    param
        .channel_mut(Option::None)
        .unwrap()
        .put(project::Keyframe::new(
            time::Tick::ZERO,
            project::ParamValue::Scalar(0.2),
        ))
        .unwrap();
    param
        .channel_mut(Option::None)
        .unwrap()
        .put(project::Keyframe::new(
            frame(10),
            project::ParamValue::Scalar(0.8),
        ))
        .unwrap();
    item.animations.set(path, param).unwrap();
    let project = project::Project::new(data).unwrap();
    let mut renderer =
        render::Renderer::new(&project, render::RenderSettings::from_project(&project)).unwrap();
    for (at, expected) in [(5, 0.2), (10, 0.5), (15, 0.8)] {
        let image = renderer
            .render_frame(frame(at), &mut crate::support::Empty, &h)
            .unwrap();
        assert!(
            (alpha(&image, 20, 20) - expected).abs() < 1e-6,
            "item-local mask opacity at frame {at}"
        );
    }
}

#[test]
fn animated_mask_centre_matches_its_static_twin_only_at_key_times() {
    let h = Hooks::default();
    let frame = |n| time::FrameRate::FPS_30.tick_of(n).unwrap();
    // The item starts at frame 5, so local time is timeline frame minus 5.
    let scene = |mask: Mask, keys: Option<(f64, f64)>| {
        let mut data = masked_scene(40, 40, vec![mask.clone()]).into_data();
        let item = data.scene_mut().timeline.main.items.first_mut().unwrap();
        item.start = frame(5);
        if let Some((from, to)) = keys {
            let mut param = project::Param::new(project::ParamValue::Scalar(from)).unwrap();
            let channel = param.channel_mut(Option::None).unwrap();
            channel
                .put(project::Keyframe::new(
                    time::Tick::ZERO,
                    project::ParamValue::Scalar(from),
                ))
                .unwrap();
            channel
                .put(project::Keyframe::new(
                    frame(10),
                    project::ParamValue::Scalar(to),
                ))
                .unwrap();
            item.animations
                .set(mask.path("center_x").unwrap(), param)
                .unwrap();
        }
        project::Project::new(data).unwrap()
    };
    let grid = |project: &project::Project, at: i64| {
        let mut renderer =
            render::Renderer::new(project, render::RenderSettings::from_project(project)).unwrap();
        let image = renderer
            .render_frame(frame(at), &mut crate::support::Empty, &h)
            .unwrap();
        (0..40)
            .flat_map(|y| (0..40).map(move |x| (x, y)))
            .map(|(x, y)| alpha(&image, x, y))
            .collect::<Vec<f32>>()
    };
    let plain = Mask::new("moving", MaskShape::Rectangle);
    let mut twin = plain.clone();
    twin.center_x = 0.2;
    let static_start = grid(&scene(plain.clone(), None), 5);
    let static_end = grid(&scene(twin, None), 15);
    let animated = scene(plain, Some((0., 0.2)));
    assert_eq!(grid(&animated, 5), static_start, "key at local 0");
    assert_eq!(grid(&animated, 15), static_end, "key at local 10");
    assert_ne!(grid(&animated, 10), static_start, "between keys it moves");
    assert_ne!(grid(&animated, 10), static_end, "between keys it moves");
}

#[test]
fn text_mask_typography_changes_coverage() {
    let h = Hooks::default();
    let render = |edit: &dyn Fn(&mut project::MaskShape)| {
        let mut shape = project::MaskShape::text("Mask", 36.);
        edit(&mut shape);
        let mut mask = project::Mask::new("glyphs", shape);
        mask.width = 1.;
        mask.height = 1.;
        let project = masked_scene(128, 64, vec![mask]);
        render::Renderer::new(&project, render::RenderSettings::from_project(&project))
            .unwrap()
            .render_frame(time::Tick::ZERO, &mut crate::support::Empty, &h)
            .unwrap()
    };
    let plain = render(&|_| {});
    type Edit<'a> = &'a dyn Fn(&mut project::MaskShape);
    let rows: [(&str, Edit); 4] = [
        ("family", &|s| {
            if let project::MaskShape::Text { font, .. } = s {
                *font = "Noto Serif".into();
            }
        }),
        ("weight", &|s| {
            if let project::MaskShape::Text { weight, .. } = s {
                *weight = 700;
            }
        }),
        ("italic", &|s| {
            if let project::MaskShape::Text { italic, .. } = s {
                *italic = true;
            }
        }),
        ("letter spacing", &|s| {
            if let project::MaskShape::Text { letter_spacing, .. } = s {
                *letter_spacing = 5.;
            }
        }),
    ];
    for (label, edit) in rows {
        assert_ne!(plain.pixels(), render(edit).pixels(), "{label}");
    }
}

fn masked_scene(width: u32, height: u32, masks: Vec<project::Mask>) -> project::Project {
    let mut data = project::Project::empty(project::ProjectId(1), "Masks".into())
        .unwrap()
        .into_data();
    data.canvas.width = width;
    data.canvas.height = height;
    data.canvas.background = project::Color([0; 4]);
    data.scene_mut().timeline.main.items.push(
        testkit::item::item(
            project::ItemId(1),
            project::ItemParams::Overlay(project::OverlayParams {
                color: project::Color([255; 4]),
            }),
        )
        .source(
            time::SourceRange::new(
                time::Tick::ZERO,
                time::FrameRate::FPS_30.tick_of(30).unwrap(),
            )
            .unwrap(),
        )
        .masks(masks)
        .build(),
    );
    project::Project::new(data).unwrap()
}

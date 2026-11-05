#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
mod common;
use common::*;
use project::{AnimationPath, BlendMode, Param, ParamValue};
use render::*;
use serde::Deserialize;
use time::Tick;

#[derive(Deserialize)]
struct Pair {
    below: Vec<[f32; 4]>,
    source: Vec<[f32; 4]>,
    cases: Vec<Golden>,
}
#[derive(Deserialize)]
struct Golden {
    mode: BlendMode,
    opacity: f32,
    pixels: Vec<[f32; 4]>,
}
fn pair() -> Pair {
    serde_json::from_str(include_str!("fixtures/blend-linear.json")).unwrap()
}
fn difference(a: &Image, b: &Image) -> f32 {
    a.pixels()
        .iter()
        .zip(b.pixels())
        .flat_map(|(a, b)| a.iter().zip(b).map(|(a, b)| (a - b).abs()))
        .fold(0., f32::max)
}

#[test]
fn every_mode_matches_linear_image_pair_at_zero_half_and_full_opacity() {
    let fixture = pair();
    let hooks = Hooks::default();
    let src = Image::from_working_pixels(4, 4, fixture.source.iter().copied(), &hooks).unwrap();
    let mut maximum = 0.0_f32;
    for case in fixture.cases {
        let mut dst =
            Image::from_working_pixels(4, 4, fixture.below.iter().copied(), &hooks).unwrap();
        blend::composite(&mut dst, &src, case.opacity, case.mode, &hooks).unwrap();
        let expected =
            Image::from_working_pixels(4, 4, case.pixels.iter().copied(), &hooks).unwrap();
        let max = difference(&dst, &expected);
        maximum = maximum.max(max);
        assert!(
            max <= 3e-6,
            "{:?} opacity {} max {max}",
            case.mode,
            case.opacity
        );
        if case.opacity == 0. {
            assert_eq!(dst.pixels(), fixture.below);
        }
    }
    eprintln!("all-mode CPU golden max_abs={maximum}");
    let mut dst = Image::new(4, 4, [0.1, 0.2, 0.3, 1.], &hooks).unwrap();
    let before = dst.pixels().to_vec();
    for opacity in [f32::NAN, f32::INFINITY, -0.1, 1.1] {
        assert!(blend::composite(&mut dst, &src, opacity, BlendMode::Normal, &hooks).is_err());
        assert_eq!(dst.pixels(), before);
    }
    let wrong = Image::new(1, 1, [0.; 4], &hooks).unwrap();
    assert!(blend::composite(&mut dst, &wrong, 1., BlendMode::Normal, &hooks).is_err());
    assert_eq!(dst.pixels(), before);
}

#[test]
#[ignore = "bounded local throughput measurement"]
fn blend_frame_benchmark() {
    let hooks = Hooks::default();
    let (width, height) = (640, 360);
    let src = Image::new(width, height, [0.2, 0.4, 0.1, 0.5], &hooks).unwrap();
    for mode in BlendMode::ALL {
        let mut dst = Image::new(width, height, [0.4, 0.1, 0.3, 0.75], &hooks).unwrap();
        blend::composite(&mut dst, &src, 0.5, mode, &hooks).unwrap();
        let start = std::time::Instant::now();
        for _ in 0..10 {
            blend::composite(&mut dst, &src, 0.5, mode, &hooks).unwrap();
        }
        let seconds = start.elapsed().as_secs_f64();
        eprintln!(
            "mode={mode:?} frames=10 geometry={width}x{height} seconds={seconds:.6} mpixels_per_second={:.3}",
            f64::from(width * height) * 10.0 / seconds / 1e6
        );
        std::hint::black_box(dst);
    }
    eprintln!(
        "peak_charged_bytes={}",
        hooks.peak.load(std::sync::atomic::Ordering::SeqCst)
    );
}

#[test]
fn mask_then_opacity_then_blend_matches_track_backdrop_and_preview() {
    let mut data = solid_project(1, 64, 64, [200, 40, 100, 128]).into_data();
    data.canvas.background = project::Color([40, 100, 200, 192]);
    let item = data
        .scene_mut()
        .timeline
        .overlays
        .first_mut()
        .unwrap()
        .items
        .first_mut()
        .unwrap();
    item.blend_mode = BlendMode::Multiply;
    let mut mask = project::Mask::new("coverage", project::MaskShape::Rectangle);
    mask.width = 0.5;
    mask.height = 0.5;
    mask.opacity = 0.5;
    item.masks.push(mask);
    item.animations
        .set(
            AnimationPath::new("opacity").unwrap(),
            Param::new(ParamValue::Scalar(0.5)).unwrap(),
        )
        .unwrap();
    let p = project::Project::new(data).unwrap();
    let hooks = Hooks::default();
    let settings = RenderSettings::from_project(&p);
    let mut renderer = Renderer::new(&p, settings).unwrap();
    let frame = renderer
        .render_frame(Tick::ZERO, &mut Empty, &hooks)
        .unwrap();
    let preview = renderer
        .preview_frame(Tick::ZERO, &mut Empty, &hooks)
        .unwrap();
    assert_eq!(frame.pixels(), preview.pixels());
    let b = Image::from_rgba8(1, 1, &[40, 100, 200, 192], false, 13, &hooks)
        .unwrap()
        .pixels()[0];
    let s = Image::from_rgba8(1, 1, &[200, 40, 100, 128], false, 13, &hooks)
        .unwrap()
        .pixels()[0];
    let expected = blend::pixel(b, s, 0.25, BlendMode::Multiply);
    for (x, y, expected) in [(0, 0, b), (32, 32, expected)] {
        for (actual, expected) in frame.pixels()[y * 64 + x].iter().zip(expected) {
            assert!((actual - expected).abs() < 3e-6);
        }
    }
    for opacity in [0., 1.] {
        let mut data = p.clone().into_data();
        let target = data
            .scene_mut()
            .timeline
            .overlays
            .first_mut()
            .unwrap()
            .items
            .first_mut()
            .unwrap();
        target
            .animations
            .set(
                AnimationPath::new("opacity").unwrap(),
                Param::new(ParamValue::Scalar(opacity)).unwrap(),
            )
            .unwrap();
        let project = project::Project::new(data).unwrap();
        let result = Renderer::new(&project, RenderSettings::from_project(&project))
            .unwrap()
            .render_frame(Tick::ZERO, &mut Empty, &hooks)
            .unwrap();
        let expected = blend::pixel(b, s, opacity as f32 * 0.5, BlendMode::Multiply);
        for (actual, expected) in result.pixels()[32 * 64 + 32].iter().zip(expected) {
            assert!((actual - expected).abs() < 3e-6);
        }
        if opacity == 0. {
            assert!(result.pixels().iter().all(|p| *p == b));
        }
    }
}

#[test]
fn encoded_half_opacity_edge_and_color_patch_match_linear_golden() {
    if !tools() {
        return;
    }
    let mut data = solid_project(1, 128, 96, [240, 80, 32, 128]).into_data();
    data.canvas.background = project::Color([32, 64, 128, 255]);
    let item = data
        .scene_mut()
        .timeline
        .overlays
        .first_mut()
        .unwrap()
        .items
        .first_mut()
        .unwrap();
    item.blend_mode = BlendMode::Screen;
    item.animations
        .set(
            AnimationPath::new("opacity").unwrap(),
            Param::new(ParamValue::Scalar(0.5)).unwrap(),
        )
        .unwrap();
    let mut mask = project::Mask::new("edge", project::MaskShape::Rectangle);
    mask.width = 0.5;
    mask.height = 0.5;
    mask.feather = 12.;
    item.masks.push(mask);
    let p = project::Project::new(data).unwrap();
    let hooks = Hooks::default();
    let mut settings = ExportSettings::from_project(&p);
    settings.include_audio = false;
    settings.mode = ExportMode::Encode;
    settings.quality = 100;
    settings.overwrite = project::atomic::Overwrite::Replace;
    // VP9 is available in the pinned LGPL software build on both platforms.
    settings.container = media::Container::WebM;
    let reference = Renderer::new(&p, settings.render.clone())
        .unwrap()
        .render_frame(Tick::ZERO, &mut Empty, &hooks)
        .unwrap();
    let golden = reference.video_rgba8(1, &hooks).unwrap();
    for (a, b) in golden.bytes[(48 * 128 + 64) * 4..(48 * 128 + 64) * 4 + 3]
        .iter()
        .zip([119u8, 60, 116])
    {
        assert!(a.abs_diff(b) <= 1, "linear color patch {a} != {b}");
    }
    let path = root().join("blend-edge-patch.webm");
    export_with_sources(
        &p,
        time::TimeRange::new(Tick::ZERO, p.duration()).unwrap(),
        &settings,
        &path,
        &mut Empty,
        &hooks,
    )
    .unwrap();
    let actual = rgba(&path);
    assert_eq!(actual.len(), golden.bytes.len());
    let max = actual
        .iter()
        .zip(&golden.bytes)
        .map(|(a, b)| a.abs_diff(*b))
        .max()
        .unwrap();
    let mse = actual
        .iter()
        .zip(&golden.bytes)
        .map(|(a, b)| (f64::from(*a) - f64::from(*b)).powi(2))
        .sum::<f64>()
        / actual.len() as f64;
    eprintln!("encoded blend max={max} mse={mse}");
    assert!(
        max <= 20 && mse <= 8.,
        "encoded alpha/colour max={max} mse={mse}"
    );
    for (x, y) in [(0, 0), (64, 48)] {
        let offset = (y * 128 + x) * 4;
        for (a, b) in actual[offset..offset + 3]
            .iter()
            .zip(&golden.bytes[offset..offset + 3])
        {
            assert!(a.abs_diff(*b) <= 6, "decoded patch {x},{y}: {a} != {b}");
        }
    }
    // Fixed source/background patch is independently known before encoding.
    let b = Image::from_rgba8(1, 1, &[32, 64, 128, 255], false, 13, &hooks)
        .unwrap()
        .pixels()[0];
    let s = Image::from_rgba8(1, 1, &[240, 80, 32, 128], false, 13, &hooks)
        .unwrap()
        .pixels()[0];
    let expected = blend::pixel(b, s, 0.5, BlendMode::Screen);
    for (a, b) in reference.pixels()[48 * 128 + 64].iter().zip(expected) {
        assert!((a - b).abs() < 3e-6);
    }
    let raw = root().join("blend-edge-patch.rgba");
    std::fs::write(&raw, &golden.bytes).unwrap();
    let png = root().join("blend-edge-patch.png");
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
    let png = root().join("blend-edge-patch-decoded.png");
    ffmpeg(&[
        "-i".as_ref(),
        path.as_os_str(),
        "-frames:v".as_ref(),
        "1".as_ref(),
        png.as_os_str(),
    ]);
}

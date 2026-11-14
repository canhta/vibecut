#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
//! Person matte: the stored plane's encoding, its feather, its IoU on annotated fixtures and the
//! picture render draws from it.
use crate::support::{Hooks, tick};
use render::effects::CpuEffects;
use render::matte::{self, CELLS, MatteBuilder, MatteTrack, SIDE};
use render::{
    AudioBlock, ExecutionHooks, Image, MemoryKind, RenderSettings, Renderer, Result,
    SourceProvider, UnavailableReason,
};
use std::sync::Arc;
use time::{SourceRange, Tick};

/// Recorded IoU floors of `coverage >= 0.5` against the annotation, from the person-matte
/// fixtures of the segmentation layer: analytic bounds with margin (boundary error well under a
/// pixel, feather shrinkage of the small head at the widest setting). The run prints the
/// measured values next to them.
const IOU_FEATHER_OFF: f64 = 0.95;
const IOU_FEATHER_25: f64 = 0.95;
const IOU_FEATHER_100: f64 = 0.88;

/// Implicit shape in normalized frame coordinates: negative inside. A head ellipse over a
/// shoulders ellipse that overlap slightly.
fn shape(u: f64, v: f64) -> f64 {
    let ellipse = |cx: f64, cy: f64, rx: f64, ry: f64| {
        (((u - cx) / rx).powi(2) + ((v - cy) / ry).powi(2)).sqrt() - 1.0
    };
    ellipse(0.5, 0.27, 0.10, 0.14).min(ellipse(0.5, 0.72, 0.26, 0.34))
}
/// Ground truth at frame size: pixel centres inside the shape.
fn annotation(width: usize, height: usize) -> Vec<bool> {
    (0..height)
        .flat_map(|y| (0..width).map(move |x| (x, y)))
        .map(|(x, y)| {
            shape(
                (x as f64 + 0.5) / width as f64,
                (y as f64 + 0.5) / height as f64,
            ) < 0.
        })
        .collect()
}
/// The stand-in model output, stored as the engine stores it: a linear ramp across the boundary
/// (about four cells wide) plus seeded noise of at most 0.03, quantized to bytes.
fn person_plane() -> Vec<u8> {
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    let mut values = Vec::with_capacity(CELLS);
    for j in 0..SIDE {
        for i in 0..SIDE {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            let noise = ((state >> 40) as f64 / (1u64 << 24) as f64 - 0.5) * 0.06;
            let g = shape(
                (i as f64 + 0.5) / SIDE as f64,
                (j as f64 + 0.5) / SIDE as f64,
            );
            values.push(((0.5 - g * 12. + noise).clamp(0., 1.) * 255.).round() as u8);
        }
    }
    values
}
/// Coverage of an opaque white `width` x `height` layer after the matte, the layer filling the
/// frame the plane was measured on.
fn coverage(plane: &[u8], feather: f32, width: u32, height: u32) -> Vec<f32> {
    let hooks = Hooks::default();
    let layer = Image::from_rgba8(
        width,
        height,
        &vec![255; (width * height * 4) as usize],
        false,
        13,
        &hooks,
    )
    .unwrap();
    let map = matte::plane_map(width, height, |p| p).unwrap();
    let field = matte::feathered(plane, feather).unwrap();
    let out = matte::apply(layer, &field, map, &CpuEffects, &hooks).unwrap();
    out.pixels().iter().map(|pixel| pixel[3]).collect()
}
fn iou(alpha: &[f32], truth: &[bool]) -> f64 {
    assert_eq!(alpha.len(), truth.len());
    let (mut both, mut either) = (0u64, 0u64);
    for (a, t) in alpha.iter().zip(truth) {
        let a = *a >= 0.5;
        both += u64::from(a && *t);
        either += u64::from(a || *t);
    }
    both as f64 / either as f64
}
fn track(planes: &[(Tick, Vec<u8>)]) -> MatteTrack {
    let mut builder = MatteBuilder::default();
    for (at, plane) in planes {
        builder.push(*at, plane).unwrap();
    }
    builder.finish().unwrap()
}

#[test]
fn a_matte_encodes_to_the_same_bytes_and_reads_back_every_plane() {
    let person = person_plane();
    let ramp: Vec<u8> = (0..CELLS).map(|i| (i % 251) as u8).collect();
    let frames = [
        (Tick::ZERO, person.clone()),
        (tick(1, 30), ramp.clone()),
        (tick(2, 30), vec![0; CELLS]),
        (tick(3, 30), vec![255; CELLS]),
    ];
    let stored = track(&frames);
    let bytes = stored.encode();
    assert_eq!(bytes, track(&frames).encode(), "deterministic encoding");
    let back = MatteTrack::decode(&bytes).unwrap();
    assert_eq!(back, stored);
    for (index, (at, plane)) in frames.iter().enumerate() {
        assert_eq!(back.plane(index).unwrap(), *plane);
        assert_eq!(back.covering(*at), index);
    }
    // A flat plane packs to a few bytes; the person plane to far less than raw.
    assert!(bytes.len() < 2 * CELLS + 4 * 600, "{}", bytes.len());
    // Between frames the earlier one covers; before the first, the first.
    assert_eq!(back.covering(tick(3, 60)), 1);
    assert_eq!(back.covering(Tick(-5)), 0);
    // Truncation, a bad magic, a bad plane stream and disorder are refused.
    assert!(MatteTrack::decode(&bytes[..bytes.len() - 1]).is_err());
    let mut bad = bytes.clone();
    bad[0] = b'X';
    assert!(MatteTrack::decode(&bad).is_err());
    let mut longer = bytes.clone();
    longer.push(0);
    assert!(MatteTrack::decode(&longer).is_err());
    let mut builder = MatteBuilder::default();
    builder.push(tick(1, 30), &person).unwrap();
    assert!(
        builder.push(tick(1, 30), &person).is_err(),
        "times strictly increase"
    );
    assert!(
        builder.push(tick(2, 30), &person[1..]).is_err(),
        "one plane per frame"
    );
    assert!(
        MatteBuilder::default().finish().is_err(),
        "at least one frame"
    );
}

#[test]
fn feather_off_is_the_stored_plane_and_a_feather_widens_the_edge() {
    let plane = person_plane();
    let exact: Vec<f32> = plane.iter().map(|v| f32::from(*v) / 255.0).collect();
    assert_eq!(matte::feathered(&plane, 0.0).unwrap(), exact);
    // At the plane's own size the application reads each cell exactly: feather 0 is the identity.
    let alpha = coverage(&plane, 0.0, SIDE as u32, SIDE as u32);
    for (a, e) in alpha.iter().zip(&exact) {
        assert!((a - e).abs() <= 1e-6, "{a} != {e}");
    }
    let band = |feather: f32| {
        coverage(&plane, feather, 640, 360)
            .iter()
            .filter(|a| **a > 0.002 && **a < 0.998)
            .count()
    };
    let bands = [band(0.0), band(25.0), band(100.0)];
    assert!(bands[0] < bands[1] && bands[1] < bands[2], "{bands:?}");
    assert_ne!(
        matte::feathered(&plane, 0.01).unwrap(),
        matte::feathered(&plane, 0.0).unwrap()
    );
    for feather in [-0.5, 100.5, f32::NAN] {
        assert!(matte::feathered(&plane, feather).is_err());
    }
    assert!(matte::feathered(&plane[1..], 0.0).is_err());
}

#[test]
fn matte_iou_on_annotated_fixtures_meets_the_recorded_thresholds() {
    let plane = person_plane();
    // 160x90 is smaller than one plane, so it takes the in-place path; the rest run the pass.
    for (width, height) in [
        (640usize, 360usize),
        (360, 640),
        (333, 333),
        (1280, 720),
        (160, 90),
    ] {
        let truth = annotation(width, height);
        for (feather, floor) in [
            (0., IOU_FEATHER_OFF),
            (25., IOU_FEATHER_25),
            (100., IOU_FEATHER_100),
        ] {
            let alpha = coverage(&plane, feather, width as u32, height as u32);
            let measured = iou(&alpha, &truth);
            println!(
                "MATTE_IOU {width}x{height} feather={feather} iou={measured:.4} floor={floor}"
            );
            assert!(
                measured >= floor,
                "{width}x{height} feather {feather}: {measured} < {floor}"
            );
            assert_eq!(
                alpha,
                coverage(&plane, feather, width as u32, height as u32),
                "deterministic"
            );
        }
    }
}

/// A source of `SIZE` square frames whose picture is an opaque diagonal ramp.
const SIZE: u32 = 192;
struct Ramp;
impl SourceProvider for Ramp {
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
        hooks: &dyn ExecutionHooks,
    ) -> Result<Option<Image>> {
        let bytes: Vec<u8> = (0..SIZE * SIZE)
            .flat_map(|i| {
                let (x, y) = (i % SIZE, i / SIZE);
                let v = (40 + x / 2 + y / 3) as u8;
                [v, 255 - v, v / 2, 255]
            })
            .collect();
        Ok(Some(Image::from_rgba8(
            SIZE, SIZE, &bytes, false, 13, hooks,
        )?))
    }
    fn audio(
        &mut self,
        _: &media::SourceAsset,
        _: project::ItemId,
        _: i64,
        n: usize,
        rate: u32,
        hooks: &dyn ExecutionHooks,
    ) -> Result<AudioBlock> {
        AudioBlock::new(rate, 2, n, MemoryKind::Decoded, hooks)
    }
}
fn scene(removal: Option<project::BackgroundRemoval>, hidden: bool) -> project::Project {
    let asset = testkit::source::asset(1, std::env::temp_dir().join("render-matte"))
        .format("synthetic")
        .duration(tick(2, 1))
        .stream(
            testkit::source::video()
                .codec("rgba")
                .time_base(media::TimeBase::new(1, 30).unwrap())
                .duration(tick(2, 1))
                .start(Tick::ZERO)
                .frame_count(60)
                .variable_frame_rate(false)
                .size(SIZE, SIZE)
                .build(),
        )
        .build();
    let mut data = project::Project::empty(project::ProjectId(7), "matte".into())
        .unwrap()
        .into_data();
    data.canvas.width = SIZE;
    data.canvas.height = SIZE;
    data.assets.push(asset);
    let mut item = testkit::item::video(project::ItemId(1))
        .asset(media::AssetId(1))
        .source(SourceRange::new(Tick::ZERO, tick(2, 1)).unwrap())
        .build();
    item.hidden = hidden;
    let project::ItemParams::Video(params) = &mut item.content else {
        panic!("video item");
    };
    params.background_removal = removal;
    data.scene_mut().timeline.main.items.push(item);
    project::Project::new(data).unwrap()
}
/// The left half is certainly a person, the right half certainly not.
fn halves() -> Arc<MatteTrack> {
    let plane: Vec<u8> = (0..CELLS)
        .map(|i| if i % SIDE < SIDE / 2 { 255 } else { 0 })
        .collect();
    Arc::new(track(&[(Tick::ZERO, plane)]))
}

#[test]
fn a_matted_item_renders_the_same_in_preview_and_export_and_a_missing_matte_renders_unmatted() {
    let hooks = Hooks::default();
    let render = |project: &project::Project, stored: Option<Arc<MatteTrack>>, preview: bool| {
        let mut settings = RenderSettings::from_project(project);
        if let Some(stored) = stored {
            settings.person_mattes.insert(project::ItemId(1), stored);
        }
        let mut renderer = Renderer::new(project, settings).unwrap();
        if preview {
            renderer
                .preview_frame(Tick::ZERO, &mut Ramp, &hooks)
                .unwrap()
        } else {
            renderer
                .render_frame(Tick::ZERO, &mut Ramp, &hooks)
                .unwrap()
        }
    };
    let on = Some(project::BackgroundRemoval { feather: 0.0 });
    let off = scene(None, false);
    let plain = render(&off, None, false);
    let empty = render(&scene(None, true), None, false);
    // Switched on with no stored matte: never an error, and the picture is the unmatted one.
    assert_eq!(
        render(&scene(on, false), None, false).pixels(),
        plain.pixels()
    );
    // A matte for an item whose switch is off changes nothing.
    assert_eq!(render(&off, Some(halves()), false).pixels(), plain.pixels());
    // Switched on with a matte: the person half keeps the clip, the other half shows what is
    // below (here the canvas), and preview equals export.
    let export = render(&scene(on, false), Some(halves()), false);
    let preview = render(&scene(on, false), Some(halves()), true);
    assert_eq!(export.pixels(), preview.pixels());
    let row = SIZE as usize;
    for y in 0..row {
        for x in [0, row / 4, row / 2 - 1] {
            assert_eq!(export.pixels()[y * row + x], plain.pixels()[y * row + x]);
        }
        for x in [row / 2 + 1, 3 * row / 4, row - 1] {
            assert_eq!(export.pixels()[y * row + x], empty.pixels()[y * row + x]);
        }
    }
    // Two renders from the same inputs are byte-equal.
    assert_eq!(
        render(&scene(on, false), Some(halves()), false).pixels(),
        export.pixels()
    );
    // A feather softens the cut: the column at the boundary is neither clip nor canvas.
    let soft = render(
        &scene(Some(project::BackgroundRemoval { feather: 100.0 }), false),
        Some(halves()),
        false,
    );
    let edge = row / 2;
    assert_ne!(soft.pixels()[edge], export.pixels()[edge]);
}

#[test]
fn a_4k_matte_stays_within_four_layers_of_memory_and_stops_when_cancelled() {
    let (width, height) = (3840u32, 2160u32);
    let layer_bytes = u64::from(width) * u64::from(height) * 16;
    let opaque = || std::iter::repeat_n([1.0f32; 4], (width * height) as usize);
    let hooks = Hooks::default();
    let layer = Image::from_working_pixels(width, height, opaque(), &hooks).unwrap();
    let map = matte::plane_map(width, height, |p| p).unwrap();
    let field = matte::feathered(&person_plane(), 50.0).unwrap();
    let out = matte::apply(layer, &field, map, &CpuEffects, &hooks).unwrap();
    // The layer, the packed plane input and the CPU reference's two working layers, as any
    // effect instance holds, plus bounded scratch.
    let peak = hooks.peak.load(std::sync::atomic::Ordering::SeqCst);
    println!("MATTE_4K peak={peak} layer={layer_bytes}");
    assert!(peak <= 4 * layer_bytes + (16 << 20), "{peak}");
    let at = |x: u32, y: u32| out.pixels()[(y * width + x) as usize][3];
    assert!(at(1920, 1080) > 0.999, "torso centre");
    assert!(at(0, 0) < 0.001, "corner");
    // A cancelled frame stops instead of finishing.
    let cancelled = Hooks::default();
    let layer = Image::from_working_pixels(width, height, opaque(), &cancelled).unwrap();
    cancelled
        .state
        .store(1, std::sync::atomic::Ordering::SeqCst);
    assert!(matches!(
        matte::apply(layer, &field, map, &CpuEffects, &cancelled),
        Err(render::Error::Cancelled)
    ));
}

#[test]
fn a_screen_point_maps_back_to_the_decoded_frame_through_crop_and_fit() {
    let mut data = scene(None, false).into_data();
    let project::ItemParams::Video(params) = &mut data.scene_mut().timeline.main.items[0].content
    else {
        panic!("video item");
    };
    params.fit = project::FitMode::Stretch;
    let project = project::Project::new(data.clone()).unwrap();
    let viewport = render::viewport::Viewport::new(
        render::Rect {
            x: 0.0,
            y: 0.0,
            width: 192.0,
            height: 192.0,
        },
        [SIZE, SIZE],
        1.0,
        [0.0, 0.0],
    )
    .unwrap();
    let renderer = Renderer::new(&project, RenderSettings::from_project(&project)).unwrap();
    let at = |renderer: &Renderer<'_>, screen: [f64; 2]| {
        renderer
            .frame_point(&viewport, project::ItemId(1), Tick::ZERO, screen)
            .unwrap()
            .unwrap()
    };
    let (point, frame) = at(&renderer, [96.0, 48.0]);
    assert_eq!(frame, [SIZE, SIZE]);
    assert!(
        (point[0] - 0.5).abs() < 1e-9 && (point[1] - 0.25).abs() < 1e-9,
        "{point:?}"
    );
    // The right half of the frame fills the canvas: the canvas middle shows frame x 0.75.
    let project::ItemParams::Video(params) = &mut data.scene_mut().timeline.main.items[0].content
    else {
        panic!("video item");
    };
    params.crop = Some(project::Crop {
        x: 0.5,
        y: 0.0,
        width: 0.5,
        height: 1.0,
    });
    let cropped = project::Project::new(data).unwrap();
    let renderer = Renderer::new(&cropped, RenderSettings::from_project(&cropped)).unwrap();
    let (point, _) = at(&renderer, [96.0, 48.0]);
    assert!(
        (point[0] - 0.75).abs() < 1e-9 && (point[1] - 0.25).abs() < 1e-9,
        "{point:?}"
    );
}

#[test]
fn matte_strokes_add_and_remove_over_the_computed_matte_on_their_own_frame_only() {
    use project::{MatteStroke, MatteStrokeMode as Mode};
    let hooks = Hooks::default();
    let on = Some(project::BackgroundRemoval { feather: 0.0 });
    let two_frames = || {
        let plane: Vec<u8> = (0..CELLS)
            .map(|i| if i % SIDE < SIDE / 2 { 255 } else { 0 })
            .collect();
        Arc::new(track(&[(Tick::ZERO, plane.clone()), (tick(1, 1), plane)]))
    };
    let render = |strokes: Vec<MatteStroke>| {
        let mut data = scene(on, false).into_data();
        let project::ItemParams::Video(params) =
            &mut data.scene_mut().timeline.main.items[0].content
        else {
            panic!("video item");
        };
        params.matte_strokes = strokes;
        let project = project::Project::new(data).unwrap();
        let mut settings = RenderSettings::from_project(&project);
        settings
            .person_mattes
            .insert(project::ItemId(1), two_frames());
        Renderer::new(&project, settings)
            .unwrap()
            .render_frame(Tick::ZERO, &mut Ramp, &hooks)
            .unwrap()
    };
    let stroke = |mode, at, x: f32| MatteStroke {
        mode,
        at,
        radius: 0.1,
        points: vec![[x, 0.5], [x, 0.6]],
    };
    let row = SIZE as usize;
    // A pixel on the tip's axis, away from its antialiased edge.
    let probe = |image: &Image, x: usize| image.pixels()[(row / 2 + 4) * row + x];
    let plain = render(Vec::new());
    let (person, away) = (row / 4, 3 * row / 4);
    // Add paints the person into the empty half; the picture there is the clip's own.
    let added = render(vec![stroke(Mode::Add, Tick::ZERO, 0.75)]);
    assert_ne!(probe(&added, away), probe(&plain, away));
    assert_eq!(probe(&added, person), probe(&plain, person));
    // Remove clears the person half under the tip and nothing outside it.
    let removed = render(vec![stroke(Mode::Remove, Tick::ZERO, 0.25)]);
    assert_ne!(probe(&removed, person), probe(&plain, person));
    assert_eq!(probe(&removed, away), probe(&plain, away));
    assert_eq!(removed.pixels()[0], plain.pixels()[0]);
    // The later stroke wins where two overlap, and the same strokes render the same bytes.
    let remove_then_add = render(vec![
        stroke(Mode::Remove, Tick::ZERO, 0.25),
        stroke(Mode::Add, Tick::ZERO, 0.25),
    ]);
    assert_eq!(probe(&remove_then_add, person), probe(&plain, person));
    let add_then_remove = render(vec![
        stroke(Mode::Add, Tick::ZERO, 0.25),
        stroke(Mode::Remove, Tick::ZERO, 0.25),
    ]);
    assert_eq!(probe(&add_then_remove, person), probe(&removed, person));
    assert_eq!(
        render(vec![stroke(Mode::Add, Tick::ZERO, 0.75)]).pixels(),
        added.pixels()
    );
    // A stroke painted on another matte frame leaves this frame untouched.
    let elsewhere = render(vec![stroke(Mode::Add, tick(1, 1), 0.75)]);
    assert_eq!(elsewhere.pixels(), plain.pixels());
}

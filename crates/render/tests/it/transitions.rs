#![allow(clippy::unwrap_used, clippy::indexing_slicing)]
use super::support::{Empty, Hooks};
use project::*;
use render::*;
use render::{Error, Result};
use std::sync::atomic::{AtomicUsize, Ordering};
use time::{SourceRange, Tick};

/// One row per catalogue kind: a 4x4 grid of mean 8-bit RGB at progress 0.25, 0.5 and 0.75.
const ORACLE: &[(&str, [&str; 3])] = &[];

/// The 4x4 mean-RGB fingerprint at the midpoint of the red-to-blue scene, for the kinds whose
/// midpoint varies across the frame (the uniform kinds record an exact pixel in `midpoint`).
const MIDPOINTS: &[(&str, &str)] = &[];

fn scene(kind: &str) -> Project {
    let mut d = Project::empty(ProjectId(0), "transition goldens".into())
        .unwrap()
        .into_data();
    d.canvas.width = 4;
    d.canvas.height = 4;
    d.scene_mut().timeline.main.items = [(10, 0, [255, 0, 0, 255]), (11, 100, [0, 0, 255, 255])]
        .into_iter()
        .map(|(id, start, c)| Item {
            hidden: false,
            retime: Default::default(),
            id: ItemId(id),
            source_asset: None,
            source_range: SourceRange::new(Tick(100), Tick(200)).unwrap(),
            start: Tick(start),
            content: ItemParams::Overlay(OverlayParams { color: Color(c) }),
            animations: Default::default(),
            effects: vec![],
            masks: vec![],
            blend_mode: Default::default(),
        })
        .collect();
    d.scene_mut().timeline.transitions.push(Transition {
        from_pieces: vec![],
        to_pieces: vec![],
        id: TransitionId(1),
        track: TrackId(0),
        from: ItemId(10),
        to: ItemId(11),
        kind: kind.into(),
        start: Tick(80),
        duration: Tick(40),
        alignment: TransitionAlignment::Center,
        params: Default::default(),
    });
    Project::new(d).unwrap()
}

/// The exact midpoint frame between pure red and pure blue, in linear light, where the frame is
/// one uniform colour; `None` when the kind varies across the frame, whose midpoint fingerprint
/// is recorded in `MIDPOINTS`.
fn midpoint(kind: &str) -> Option<[f32; 4]> {
    let linear = |v: u8| {
        let c = f32::from(v) / 255.;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    match kind {
        "cross_dissolve" => Some([0.5, 0., 0.5, 1.]),
        "dip_to_black" => Some([0., 0., 0., 1.]),
        "dip_to_white" => Some([1.; 4]),
        "dip_to_color" => Some([linear(217), linear(51), linear(51), 1.]),
        _ => None,
    }
}

#[test]
fn viewport_hit_test_selects_visible_transition_contributors_in_source_handles() {
    for definition in TRANSITION_DEFINITIONS {
        let project = scene(definition.kind);
        let renderer = Renderer::new(&project, RenderSettings::from_project(&project)).unwrap();
        let viewport = render::viewport::Viewport::new(
            Rect {
                x: 0.,
                y: 0.,
                width: 4.,
                height: 4.,
            },
            [4, 4],
            1.,
            [0., 0.],
        )
        .unwrap();
        for at in [79, 80, 81, 99, 100, 101, 119, 120] {
            let expected = if definition.kind.starts_with("dip_") {
                if at < 100 {
                    Some(ItemId(10))
                } else if at == 100 {
                    None
                } else {
                    Some(ItemId(11))
                }
            } else if at <= 80 {
                Some(ItemId(10))
            } else {
                Some(ItemId(11))
            };
            assert_eq!(
                renderer.hit_test(&viewport, [2., 2.], Tick(at)).unwrap(),
                expected,
                "{} at {at}",
                definition.kind
            );
            if let Some(id) = expected {
                assert!(renderer.item_geometry(id, Tick(at)).unwrap().is_some());
            }
        }
    }
}

#[test]
fn every_catalog_kind_has_start_mid_end_goldens_without_flash_or_frame_drop() {
    for def in TRANSITION_DEFINITIONS {
        let p = scene(def.kind);
        let h = Hooks::default();
        let mut r = Renderer::new(&p, RenderSettings::from_project(&p)).unwrap();
        for (at, expected) in [
            (79, [1., 0., 0., 1.]),
            (80, [1., 0., 0., 1.]),
            (120, [0., 0., 1., 1.]),
            (121, [0., 0., 1., 1.]),
        ] {
            let im = r.render_frame(Tick(at), &mut Empty, &h).unwrap();
            assert!(
                im.pixels()
                    .iter()
                    .all(|p| p.iter().zip(expected).all(|(a, b)| (a - b).abs() < 1e-6)),
                "{} at {at}",
                def.kind
            );
        }
        let im = r.render_frame(Tick(100), &mut Empty, &h).unwrap();
        match midpoint(def.kind) {
            Some(expected) => assert!(
                im.pixels()
                    .iter()
                    .all(|p| p.iter().zip(expected).all(|(a, b)| (a - b).abs() < 1e-6)),
                "{} at 100",
                def.kind
            ),
            None => {
                let recorded = MIDPOINTS
                    .iter()
                    .find(|(kind, _)| *kind == def.kind)
                    .unwrap_or_else(|| panic!("{}: add its midpoint fingerprint", def.kind));
                assert_eq!(&fingerprint(&im), recorded.1, "{} at 100", def.kind);
            }
        }
        // Every tick around and across the half-open span yields opaque pixels;
        // the final in-span frame converges to the exact incoming endpoint.
        let mut previous = None;
        for at in 79..=121 {
            let im = r.render_frame(Tick(at), &mut Empty, &h).unwrap();
            assert!(im.pixels().iter().all(|p| p[3] == 1.));
            if let Some(prev) = previous {
                assert!(
                    im.pixels()[0]
                        .iter()
                        .zip(prev)
                        .all(|(a, b): (&f32, f32)| (a - b).abs() <= 0.051)
                );
            }
            previous = Some(im.pixels()[0]);
        }
    }
}

#[test]
fn item_effects_masks_opacity_and_muted_hidden_tracks_compose_before_transition() {
    let p = scene("cross_dissolve");
    let mut d = p.into_data();
    for item in &mut d.scene_mut().timeline.main.items {
        item.effects.push(
            effect_definition("blur")
                .unwrap()
                .instance("blur")
                .with("intensity", EffectValue::Scalar(10.))
                .unwrap(),
        );
        item.masks
            .push(Mask::new("rectangle", MaskShape::Rectangle));
        item.animations
            .set(
                AnimationPath::new("opacity").unwrap(),
                Param::new(ParamValue::Scalar(0.5)).unwrap(),
            )
            .unwrap();
    }
    d.scene_mut().timeline.main.muted = true;
    let p = Project::new(d).unwrap();
    let h = Hooks::default();
    let im = Renderer::new(&p, RenderSettings::from_project(&p))
        .unwrap()
        .render_frame(Tick(100), &mut Empty, &h)
        .unwrap();
    // Compare the midpoint with independent evaluations of each already-filtered
    // side over the same backdrop, through the ordinary item compositor.
    let mut sides = Vec::new();
    for id in [10, 11] {
        let mut d = p.clone().into_data();
        d.scene_mut().timeline.transitions.clear();
        d.scene_mut()
            .timeline
            .main
            .items
            .retain(|i| i.id == ItemId(id));
        let side = &mut d.scene_mut().timeline.main.items[0];
        side.start = Tick(0);
        let side = Project::new(d).unwrap();
        sides.push(
            Renderer::new(&side, RenderSettings::from_project(&side))
                .unwrap()
                .render_frame(Tick(50), &mut Empty, &h)
                .unwrap(),
        );
    }
    for ((actual, a), b) in im
        .pixels()
        .iter()
        .zip(sides[0].pixels())
        .zip(sides[1].pixels())
    {
        assert!(
            actual
                .iter()
                .zip(a.iter().zip(b))
                .all(|(d, (a, b))| (d - (a + b) * 0.5).abs() < 1e-6)
        );
    }
    let mut d = p.into_data();
    d.scene_mut().timeline.main.hidden = true;
    let hidden = Project::new(d).unwrap();
    let im = Renderer::new(&hidden, RenderSettings::from_project(&hidden))
        .unwrap()
        .render_frame(Tick(100), &mut Empty, &h)
        .unwrap();
    assert!(im.pixels().iter().all(|p| *p == [0., 0., 0., 1.]));
}

struct Cancel {
    checks: AtomicUsize,
    cancel_at: usize,
    inner: Hooks,
}
impl ExecutionHooks for Cancel {
    fn cancelled(&self) -> bool {
        self.checks.fetch_add(1, Ordering::SeqCst) >= self.cancel_at
    }
    fn reserve(&self, kind: MemoryKind, bytes: u64) -> Result<Box<dyn Reservation>> {
        self.inner.reserve(kind, bytes)
    }
}
#[test]
fn cancellation_between_sides_drops_all_leases_and_next_frame_succeeds() {
    let p = scene("cross_dissolve");
    let mut r = Renderer::new(&p, RenderSettings::from_project(&p)).unwrap();
    let baseline = Cancel {
        checks: AtomicUsize::new(0),
        cancel_at: usize::MAX,
        inner: Hooks::default(),
    };
    drop(r.render_frame(Tick(100), &mut Empty, &baseline).unwrap());
    let checkpoints = baseline.checks.load(Ordering::SeqCst);
    for cancel_at in 0..checkpoints {
        let h = Cancel {
            checks: AtomicUsize::new(0),
            cancel_at,
            inner: Hooks::default(),
        };
        assert!(matches!(
            r.render_frame(Tick(100), &mut Empty, &h),
            Err(Error::Cancelled)
        ));
        assert_eq!(h.inner.used.load(Ordering::SeqCst), 0);
    }
    let h = Hooks::default();
    drop(r.render_frame(Tick(100), &mut Empty, &h).unwrap());
    assert_eq!(h.used.load(Ordering::SeqCst), 0);
}

#[test]
fn transition_consumes_both_video_handles_at_exact_source_ticks() {
    struct Sources {
        calls: Vec<(ItemId, Tick)>,
    }
    impl SourceProvider for Sources {
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
            id: ItemId,
            at: Tick,
            h: &dyn ExecutionHooks,
        ) -> Result<Option<Image>> {
            self.calls.push((id, at));
            Ok(Some(Image::new(
                4,
                4,
                if id == ItemId(10) {
                    [1., 0., 0., 1.]
                } else {
                    [0., 0., 1., 1.]
                },
                h,
            )?))
        }
        fn audio(
            &mut self,
            _: &media::SourceAsset,
            _: ItemId,
            _: i64,
            n: usize,
            rate: u32,
            h: &dyn ExecutionHooks,
        ) -> Result<AudioBlock> {
            AudioBlock::new(rate, 2, n, MemoryKind::Decoded, h)
        }
    }
    let mut d = scene("cross_dissolve").into_data();
    d.assets.push(
        testkit::source::asset(1, std::env::temp_dir().join("transition-pixels"))
            .format("synthetic")
            .duration(Tick(1000))
            .stream(
                testkit::source::video()
                    .codec("rgba")
                    .time_base(media::TimeBase::new(1, 30).unwrap())
                    .duration(Tick(1000))
                    .size(4, 4)
                    .build(),
            )
            .build(),
    );
    for i in &mut d.scene_mut().timeline.main.items {
        i.source_asset = Some(media::AssetId(1));
        i.content = ItemParams::Video(VideoParams {
            source_audio_enabled: false,
            volume: 0.,
            muted: false,
            fades: Default::default(),
            crop: None,
        });
    }
    let p = Project::new(d).unwrap();
    let h = Hooks::default();
    let mut r = Renderer::new(&p, RenderSettings::from_project(&p)).unwrap();
    let mut sources = Sources { calls: vec![] };
    for (at, calls) in [
        (80, vec![(ItemId(10), Tick(180)), (ItemId(11), Tick(80))]),
        (100, vec![(ItemId(10), Tick(200)), (ItemId(11), Tick(100))]),
        (120, vec![(ItemId(11), Tick(120))]),
    ] {
        sources.calls.clear();
        drop(r.render_frame(Tick(at), &mut sources, &h).unwrap());
        assert_eq!(sources.calls, calls);
    }
    for hidden_side in [10, 11] {
        let mut d = p.clone().into_data();
        d.scene_mut()
            .timeline
            .main
            .items
            .iter_mut()
            .find(|i| i.id == ItemId(hidden_side))
            .unwrap()
            .animations
            .set(
                AnimationPath::new("opacity").unwrap(),
                Param::new(ParamValue::Scalar(0.)).unwrap(),
            )
            .unwrap();
        let p = Project::new(d).unwrap();
        let mut r = Renderer::new(&p, RenderSettings::from_project(&p)).unwrap();
        sources.calls.clear();
        let frame = r.render_frame(Tick(100), &mut sources, &h).unwrap();
        assert_eq!(sources.calls.len(), 1);
        assert_ne!(sources.calls[0].0, ItemId(hidden_side));
        let expected = if hidden_side == 10 {
            [0., 0., 0.5, 1.]
        } else {
            [0.5, 0., 0., 1.]
        };
        assert!(frame.pixels().iter().all(|p| *p == expected));
    }
}

fn frame(width: u32, height: u32, outgoing: bool) -> Image {
    let pixels = (0..width * height).map(|n| {
        let (x, y) = (
            (n % width) as f32 / width as f32,
            (n / width) as f32 / height as f32,
        );
        if outgoing {
            [0.2 + 0.7 * x, 0.1 + 0.3 * y, 0.05, 1.]
        } else {
            [0.05, 0.2 + 0.2 * y, 0.4 + 0.5 * x, 1.]
        }
    });
    Image::from_working_pixels(width, height, pixels, &Hooks::default()).unwrap()
}
/// The default parameters plus each parameter alone at its bounds or every choice.
fn variants(definition: &TransitionDefinition) -> Vec<TransitionParams> {
    let mut all = vec![TransitionParams::new()];
    for param in definition.params {
        let values = match param.kind {
            TransitionParamKind::Float { min, max, .. } => {
                vec![TransitionValue::Float(min), TransitionValue::Float(max)]
            }
            TransitionParamKind::Choice { options, .. } => (0..options.len() as u32)
                .map(TransitionValue::Choice)
                .collect(),
            TransitionParamKind::Bool { .. } => {
                vec![TransitionValue::Bool(false), TransitionValue::Bool(true)]
            }
            TransitionParamKind::Color { .. } => vec![
                TransitionValue::Color(Color([0; 4])),
                TransitionValue::Color(Color([255; 4])),
            ],
            TransitionParamKind::Point { .. } => vec![
                TransitionValue::Point([0., 0.]),
                TransitionValue::Point([1., 1.]),
            ],
            TransitionParamKind::Angle { .. } => vec![
                TransitionValue::Angle(-1440.),
                TransitionValue::Angle(1440.),
            ],
            TransitionParamKind::Seed { .. } => {
                vec![TransitionValue::Seed(0), TransitionValue::Seed(9999)]
            }
        };
        for value in values {
            all.push(TransitionParams::from([(param.id.to_owned(), value)]));
        }
    }
    all
}
fn mean_abs(a: &Image, b: &Image) -> f32 {
    a.pixels()
        .iter()
        .zip(b.pixels())
        .flat_map(|(x, y)| x.iter().zip(y).map(|(x, y)| (x - y).abs()))
        .sum::<f32>()
        / (a.pixels().len() * 4) as f32
}

#[test]
fn every_registered_kind_meets_the_endpoint_finite_deterministic_continuous_contract() {
    let mut registered: Vec<_> = transitions::registered_kinds().collect();
    let mut catalogue: Vec<_> = TRANSITION_DEFINITIONS.iter().map(|d| d.kind).collect();
    registered.sort_unstable();
    catalogue.sort_unstable();
    assert_eq!(
        registered, catalogue,
        "catalogue and dispatcher list one set"
    );
    let h = Hooks::default();
    for definition in TRANSITION_DEFINITIONS {
        for params in variants(definition) {
            let case = format!("{} {params:?}", definition.kind);
            for (w, ht) in [(16, 9), (1, 1), (1, 7), (7, 1), (2, 2)] {
                let (a, b) = (frame(w, ht, true), frame(w, ht, false));
                let run = |p: f64, scale: f32| {
                    transitions::apply(definition.kind, &params, &a, &b, p, scale, &h)
                        .unwrap_or_else(|error| panic!("{case}: {error:?}"))
                };
                assert_eq!(run(0., 1.).pixels(), a.pixels(), "{case} p=0 is outgoing");
                assert_eq!(run(1., 1.).pixels(), b.pixels(), "{case} p=1 is incoming");
                assert_eq!(run(-3., 1.).pixels(), a.pixels(), "{case} clamps below");
                assert_eq!(run(3., 1.).pixels(), b.pixels(), "{case} clamps above");
                for scale in [1., 0.5, 4.] {
                    for p in [0.001, 0.25, 0.5, 0.75, 0.999] {
                        let image = run(p, scale);
                        assert_eq!((image.width(), image.height()), (w, ht));
                        assert!(
                            image.pixels().iter().all(|px| px
                                .iter()
                                .all(|c| c.is_finite() && (-1e-6..=1. + 1e-6).contains(c))
                                && px[..3].iter().all(|c| *c <= px[3] + 1e-6)),
                            "{case} {w}x{ht} p={p} scale={scale}"
                        );
                        assert_eq!(
                            image.pixels(),
                            run(p, scale).pixels(),
                            "{case} deterministic"
                        );
                    }
                }
                assert!(
                    mean_abs(&run(0.001, 1.), &a) < 0.02,
                    "{case} continuous at 0"
                );
                assert!(
                    mean_abs(&run(0.999, 1.), &b) < 0.02,
                    "{case} continuous at 1"
                );
            }
        }
    }
}

#[test]
fn dispatcher_refuses_hostile_inputs_and_cancels_without_leaking() {
    let h = Hooks::default();
    let (a, b) = (frame(8, 8, true), frame(8, 8, false));
    let none = TransitionParams::new();
    let wrong_type = TransitionParams::from([("hold".to_owned(), TransitionValue::Bool(true))]);
    let unknown = TransitionParams::from([("nope".to_owned(), TransitionValue::Bool(true))]);
    let short = frame(8, 4, false);
    for (kind, params, other, p, scale) in [
        ("spin_motion", &none, &b, 0.5, 1.),
        ("cross_dissolve", &none, &short, 0.5, 1.),
        ("cross_dissolve", &none, &b, f64::NAN, 1.),
        ("cross_dissolve", &none, &b, f64::INFINITY, 1.),
        ("cross_dissolve", &none, &b, 0.5, 0.),
        ("cross_dissolve", &none, &b, 0.5, f32::NAN),
        ("cross_dissolve", &none, &b, 0.5, -1.),
        ("dip_to_color", &wrong_type, &b, 0.5, 1.),
    ] {
        assert!(
            transitions::apply(kind, params, &a, other, p, scale, &h).is_err(),
            "{kind} {p} {scale}"
        );
    }
    // A stray override never changes a kind that does not read it.
    transitions::apply("cross_dissolve", &unknown, &a, &b, 0.5, 1., &h).unwrap();
    for kind in transitions::registered_kinds() {
        let hooks = Cancel {
            checks: AtomicUsize::new(0),
            cancel_at: 0,
            inner: Hooks::default(),
        };
        let (a, b) = (frame(64, 64, true), frame(64, 64, false));
        let result = transitions::apply(kind, &none, &a, &b, 0.5, 1., &hooks);
        assert!(matches!(result, Err(Error::Cancelled)), "{kind}");
        assert_eq!(hooks.inner.used.load(Ordering::SeqCst), 0);
    }
    for (kind, p, sides) in [
        ("cross_dissolve", 0., (true, false)),
        ("cross_dissolve", 0.5, (true, true)),
        ("dip_to_black", 0.49, (true, false)),
        ("dip_to_white", 0.5, (false, false)),
        ("dip_to_color", 0.51, (false, true)),
    ] {
        assert_eq!(
            transitions::exposed_sides(kind, &none, p).unwrap(),
            sides,
            "{kind} {p}"
        );
    }
    let hold = TransitionParams::from([("hold".to_owned(), TransitionValue::Float(50.))]);
    assert_eq!(
        transitions::exposed_sides("dip_to_color", &hold, 0.4).unwrap(),
        (false, false)
    );
}

#[test]
fn thumbnails_are_generated_bounded_shared_and_evicted() {
    let h = Hooks::default();
    let cache = transition_thumbnail::ThumbnailCache::new(2);
    let none = TransitionParams::new();
    let first = cache
        .thumbnail("cross_dissolve", &none, (32, 18), 0.5, &h)
        .unwrap();
    assert_eq!((first.width(), first.height()), (32, 18));
    let again = cache
        .thumbnail("cross_dissolve", &none, (32, 18), 0.5, &h)
        .unwrap();
    assert!(
        std::sync::Arc::ptr_eq(&first, &again),
        "same key shares one image"
    );
    let other = cache
        .thumbnail("dip_to_black", &none, (32, 18), 0.5, &h)
        .unwrap();
    assert_ne!(first.pixels(), other.pixels());
    let start = cache
        .thumbnail("cross_dissolve", &none, (32, 18), 0., &h)
        .unwrap();
    let end = cache
        .thumbnail("cross_dissolve", &none, (32, 18), 1., &h)
        .unwrap();
    assert_ne!(start.pixels(), end.pixels(), "the two frames differ");
    // Capacity two: the oldest entry was dropped, so the same key renders a new image.
    let rerun = cache
        .thumbnail("cross_dissolve", &none, (32, 18), 0.5, &h)
        .unwrap();
    assert!(!std::sync::Arc::ptr_eq(&first, &rerun));
    assert_eq!(first.pixels(), rerun.pixels());
    for size in [(0, 9), (9, 0), (513, 9), (9, 513)] {
        assert!(
            cache
                .thumbnail("cross_dissolve", &none, size, 0.5, &h)
                .is_err()
        );
    }
    for p in [f32::NAN, f32::INFINITY] {
        assert!(
            cache
                .thumbnail("cross_dissolve", &none, (4, 4), p, &h)
                .is_err()
        );
    }
    assert!(
        cache
            .thumbnail("spin_motion", &none, (4, 4), 0.5, &h)
            .is_err()
    );
    drop((first, again, other, start, end, rerun));
    drop(cache);
    assert_eq!(
        h.used.load(Ordering::SeqCst),
        0,
        "no lease outlives the cache"
    );
}

/// A 4x4 grid of mean 8-bit RGB per cell, as 96 hex characters: one oracle row per progress.
fn fingerprint(image: &Image) -> String {
    let h = Hooks::default();
    let bytes = image.rgba8(13, &h).unwrap();
    let (width, height) = (image.width() as usize, image.height() as usize);
    let mut sums = [[[0u64; 3]; 4]; 4];
    let mut counts = [[0u64; 4]; 4];
    for (n, px) in bytes.bytes.chunks_exact(4).enumerate() {
        let (x, y) = (n % width, n / width);
        let (column, row) = (x * 4 / width, y * 4 / height);
        counts[row][column] += 1;
        for (channel, value) in sums[row][column].iter_mut().zip(px) {
            *channel += u64::from(*value);
        }
    }
    let mut out = String::with_capacity(96);
    for row in 0..4 {
        for column in 0..4 {
            let count = counts[row][column].max(1);
            for channel in &sums[row][column] {
                out.push_str(&format!("{:02x}", (channel / count).min(255) as u8));
            }
        }
    }
    out
}

/// The report's two procedural frames, premultiplied and opaque.
fn oracle_frames(width: u32, height: u32) -> (Image, Image) {
    let frame = |outgoing: bool| {
        let pixels = (0..width * height).map(|n| {
            let x = (n % width) as f32 + 0.5;
            let y = (n / width) as f32 + 0.5;
            let (w, h) = (width as f32, height as f32);
            if outgoing {
                let cell = (w / 8.).max(1.);
                let checker = if ((x / cell).floor() + (y / cell).floor()) % 2. == 0. {
                    0.15
                } else {
                    0.
                };
                [
                    (0.2 + 0.7 * x / w + checker).min(1.),
                    0.1 + 0.3 * y / h,
                    0.05 + checker,
                    1.,
                ]
            } else {
                let disc = if (x - w * 0.62).hypot(y - h * 0.4) < h * 0.22 {
                    0.6
                } else {
                    0.
                };
                [
                    (0.05f32 + disc).min(1.),
                    (0.2 + 0.2 * y / h + disc).min(1.),
                    0.4 + 0.5 * y / h,
                    1.,
                ]
            }
        });
        Image::from_working_pixels(width, height, pixels, &Hooks::default()).unwrap()
    };
    (frame(true), frame(false))
}

#[test]
fn every_kind_matches_its_recorded_oracle_row_at_three_progress_points() {
    let h = Hooks::default();
    let (a, b) = oracle_frames(128, 72);
    for definition in TRANSITION_DEFINITIONS {
        let row = ORACLE
            .iter()
            .find(|(kind, _)| *kind == definition.kind)
            .unwrap_or_else(|| panic!("{}: add its oracle row", definition.kind));
        for (progress, expected) in [0.25f64, 0.5, 0.75].into_iter().zip(row.1) {
            let image = transitions::apply(
                definition.kind,
                &TransitionParams::new(),
                &a,
                &b,
                progress,
                1.,
                &h,
            )
            .unwrap();
            assert_eq!(&fingerprint(&image), expected, "{} at {progress}", definition.kind);
        }
    }
    assert_eq!(ORACLE.len(), TRANSITION_DEFINITIONS.len());
}

#[test]
#[ignore = "prints the oracle table after an intended change"]
fn print_oracle_rows() {
    let h = Hooks::default();
    let (a, b) = oracle_frames(128, 72);
    for definition in TRANSITION_DEFINITIONS {
        let prints: Vec<_> = [0.25f64, 0.5, 0.75]
            .into_iter()
            .map(|progress| {
                format!(
                    "\"{}\"",
                    fingerprint(
                        &transitions::apply(
                            definition.kind,
                            &TransitionParams::new(),
                            &a,
                            &b,
                            progress,
                            1.,
                            &h,
                        )
                        .unwrap(),
                    )
                )
            })
            .collect();
        println!("    (\"{}\", [{}]),", definition.kind, prints.join(", "));
    }
    for definition in TRANSITION_DEFINITIONS {
        if midpoint(definition.kind).is_some() {
            continue;
        }
        let p = scene(definition.kind);
        let h = Hooks::default();
        let mut r = Renderer::new(&p, RenderSettings::from_project(&p)).unwrap();
        let im = r.render_frame(Tick(100), &mut Empty, &h).unwrap();
        println!("    (\"{}\", \"{}\"),", definition.kind, fingerprint(&im));
    }
}

/// A value pair that must move the picture away from the catalogue default.
fn changed_values(param: &TransitionParamDef) -> Vec<TransitionValue> {
    let different = |v: f64, default: f64| (v - default).abs() > 1e-9;
    match param.kind {
        TransitionParamKind::Float { min, max, default, .. } => [
            default + (max - default) * 0.5,
            default - (default - min) * 0.5,
            max,
            min,
        ]
        .into_iter()
        .filter(|v| different(*v, default))
        .map(TransitionValue::Float)
        .collect(),
        TransitionParamKind::Choice { options, default } => (0..options.len() as u32)
            .filter(|n| *n != default)
            .map(TransitionValue::Choice)
            .collect(),
        TransitionParamKind::Bool { default } => vec![TransitionValue::Bool(!default)],
        TransitionParamKind::Color { .. } => vec![
            TransitionValue::Color(Color([0, 0, 0, 255])),
            TransitionValue::Color(Color([255, 0, 255, 255])),
        ],
        TransitionParamKind::Point { default } => [[0., 0.], [1., 1.]]
            .into_iter()
            .filter(|v| v != &default)
            .map(TransitionValue::Point)
            .collect(),
        TransitionParamKind::Angle { default } => [default + 70., default - 70.]
            .into_iter()
            .map(TransitionValue::Angle)
            .collect(),
        TransitionParamKind::Seed { default } => [default.wrapping_add(1).min(9999), 0, 9999]
            .into_iter()
            .filter(|v| *v != default)
            .map(TransitionValue::Seed)
            .collect(),
    }
}

#[test]
fn every_parameter_changes_the_picture() {
    let h = Hooks::default();
    let (a, b) = oracle_frames(160, 90);
    let progresses = [0.3f64, 0.5, 0.7];
    let render = |kind: &str, params: &TransitionParams| {
        progresses
            .map(|progress| {
                transitions::apply(kind, params, &a, &b, progress, 1., &h).unwrap_or_else(|error| {
                    panic!("{kind} {progress}: {error:?}")
                })
            })
    };
    for definition in TRANSITION_DEFINITIONS {
        let base = render(definition.kind, &TransitionParams::new());
        for param in definition.params {
            let mut best = 0f32;
            for value in changed_values(param) {
                let mut params = TransitionParams::from([(param.id.to_owned(), value)]);
                if param.id == "border_color" {
                    params.insert("border_width".into(), TransitionValue::Float(12.));
                }
                let frames = render(definition.kind, &params);
                for (frame, base) in frames.iter().zip(&base) {
                    best = best.max(mean_abs(frame, base));
                }
            }
            assert!(
                best > 1e-5,
                "{} {} never changes the picture ({best})",
                definition.kind,
                param.id
            );
        }
    }
}

/// Prints median wall time per catalogue kind at 1920x1080/30 fps, five reps after one warm-up.
#[test]
#[ignore = "cost report, not a gate"]
fn transition_cost_at_1080p() {
    let h = Hooks::default();
    let (a, b) = oracle_frames(1920, 1080);
    let mut rows: Vec<(f64, &str)> = Vec::new();
    for definition in TRANSITION_DEFINITIONS {
        let mut run = || {
            transitions::apply(
                definition.kind,
                &TransitionParams::new(),
                &a,
                &b,
                0.5,
                1.,
                &h,
            )
            .unwrap();
        };
        run();
        let mut samples: Vec<f64> = (0..5)
            .map(|_| {
                let start = std::time::Instant::now();
                run();
                start.elapsed().as_secs_f64() * 1000.
            })
            .collect();
        samples.sort_by(|a, b| a.total_cmp(b));
        rows.push((samples[2], definition.kind));
    }
    rows.sort_by(|a, b| b.0.total_cmp(&a.0));
    for (ms, kind) in &rows {
        println!("{ms:8.2} ms  {kind}");
    }
}

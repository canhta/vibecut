#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
//! Person matte: synthetic annotated fixtures, feather identity, bounds and cancellation.
//!
//! The fixtures stand in for the model output, not for the model: a deterministic
//! head-and-shoulders annotation is rendered at full frame size (the ground truth)
//! and, with a seeded perturbation, as the 192x192 probability plane the graph
//! would emit. The matte path under test turns that plane back into frame-size
//! alpha; the model's own accuracy is the ignored real-weights test below.
use ai::{
    Error, ExecutionHooks, Stage,
    vision::{MATTE_SIDE, PersonFrame, PersonMatte},
};
use std::sync::atomic::{AtomicUsize, Ordering};

/// Recorded IoU floors of `alpha >= 128` against the annotation. They are
/// analytic bounds chosen with margin (boundary error well under a pixel, feather
/// shrinkage of the small head at the widest setting); the combined build prints
/// the measured values next to them and a lower measurement lowers nothing here.
const IOU_FEATHER_OFF: f64 = 0.95;
const IOU_FEATHER_25: f64 = 0.95;
const IOU_FEATHER_100: f64 = 0.88;
/// Upstream model card (PP-HumanSeg, OpenCV Zoo eval): mIoU on its own set.
const MODEL_CARD_MIOU: f64 = 0.9164;

struct Hooks;
impl ExecutionHooks for Hooks {
    fn cancelled(&self) -> bool {
        false
    }
    fn stage(&self, _: Stage) {}
}
struct Cancelled;
impl ExecutionHooks for Cancelled {
    fn cancelled(&self) -> bool {
        true
    }
    fn stage(&self, _: Stage) {}
}
/// Cancels once `limit` stage reports have been made.
struct CancelAfter {
    stages: AtomicUsize,
    limit: usize,
}
impl ExecutionHooks for CancelAfter {
    fn cancelled(&self) -> bool {
        self.stages.load(Ordering::SeqCst) >= self.limit
    }
    fn stage(&self, _: Stage) {
        self.stages.fetch_add(1, Ordering::SeqCst);
    }
}

/// Implicit shape in normalized frame coordinates: negative inside, zero on the
/// boundary. A head ellipse over a shoulders ellipse that overlap slightly.
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
/// The stand-in model output: a linear ramp across the boundary (about four cells
/// wide) plus seeded noise of at most 0.03.
fn plane() -> PersonMatte {
    let side = MATTE_SIDE;
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    let mut values = Vec::with_capacity(side * side);
    for j in 0..side {
        for i in 0..side {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            let noise = ((state >> 40) as f64 / (1u64 << 24) as f64 - 0.5) * 0.06;
            let g = shape(
                (i as f64 + 0.5) / side as f64,
                (j as f64 + 0.5) / side as f64,
            );
            values.push((0.5 - g * 12. + noise).clamp(0., 1.) as f32);
        }
    }
    PersonMatte::new(values).unwrap()
}
fn iou(alpha: &[u8], truth: &[bool]) -> f64 {
    assert_eq!(alpha.len(), truth.len());
    let (mut both, mut either) = (0u64, 0u64);
    for (a, t) in alpha.iter().zip(truth) {
        let a = *a >= 128;
        both += u64::from(a && *t);
        either += u64::from(a || *t);
    }
    both as f64 / either as f64
}
fn edge_band(alpha: &[u8]) -> usize {
    alpha.iter().filter(|a| (1..=254).contains(*a)).count()
}

#[test]
fn matte_iou_on_annotated_fixtures_meets_the_recorded_thresholds() {
    let matte = plane();
    for (width, height) in [(640usize, 360usize), (360, 640), (333, 333), (1280, 720)] {
        let truth = annotation(width, height);
        for (feather, floor) in [
            (0., IOU_FEATHER_OFF),
            (25., IOU_FEATHER_25),
            (100., IOU_FEATHER_100),
        ] {
            let alpha = matte
                .alpha(width as u32, height as u32, feather, &Hooks)
                .unwrap();
            let measured = iou(&alpha, &truth);
            println!(
                "MATTE_IOU {width}x{height} feather={feather} iou={measured:.4} floor={floor} model_card_miou={MODEL_CARD_MIOU}"
            );
            assert!(
                measured >= floor,
                "{width}x{height} feather {feather}: {measured} < {floor}"
            );
        }
    }
}

#[test]
fn feather_off_is_the_exact_resample_of_the_model_plane() {
    let matte = plane();
    let alpha = matte
        .alpha(MATTE_SIDE as u32, MATTE_SIDE as u32, 0., &Hooks)
        .unwrap();
    let expected: Vec<u8> = matte
        .probabilities()
        .iter()
        .map(|p| (p * 255.).round() as u8)
        .collect();
    assert_eq!(alpha, expected);
}

#[test]
fn feather_widens_the_edge_band_and_output_is_deterministic() {
    let matte = plane();
    let bands: Vec<usize> = [0., 25., 100.]
        .into_iter()
        .map(|feather| edge_band(&matte.alpha(640, 360, feather, &Hooks).unwrap()))
        .collect();
    assert!(bands[0] < bands[1] && bands[1] < bands[2], "{bands:?}");
    for feather in [0., 40., 100.] {
        assert_eq!(
            matte.alpha(331, 197, feather, &Hooks).unwrap(),
            matte.alpha(331, 197, feather, &Hooks).unwrap()
        );
    }
    // Any positive feather blurs, however small.
    assert_ne!(
        matte.alpha(640, 360, 0.01, &Hooks).unwrap(),
        matte.alpha(640, 360, 0., &Hooks).unwrap()
    );
}

#[test]
fn four_k_matte_allocates_only_its_alpha_and_stops_within_a_row_slice() {
    let matte = plane();
    let alpha = matte.alpha(3840, 2160, 50., &Hooks).unwrap();
    assert_eq!(alpha.len(), 3840 * 2160);
    assert_eq!(alpha[1080 * 3840 + 1920], 255, "torso centre");
    assert_eq!(alpha[0], 0, "corner");
    // Cancellation lands on the fifth stage report: the initial checkpoint plus
    // one per 16 output rows, so no 4K frame completes after a cancel request.
    let hooks = CancelAfter {
        stages: AtomicUsize::new(0),
        limit: 5,
    };
    assert!(matches!(
        matte.alpha(3840, 2160, 50., &hooks),
        Err(Error::Cancelled)
    ));
    assert_eq!(hooks.stages.load(Ordering::SeqCst), 5);
    assert!(matches!(
        matte.alpha(640, 360, 0., &Cancelled),
        Err(Error::Cancelled)
    ));
}

#[test]
fn hostile_geometry_feather_and_planes_are_refused() {
    let matte = plane();
    for (width, height, feather) in [
        (0, 10, 0.),
        (10, 0, 0.),
        (16_385, 1, 0.),
        (1, 16_385, 0.),
        (16_384, 16_384, 0.),
        (10, 10, -0.5),
        (10, 10, 100.5),
    ] {
        assert!(
            matches!(
                matte.alpha(width, height, feather, &Hooks),
                Err(Error::VisionShape)
            ),
            "{width}x{height} feather {feather}"
        );
    }
    assert!(matches!(
        matte.alpha(10, 10, f32::NAN, &Hooks),
        Err(Error::VisionNonFinite)
    ));
    let cells = MATTE_SIDE * MATTE_SIDE;
    assert!(matches!(
        PersonMatte::new(Vec::new()),
        Err(Error::VisionShape)
    ));
    assert!(matches!(
        PersonMatte::new(vec![0.5; cells - 1]),
        Err(Error::VisionShape)
    ));
    assert!(matches!(
        PersonMatte::new(vec![f32::NAN; cells]),
        Err(Error::VisionNonFinite)
    ));
    for bad in [1.5, -0.5] {
        assert!(matches!(
            PersonMatte::new(vec![bad; cells]),
            Err(Error::VisionShape)
        ));
    }
    // Softmax rounding slack clamps; it is not an error.
    let clamped = PersonMatte::new(vec![1.0005; cells]).unwrap();
    assert!(clamped.probabilities().iter().all(|p| *p == 1.));
    assert_eq!(format!("{clamped:?}"), "PersonMatte([redacted])");
    let pixels = 3 * MATTE_SIDE * MATTE_SIDE;
    assert!(PersonFrame::new(&vec![255.; pixels]).is_ok());
    assert!(PersonFrame::new(&vec![256.; pixels]).is_err());
    assert!(matches!(
        PersonFrame::new(&vec![f32::NAN; pixels]),
        Err(Error::VisionNonFinite)
    ));
    assert!(PersonFrame::new(&[]).is_err());
}

#[cfg(feature = "vision")]
#[test]
fn segmentation_runtime_checks_configuration_then_verifies_weights_before_parsing() {
    use ai::vision::TractSegmentation;
    use std::path::Path;
    assert!(matches!(
        TractSegmentation::load(Path::new("missing"), 0, &Hooks),
        Err(Error::VisionShape)
    ));
    assert!(matches!(
        TractSegmentation::load(Path::new("missing"), 5, &Hooks),
        Err(Error::VisionShape)
    ));
    assert!(matches!(
        TractSegmentation::load(Path::new("missing"), 1, &Cancelled),
        Err(Error::Cancelled)
    ));
    let root = tempfile::tempdir().unwrap();
    let file = root
        .path()
        .join("human_segmentation_pphumanseg_2023mar.onnx");
    for data in [vec![0u8; 3], vec![0u8; 6_163_938], vec![0u8; 6_163_939]] {
        std::fs::write(&file, data).unwrap();
        assert!(matches!(
            TractSegmentation::load(root.path(), 1, &Hooks),
            Err(Error::ModelIntegrity)
        ));
    }
}

// Needs the user-downloaded pinned weights; nothing downloads here.
#[cfg(feature = "vision")]
#[test]
#[ignore = "requires VIBECUT_PERSON_MODEL (directory holding the verified pphumanseg-2023mar file)"]
fn real_weights_run_deterministically_and_respect_the_matte_contract() {
    use ai::vision::TractSegmentation;
    let dir = std::path::PathBuf::from(
        std::env::var_os("VIBECUT_PERSON_MODEL").expect("explicit model directory"),
    );
    let mut runtime = TractSegmentation::load(&dir, 2, &Hooks).unwrap();
    let side = MATTE_SIDE;
    // A gradient frame with a bright block: not a person, but a fixed input.
    let mut pixels = vec![0f32; 3 * side * side];
    for c in 0..3 {
        for y in 0..side {
            for x in 0..side {
                let block = (60..130).contains(&x) && (40..170).contains(&y);
                pixels[c * side * side + y * side + x] = if block {
                    200. - 20. * c as f32
                } else {
                    (x + y) as f32 * 0.5
                };
            }
        }
    }
    let frame = PersonFrame::new(&pixels).unwrap();
    let first = runtime.matte(frame, &Hooks).unwrap();
    let second = runtime.matte(frame, &Hooks).unwrap();
    assert_eq!(first, second, "same pinned model and input");
    let again = TractSegmentation::load(&dir, 1, &Hooks).unwrap();
    assert_ne!(
        again.fingerprint(),
        runtime.fingerprint(),
        "threads enter the key"
    );
    assert!(first.probabilities().iter().all(|p| (0. ..=1.).contains(p)));
    assert!(matches!(
        runtime.matte(frame, &Cancelled),
        Err(Error::Cancelled)
    ));
    println!(
        "PERSON_MATTE mean_probability={:.4} fingerprint={}",
        first.probabilities().iter().sum::<f32>() / (side * side) as f32,
        runtime.fingerprint()
    );
}

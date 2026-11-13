#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
//! Person matte: the segmentation contract's planes, bounds and runtime loading. The feathered,
//! frame-size application and its IoU fixtures belong to render (`tests/it/matte.rs` there).
use ai::{
    Error,
    vision::{MATTE_SIDE, PersonFrame, PersonMatte},
};

#[cfg(feature = "vision")]
struct Hooks;
#[cfg(feature = "vision")]
impl ai::ExecutionHooks for Hooks {
    fn cancelled(&self) -> bool {
        false
    }
    fn stage(&self, _: ai::Stage) {}
}
#[cfg(feature = "vision")]
struct Cancelled;
#[cfg(feature = "vision")]
impl ai::ExecutionHooks for Cancelled {
    fn cancelled(&self) -> bool {
        true
    }
    fn stage(&self, _: ai::Stage) {}
}

#[test]
fn a_plane_quantizes_to_the_bytes_render_applies() {
    let cells = MATTE_SIDE * MATTE_SIDE;
    let values: Vec<f32> = (0..cells).map(|i| (i % 1001) as f32 / 1000.0).collect();
    let matte = PersonMatte::new(values.clone()).unwrap();
    let bytes = matte.quantized();
    assert_eq!(bytes.len(), cells);
    for (byte, value) in bytes.iter().zip(&values) {
        assert_eq!(*byte, (value * 255.0).round() as u8);
    }
    assert_eq!(bytes, PersonMatte::new(values).unwrap().quantized());
}

#[test]
fn hostile_planes_and_frames_are_refused() {
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
    assert!(clamped.quantized().iter().all(|p| *p == 255));
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

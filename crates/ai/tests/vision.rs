#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
use ai::{
    Error, ExecutionHooks, Stage,
    vision::{
        self, ActiveSpeaker, FaceFrame, SceneFrame, SceneKind, SpeakerChunk, TrackScores,
        aggregate_speakers, classify_embedding,
    },
};
struct Hooks;
impl ExecutionHooks for Hooks {
    fn cancelled(&self) -> bool {
        false
    }
    fn stage(&self, _: Stage) {}
}
struct Cancel;
impl ExecutionHooks for Cancel {
    fn cancelled(&self) -> bool {
        true
    }
    fn stage(&self, _: Stage) {}
}
#[test]
fn bounds_refuse_hostile_inputs_without_allocating() {
    for n in [0, 1, 24, 76, usize::MAX] {
        assert!(matches!(
            SpeakerChunk::new(n, &[], &[]),
            Err(Error::VisionShape)
        ));
    }
    assert!(FaceFrame::new(&[]).is_err());
    assert!(SceneFrame::new(&[]).is_err());
    let mut rgb = vec![0.; 3 * 224 * 224];
    rgb[0] = f32::NAN;
    assert!(matches!(SceneFrame::new(&rgb), Err(Error::VisionNonFinite)));
    assert!(matches!(
        aggregate_speakers(&[], &Cancel),
        Err(Error::Cancelled)
    ));
    assert!(matches!(
        aggregate_speakers(
            &[TrackScores {
                track: 0,
                start_frame: usize::MAX,
                probabilities: &[0.; 25]
            }],
            &Hooks
        ),
        Err(Error::VisionShape)
    ));
}
#[test]
fn overlap_smoothing_offscreen_and_stable_track_identity() {
    assert_eq!(
        aggregate_speakers(&[], &Hooks).unwrap(),
        ActiveSpeaker::NoVisibleSpeaker
    );
    let low = [0.1; 37];
    let high = [0.8; 75];
    let chunks = [
        TrackScores {
            track: 3,
            start_frame: 0,
            probabilities: &low,
        },
        TrackScores {
            track: 9,
            start_frame: 0,
            probabilities: &high,
        },
        TrackScores {
            track: 9,
            start_frame: 37,
            probabilities: &low,
        },
    ];
    let got = aggregate_speakers(&chunks, &Hooks).unwrap();
    assert!(matches!(got,ActiveSpeaker::Visible { track:9,probability } if probability>0.4));
    assert_eq!(
        aggregate_speakers(&chunks[..1], &Hooks).unwrap(),
        ActiveSpeaker::NoVisibleSpeaker
    );
    assert!(matches!(
        aggregate_speakers(
            &[
                TrackScores {
                    track: 3,
                    start_frame: 0,
                    probabilities: &low
                },
                TrackScores {
                    track: 3,
                    start_frame: 0,
                    probabilities: &low
                }
            ],
            &Hooks
        ),
        Err(Error::VisionShape)
    ));
    assert!(matches!(
        aggregate_speakers(
            &[TrackScores {
                track: 0,
                start_frame: 0,
                probabilities: &[f32::NAN; 25]
            }],
            &Hooks
        ),
        Err(Error::VisionNonFinite)
    ));
}
#[test]
fn normalized_prompt_classifier_and_unknown_are_explicit() {
    let mut screen = vec![0.; 512];
    screen[0] = 1.;
    let mut camera = vec![0.; 512];
    camera[1] = 1.;
    assert_eq!(
        classify_embedding(&screen, [&screen, &camera])
            .unwrap()
            .kind,
        SceneKind::Screen
    );
    assert_eq!(
        classify_embedding(&camera, [&screen, &camera])
            .unwrap()
            .kind,
        SceneKind::Camera
    );
    assert!(matches!(
        classify_embedding(&[0.; 512], [&screen, &camera]),
        Err(Error::VisionNonFinite)
    ));
    assert_eq!(vision::SceneResult::unknown().kind, SceneKind::Unknown);
    assert!(vision::SceneResult::unknown().screen_probability.is_none());
}
#[cfg(feature = "vision")]
#[test]
fn runtime_refuses_cancelled_and_invalid_configuration_before_io() {
    use std::path::Path;
    use vision::{TractVision, VisionPaths};
    let paths = || VisionPaths {
        yunet: Path::new("missing"),
        clip: Path::new("missing"),
        light_asd: Path::new("missing"),
    };
    assert!(matches!(
        TractVision::load(paths(), 0, 3, &Hooks),
        Err(Error::VisionShape)
    ));
    assert!(matches!(
        TractVision::load(paths(), 1, usize::MAX, &Hooks),
        Err(Error::VisionShape)
    ));
    assert!(matches!(
        TractVision::load(paths(), 1, 3, &Cancel),
        Err(Error::Cancelled)
    ));
}

// Retained private /13 input tensors and pinned local weights are required.
// No downloads, image/audio decoding or remote calls occur in this check.
#[cfg(all(feature = "vision", feature = "candle-align"))]
#[test]
#[ignore = "requires VIBECUT_VISION_MODELS and VIBECUT_VISION_CORPUS; private /13 parity corpus"]
fn real_model_parity_and_exact_tails() {
    use candle_core::{Device, Tensor};
    use std::{collections::BTreeMap, path::PathBuf};
    use vision::{TractVision, VisionDetector, VisionPaths};
    fn data(t: &Tensor) -> Vec<f32> {
        t.flatten_all().unwrap().to_vec1::<f32>().unwrap()
    }
    fn load(root: &std::path::Path, name: &str) -> std::collections::HashMap<String, Tensor> {
        candle_core::safetensors::load(root.join(name), &Device::Cpu).unwrap()
    }
    fn probabilities(t: &Tensor) -> Vec<f32> {
        data(t)
            .as_chunks::<2>()
            .0
            .iter()
            .map(|p| {
                let m = p[0].max(p[1]);
                let a = (p[0] - m).exp();
                let b = (p[1] - m).exp();
                b / (a + b)
            })
            .collect()
    }
    let weights =
        PathBuf::from(std::env::var_os("VIBECUT_VISION_MODELS").expect("model paths required"));
    let root = PathBuf::from(std::env::var_os("VIBECUT_VISION_CORPUS").expect("corpus required"));
    let threads = std::env::var("VIBECUT_VISION_THREADS")
        .unwrap_or("1".into())
        .parse()
        .unwrap();
    let load_started = std::time::Instant::now();
    let mut runtime = TractVision::load(
        VisionPaths {
            yunet: &weights,
            clip: &weights,
            light_asd: &weights,
        },
        threads,
        6,
        &Hooks,
    )
    .unwrap();
    println!(
        "LOAD elapsed_ms={}",
        load_started.elapsed().as_secs_f64() * 1000.
    );
    let text = load(&root, "text.safetensors");
    let te = data(&text["text"]);
    let counts: Vec<usize> =
        serde_json::from_slice(&std::fs::read(root.join("face-counts.json")).unwrap()).unwrap();
    assert_eq!(counts.len(), 30);
    let boxes_reference: Vec<Vec<[f32; 5]>> =
        serde_json::from_slice(&std::fs::read(root.join("face-boxes.json")).unwrap()).unwrap();
    for (i, count) in counts.iter().enumerate() {
        let tensors = load(&root, &format!("yunet_{i:02}.safetensors"));
        let x = data(&tensors["input"]);
        let actual = runtime.faces(FaceFrame::new(&x).unwrap(), &Hooks).unwrap();
        assert_eq!(actual.len(), *count, "face frame {i}");
        for (actual, expected) in actual.iter().zip(&boxes_reference[i]) {
            for (a, b) in [actual.x, actual.y, actual.width, actual.height]
                .iter()
                .zip(&expected[..4])
            {
                assert!((a - b).abs() < 0.02, "face box frame {i}");
            }
            assert!((actual.confidence - expected[4]).abs() < 0.0001);
        }
    }
    for i in 0..50 {
        let tensors = load(&root, &format!("clip_{i:02}.safetensors"));
        let x = data(&tensors["input"]);
        let expected =
            classify_embedding(&data(&tensors["out0"]), [&te[..512], &te[512..]]).unwrap();
        let actual = runtime.scene(SceneFrame::new(&x).unwrap(), &Hooks).unwrap();
        assert_eq!(actual.kind, expected.kind, "scene frame {i}");
        assert!(
            (actual.screen_probability.unwrap() - expected.screen_probability.unwrap()).abs()
                < 0.0001
        );
    }
    #[derive(serde::Deserialize)]
    struct Chunk {
        track: u32,
        shot: u32,
        frame: usize,
        n: usize,
    }
    let chunks: Vec<Chunk> =
        serde_json::from_slice(&std::fs::read(root.join("chunks.json")).unwrap()).unwrap();
    assert_eq!(chunks.len(), 121);
    type ChunkComparison = (u32, usize, Vec<f32>, Vec<f32>);
    let mut shot_data: BTreeMap<u32, Vec<ChunkComparison>> = BTreeMap::new();
    let mut maxdiff = 0f32;
    for (i, chunk) in chunks.iter().enumerate() {
        let tensors = load(&root, &format!("asd_{i:03}.safetensors"));
        let a = data(&tensors["audio"]);
        let v = data(&tensors["video"]);
        let actual = runtime
            .speaker(SpeakerChunk::new(chunk.n, &a, &v).unwrap(), &Hooks)
            .unwrap();
        let expected = probabilities(&tensors["out0"]);
        for (a, b) in actual.iter().zip(&expected) {
            maxdiff = maxdiff.max((a - b).abs());
        }
        shot_data
            .entry(chunk.shot)
            .or_default()
            .push((chunk.track, chunk.frame, actual, expected));
        assert!(runtime.cached_plans() <= 6);
    }
    assert!(maxdiff < 0.0001, "ASD probability maxdiff {maxdiff}");
    assert_eq!(shot_data.len(), 30);
    let saved_verdicts: BTreeMap<u32, Option<u32>> =
        serde_json::from_slice(&std::fs::read(root.join("speaker-verdicts.json")).unwrap())
            .unwrap();
    assert_eq!(saved_verdicts.len(), 30);
    for (shot, rows) in &shot_data {
        let actual: Vec<_> = rows
            .iter()
            .map(|(track, start, a, _)| TrackScores {
                track: *track,
                start_frame: *start,
                probabilities: a,
            })
            .collect();
        let expected: Vec<_> = rows
            .iter()
            .map(|(track, start, _, e)| TrackScores {
                track: *track,
                start_frame: *start,
                probabilities: e,
            })
            .collect();
        let identity = |v| match v {
            ActiveSpeaker::Visible { track, .. } => Some(track),
            ActiveSpeaker::NoVisibleSpeaker => None,
        };
        assert_eq!(
            identity(aggregate_speakers(&actual, &Hooks).unwrap()),
            identity(aggregate_speakers(&expected, &Hooks).unwrap())
        );
        assert_eq!(
            identity(aggregate_speakers(&actual, &Hooks).unwrap()),
            saved_verdicts[shot],
            "original /12 verdict for shot {shot}"
        );
    }
    // Original dynamic models, exact prefixes of a real 75-frame chunk, ORT
    // references. Every length 37..=75 executes with no padding/operator edits.
    let tail = load(&root, "tails-input.safetensors");
    let a = data(&tail["audio"]);
    let v = data(&tail["video"]);
    let references = load(&root, "tails-reference.safetensors");
    for n in 37..=75 {
        let actual = runtime
            .speaker(
                SpeakerChunk::new(n, &a[..n * 4 * 13], &v[..n * 112 * 112]).unwrap(),
                &Hooks,
            )
            .unwrap();
        let expected = probabilities(&references[&format!("n{n}")]);
        assert_eq!(actual.len(), n);
        assert!(
            actual
                .iter()
                .zip(&expected)
                .all(|(a, b)| (a - b).abs() < 0.0001),
            "tail {n}"
        );
        assert!(runtime.cached_plans() <= 6);
    }
    fn measure(kind: &str, mut f: impl FnMut()) {
        for _ in 0..3 {
            f();
        }
        let mut ms = Vec::new();
        for _ in 0..15 {
            let start = std::time::Instant::now();
            f();
            ms.push(start.elapsed().as_secs_f64() * 1000.);
        }
        ms.sort_by(f64::total_cmp);
        println!(
            "BENCH kind={kind} warmups=3 repetitions=15 median_ms={} p95_ms={} calls_per_second={}",
            ms[7],
            ms[14],
            15000. / ms.iter().sum::<f64>()
        );
    }
    let face_fixture = load(&root, "yunet_00.safetensors");
    let face_pixels = data(&face_fixture["input"]);
    measure("yunet", || {
        runtime
            .faces(FaceFrame::new(&face_pixels).unwrap(), &Hooks)
            .unwrap();
    });
    let scene_fixture = load(&root, "clip_00.safetensors");
    let scene_pixels = data(&scene_fixture["input"]);
    measure("clip", || {
        runtime
            .scene(SceneFrame::new(&scene_pixels).unwrap(), &Hooks)
            .unwrap();
    });
    measure("asd", || {
        runtime
            .speaker(SpeakerChunk::new(75, &a, &v).unwrap(), &Hooks)
            .unwrap();
    });
    struct AfterStage(std::sync::atomic::AtomicBool);
    impl ExecutionHooks for AfterStage {
        fn cancelled(&self) -> bool {
            self.0.load(std::sync::atomic::Ordering::SeqCst)
        }
        fn stage(&self, _: Stage) {
            self.0.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }
    let hooks = AfterStage(std::sync::atomic::AtomicBool::new(false));
    assert!(matches!(
        runtime.speaker(SpeakerChunk::new(75, &a, &v).unwrap(), &hooks),
        Err(Error::Cancelled)
    ));
    println!(
        "PARITY faces=30/30 scenes=50/50 speakers=30/30 chunks=121 exact_tails=39/39 max_asd_probability_diff={maxdiff} threads={threads} fingerprint={}",
        runtime.fingerprint().unwrap()
    );
}

#[cfg(feature = "vision")]
#[test]
fn malformed_or_truncated_weights_are_rejected_before_parser_or_inference() {
    use std::path::Path;
    use vision::{TractVision, VisionPaths};
    let root = std::env::temp_dir().join(format!("vibecut-vision-corrupt-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let file = root.join("face_detection_yunet_2026may.onnx");
    for data in [vec![0u8; 3], vec![0u8; 229738], vec![0u8; 229739]] {
        std::fs::write(&file, data).unwrap();
        let result = TractVision::load(
            VisionPaths {
                yunet: &root,
                clip: Path::new("missing"),
                light_asd: Path::new("missing"),
            },
            1,
            3,
            &Hooks,
        );
        assert!(matches!(result, Err(Error::ModelIntegrity)));
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn shot_result_refuses_unbounded_or_inconsistent_evidence() {
    use vision::{FaceBox, SceneResult, ShotResult};
    let result = ShotResult::new(
        Vec::new(),
        ActiveSpeaker::NoVisibleSpeaker,
        SceneResult::unknown(),
    )
    .unwrap();
    assert_eq!(result.face_count(), 0);
    assert!(matches!(
        ShotResult::new(
            vec![
                FaceBox {
                    x: 0.,
                    y: 0.,
                    width: 1.,
                    height: 1.,
                    confidence: 1.
                };
                257
            ],
            ActiveSpeaker::NoVisibleSpeaker,
            SceneResult::unknown()
        ),
        Err(Error::VisionShape)
    ));
    assert!(matches!(
        ShotResult::new(
            Vec::new(),
            ActiveSpeaker::Visible {
                track: 0,
                probability: f32::NAN
            },
            SceneResult::unknown()
        ),
        Err(Error::VisionNonFinite)
    ));
    assert!(matches!(
        ShotResult::new(
            Vec::new(),
            ActiveSpeaker::NoVisibleSpeaker,
            SceneResult {
                kind: SceneKind::Screen,
                screen_probability: Some(0.1)
            }
        ),
        Err(Error::VisionShape)
    ));
}

//! Vision media preparation through the public interface.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod common;

use media::{
    AssetId, FaceBox, MediaError, SourceAsset, VisionFrame, VisionPrepLimits, decode_frame_at,
    decode_shot_frames, mfcc_range,
};
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use time::{Rounding, SourceRange, TICKS_PER_SECOND, Tick};

fn seconds(value: f64) -> Tick {
    Tick::from_seconds_f64(value, Rounding::Nearest).unwrap()
}

fn asset(path: &Path, id: u64) -> SourceAsset {
    SourceAsset::import(AssetId(id), path.to_path_buf()).unwrap()
}

fn cancel() -> Arc<AtomicBool> {
    Arc::new(AtomicBool::new(false))
}

fn wav_16k_mono(samples: &[i16]) -> Vec<u8> {
    let data = samples.len() * 2;
    let mut bytes = Vec::with_capacity(44 + data);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36u32 + data as u32).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&16_000u32.to_le_bytes());
    bytes.extend_from_slice(&32_000u32.to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&(data as u32).to_le_bytes());
    for sample in samples {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    bytes
}

fn mfcc_fixture() -> PathBuf {
    let dir = common::fixtures_dir().join("vision");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("mfcc_reference.wav");
    let mut state = 0x1234_5678u32;
    let mut samples = Vec::with_capacity(16_000);
    for _ in 0..16_000 {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        samples.push((((state >> 16) & 0xffff) as i32 - 32_768) as i16);
    }
    std::fs::write(&path, wav_16k_mono(&samples)).unwrap();
    path
}

fn collect_mfcc(
    asset: &SourceAsset,
    range: SourceRange,
    limits: &VisionPrepLimits,
    cancel: Arc<AtomicBool>,
) -> Vec<media::MfccFrame> {
    let mut frames = Vec::new();
    mfcc_range(asset, range, limits, cancel, |frame| {
        frames.push(frame);
        Ok(())
    })
    .unwrap();
    frames
}

#[test]
fn face_crop_matches_reference_recipe_with_padding() {
    let (width, height) = (128u32, 96u32);
    let mut pixels = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            let r = ((x * 11 + y * 3) % 256) as u8;
            let g = ((x * 5 + y * 17) % 256) as u8;
            let b = ((x * 7 + y * 13 + 40) % 256) as u8;
            pixels.extend_from_slice(&[r, g, b, 255]);
        }
    }
    let frame = VisionFrame::from_rgba8(Tick::ZERO, width, height, pixels).unwrap();
    let crop = frame
        .face_crop(
            FaceBox {
                x: 84.0,
                y: 34.0,
                width: 32.0,
                height: 32.0,
            },
            &VisionPrepLimits::default(),
        )
        .unwrap();
    assert_eq!(crop.width(), 112);
    assert_eq!(crop.height(), 112);
    let golden = include_bytes!("fixtures/vision/face_crop_reference.bin");
    assert_eq!(golden.len(), 112 * 112);
    let mut max = 0u8;
    let mut total = 0u64;
    for (actual, expected) in crop.pixels().iter().zip(golden.iter().copied()) {
        let difference = actual.abs_diff(expected);
        max = max.max(difference);
        total += u64::from(difference);
    }
    let mean = total as f64 / golden.len() as f64;
    // OpenCV's 1-channel 8-bit INTER_LINEAR path is not the exact fixed-point
    // interpolation; this recipe matches it within two levels on this crop.
    assert!(
        max <= 2,
        "face crop differs by {max} from the reference recipe"
    );
    assert!(mean < 0.35, "face crop mean difference {mean}");
}

#[test]
fn face_crop_refuses_hostile_or_empty_boxes() {
    let frame = VisionFrame::from_rgba8(Tick::ZERO, 32, 32, vec![0; 32 * 32 * 4]).unwrap();
    for face in [
        FaceBox {
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 10.0,
        },
        FaceBox {
            x: f64::NAN,
            y: 0.0,
            width: 10.0,
            height: 10.0,
        },
        FaceBox {
            x: 0.0,
            y: 0.0,
            width: 1.0e12,
            height: 1.0e12,
        },
    ] {
        assert!(frame.face_crop(face, &VisionPrepLimits::default()).is_err());
    }
}

#[test]
fn mfcc_matches_reference_coefficients() {
    let source = asset(&mfcc_fixture(), 1);
    let frames = collect_mfcc(
        &source,
        SourceRange::new(Tick::ZERO, seconds(1.0)).unwrap(),
        &VisionPrepLimits::default(),
        cancel(),
    );
    assert_eq!(frames.len(), 99);
    let expected = [
        (
            0usize,
            [
                25.513292434,
                -32.796993690,
                -8.361240628,
                -11.671649904,
                -6.851174003,
                -2.902386234,
                -0.159015714,
                2.289507878,
                -2.445255710,
                -6.226338362,
                3.519586515,
                -0.474232787,
                3.635145608,
            ],
        ),
        (
            50usize,
            [
                25.602214362,
                -35.171739933,
                -7.872445110,
                -8.421733858,
                -2.401385802,
                -0.706584187,
                -3.986993563,
                2.360481778,
                0.362640729,
                -4.012123327,
                -3.669536305,
                -2.949970498,
                -11.636710620,
            ],
        ),
        (
            98usize,
            [
                25.575564951,
                -32.800059715,
                -3.935204806,
                -5.379313244,
                0.580297886,
                -7.282537610,
                -2.217790688,
                2.267935717,
                -5.237335886,
                -2.741504035,
                -3.247650297,
                0.563894924,
                1.519113923,
            ],
        ),
    ];
    for (index, coefficients) in expected {
        for (actual, reference) in frames[index]
            .coefficients
            .iter()
            .zip(coefficients.iter().copied())
        {
            assert!(
                (f64::from(*actual) - reference).abs() < 1.0e-4,
                "frame {index}: {actual} != {reference}"
            );
        }
    }
}

#[test]
fn mfcc_range_is_globally_anchored() {
    let source = asset(&mfcc_fixture(), 2);
    let full = collect_mfcc(
        &source,
        SourceRange::new(Tick::ZERO, seconds(1.0)).unwrap(),
        &VisionPrepLimits::default(),
        cancel(),
    );
    let part = collect_mfcc(
        &source,
        SourceRange::new(seconds(0.4), seconds(0.9)).unwrap(),
        &VisionPrepLimits::default(),
        cancel(),
    );
    assert_eq!(part.len(), 50);
    for (offset, frame) in part.iter().enumerate() {
        assert_eq!(frame.source_start, full[40 + offset].source_start);
        for (actual, expected) in frame
            .coefficients
            .iter()
            .zip(full[40 + offset].coefficients.iter())
        {
            assert!((actual - expected).abs() < 1.0e-6);
        }
    }
    let sub_chunk = collect_mfcc(
        &source,
        SourceRange::new(Tick::ZERO, Tick(TICKS_PER_SECOND / 200)).unwrap(),
        &VisionPrepLimits::default(),
        cancel(),
    );
    assert_eq!(sub_chunk.len(), 1);
    assert!(
        sub_chunk[0]
            .coefficients
            .iter()
            .all(|value| value.is_finite())
    );

    let limits = VisionPrepLimits {
        max_mfcc_frames: 1,
        ..VisionPrepLimits::default()
    };
    assert!(matches!(
        mfcc_range(
            &source,
            SourceRange::new(Tick::ZERO, seconds(1.0)).unwrap(),
            &limits,
            cancel(),
            |_| Ok(()),
        ),
        Err(MediaError::TooLarge(_))
    ));

    for (name, path, tolerance, id) in [
        (
            "44.1 kHz PCM",
            common::generate(
                "vision mfcc 44k",
                "vision_mfcc_44100.wav",
                &[
                    "-f",
                    "lavfi",
                    "-i",
                    "sine=frequency=440:sample_rate=44100:duration=8",
                    "-c:a",
                    "pcm_s16le",
                ],
            ),
            1.0e-4,
            20,
        ),
        (
            "44.1 kHz AAC",
            common::generate(
                "vision mfcc compressed seek",
                "vision_mfcc_44100.m4a",
                &[
                    "-f",
                    "lavfi",
                    "-i",
                    "sine=frequency=440:sample_rate=44100:duration=8",
                    "-c:a",
                    "aac",
                    "-b:a",
                    "128k",
                ],
            ),
            1.0e-3,
            21,
        ),
    ] {
        let Some(path) = path else {
            return;
        };
        let source = asset(&path, id);
        let full = collect_mfcc(
            &source,
            SourceRange::new(Tick::ZERO, seconds(8.0)).unwrap(),
            &VisionPrepLimits::default(),
            cancel(),
        );
        let part = collect_mfcc(
            &source,
            SourceRange::new(seconds(5.0), seconds(6.0)).unwrap(),
            &VisionPrepLimits::default(),
            cancel(),
        );
        assert_eq!(part.len(), 100, "{name}");
        let mut max = 0.0f32;
        for (actual, expected) in part.iter().zip(full[500..].iter()) {
            for (left, right) in actual.coefficients.iter().zip(expected.coefficients.iter()) {
                max = max.max((left - right).abs());
            }
        }
        assert!(max < tolerance, "{name}: late-seek coefficient delta {max}");
    }

    let Some(path) = common::generate(
        "vision mfcc unknown duration",
        "vision_unknown_duration.mkv",
        &[
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=16000:duration=1",
            "-c:a",
            "pcm_s16le",
        ],
    ) else {
        return;
    };
    let source = asset(&path, 22);
    let frames = collect_mfcc(
        &source,
        SourceRange::new(Tick::ZERO, seconds(5.0)).unwrap(),
        &VisionPrepLimits::default(),
        cancel(),
    );
    assert_eq!(frames.len(), 99, "synthetic windows past EOF");
    assert!(frames.last().unwrap().source_start < seconds(1.0));
    let after = collect_mfcc(
        &source,
        SourceRange::new(seconds(2.0), seconds(3.0)).unwrap(),
        &VisionPrepLimits::default(),
        cancel(),
    );
    assert!(after.is_empty(), "windows invented after the audio ended");
}

#[test]
fn mfcc_cancellation_stops_between_frames() {
    let source = asset(&mfcc_fixture(), 3);
    let token = cancel();
    let result = mfcc_range(
        &source,
        SourceRange::new(Tick::ZERO, seconds(1.0)).unwrap(),
        &VisionPrepLimits::default(),
        token.clone(),
        |_| {
            token.store(true, Ordering::Relaxed);
            Ok(())
        },
    );
    assert!(matches!(result, Err(MediaError::Cancelled)));
}

fn video_source() -> Option<PathBuf> {
    common::generate(
        "vision cfr",
        "vision_cfr.mp4",
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=64x48:rate=25:duration=2",
            "-c:v",
            "libx264",
            "-bf",
            "0",
            "-g",
            "10",
            "-pix_fmt",
            "yuv420p",
            "-an",
        ],
    )
}

fn long_gop_source() -> Option<PathBuf> {
    common::generate(
        "vision long gop",
        "vision_long_gop.mp4",
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=64x48:rate=25:duration=10",
            "-c:v",
            "libx264",
            "-bf",
            "0",
            "-g",
            "250",
            "-keyint_min",
            "250",
            "-sc_threshold",
            "0",
            "-pix_fmt",
            "yuv420p",
            "-an",
        ],
    )
}

fn vfr_source() -> Option<PathBuf> {
    common::generate(
        "vision vfr",
        "vision_vfr.mp4",
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=64x48:rate=25:duration=2",
            "-vf",
            "setpts='if(lt(N,10),N*0.02/TB,(0.2+(N-10)*0.1)/TB)'",
            "-fps_mode",
            "vfr",
            "-c:v",
            "libx264",
            "-bf",
            "0",
            "-pix_fmt",
            "yuv420p",
            "-an",
        ],
    )
}

#[test]
fn shot_frames_select_exact_source_times() {
    let Some(path) = video_source() else {
        return;
    };
    let source = asset(&path, 4);
    let range = SourceRange::new(seconds(0.2), seconds(0.52)).unwrap();
    let mut times = Vec::new();
    decode_shot_frames(
        &source,
        range,
        &VisionPrepLimits::default(),
        cancel(),
        |frame| {
            times.push(frame.source_pts());
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(times.len(), 8, "{times:?}");
    for (index, time) in times.iter().enumerate() {
        assert_eq!(*time, seconds(0.2 + index as f64 * 0.04));
    }
    let at = decode_frame_at(
        &source,
        seconds(0.21),
        &VisionPrepLimits::default(),
        cancel(),
    )
    .unwrap();
    assert_eq!(at.source_pts(), seconds(0.2));
}

#[test]
fn sub_frame_shot_uses_the_covering_frame() {
    let Some(path) = video_source() else {
        return;
    };
    let source = asset(&path, 5);
    let range = SourceRange::new(seconds(0.201), seconds(0.202)).unwrap();
    let mut times = Vec::new();
    decode_shot_frames(
        &source,
        range,
        &VisionPrepLimits::default(),
        cancel(),
        |frame| {
            times.push(frame.source_pts());
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(times, vec![seconds(0.2)]);
    // SourceRange is nonempty by contract; a caller with a zero-duration shot
    // asks for its single source time instead.
    let zero = decode_frame_at(
        &source,
        seconds(0.2),
        &VisionPrepLimits::default(),
        cancel(),
    )
    .unwrap();
    assert_eq!(zero.source_pts(), seconds(0.2));
}

#[test]
fn variable_frame_rate_queries_return_the_covering_frame() {
    let Some(path) = vfr_source() else {
        return;
    };
    let source = asset(&path, 6);
    let mut times = Vec::new();
    decode_shot_frames(
        &source,
        SourceRange::new(Tick::ZERO, seconds(2.0)).unwrap(),
        &VisionPrepLimits::default(),
        cancel(),
        |frame| {
            times.push(frame.source_pts());
            Ok(())
        },
    )
    .unwrap();
    assert!(times.len() > 5);
    assert!(times.windows(2).all(|pair| pair[0] < pair[1]));
    for pair in times.windows(2) {
        let inside = seconds_tick(pair[0], 1);
        assert_eq!(
            decode_frame_at(&source, inside, &VisionPrepLimits::default(), cancel())
                .unwrap()
                .source_pts(),
            pair[0]
        );
        let before_next = seconds_tick(pair[1], -1);
        assert_eq!(
            decode_frame_at(&source, before_next, &VisionPrepLimits::default(), cancel())
                .unwrap()
                .source_pts(),
            pair[0]
        );
    }
    assert_eq!(
        decode_frame_at(
            &source,
            *times.last().unwrap(),
            &VisionPrepLimits::default(),
            cancel()
        )
        .unwrap()
        .source_pts(),
        *times.last().unwrap()
    );
}

fn seconds_tick(time: Tick, delta: i64) -> Tick {
    time.checked_add(Tick(delta)).unwrap()
}

#[test]
fn frame_decode_cancels_and_refuses_hostile_limits() {
    let Some(path) = video_source() else {
        return;
    };
    let source = asset(&path, 7);
    let token = Arc::new(AtomicBool::new(true));
    assert!(matches!(
        decode_shot_frames(
            &source,
            SourceRange::new(Tick::ZERO, seconds(1.0)).unwrap(),
            &VisionPrepLimits::default(),
            token,
            |_| Ok(()),
        ),
        Err(MediaError::Cancelled)
    ));

    let token = cancel();
    let result = decode_shot_frames(
        &source,
        SourceRange::new(Tick::ZERO, seconds(1.0)).unwrap(),
        &VisionPrepLimits::default(),
        token.clone(),
        |_| {
            token.store(true, Ordering::Relaxed);
            Ok(())
        },
    );
    assert!(matches!(result, Err(MediaError::Cancelled)));

    let limits = VisionPrepLimits {
        max_frames: 2,
        ..VisionPrepLimits::default()
    };
    assert!(matches!(
        decode_shot_frames(
            &source,
            SourceRange::new(Tick::ZERO, seconds(1.0)).unwrap(),
            &limits,
            cancel(),
            |_| Ok(()),
        ),
        Err(MediaError::TooLarge(_))
    ));

    let limits = VisionPrepLimits {
        max_shot_duration: seconds(0.5),
        ..VisionPrepLimits::default()
    };
    assert!(matches!(
        decode_shot_frames(
            &source,
            SourceRange::new(Tick::ZERO, seconds(1.0)).unwrap(),
            &limits,
            cancel(),
            |_| Ok(()),
        ),
        Err(MediaError::TooLarge(_))
    ));

    // A one-frame budget must also cover the long-GOP keyframe preroll.
    let Some(path) = long_gop_source() else {
        return;
    };
    let source = asset(&path, 8);
    let limits = VisionPrepLimits {
        max_frames: 1,
        ..VisionPrepLimits::default()
    };
    assert!(matches!(
        decode_shot_frames(
            &source,
            SourceRange::new(seconds(9.201), seconds(9.202)).unwrap(),
            &limits,
            cancel(),
            |_| Ok(()),
        ),
        Err(MediaError::TooLarge(_))
    ));
    assert!(matches!(
        decode_frame_at(&source, seconds(9.201), &limits, cancel()),
        Err(MediaError::TooLarge(_))
    ));
}

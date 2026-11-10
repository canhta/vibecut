//! Decoder allocation limits exercised through the public streaming interfaces.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use media::{
    DecodePreference, MediaError, PacketReader, StreamKind, THUMBNAIL_MAX_EDGE, VideoDecoder,
    VideoFrame, VideoReader, thumbnail, thumbnails_at,
};
use std::path::Path;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use time::{FrameRate, TICKS_PER_SECOND, Tick};

fn pixels(frame: &VideoFrame) -> Vec<u8> {
    let cpu = frame.download().unwrap();
    assert_eq!(cpu.pixel_format(), ffmpeg_next::format::Pixel::YUV420P);
    let mut bytes = Vec::new();
    for plane in 0..3 {
        let divisor = if plane == 0 { 1 } else { 2 };
        let width = cpu.width().div_ceil(divisor) as usize;
        let height = cpu.height().div_ceil(divisor) as usize;
        let stride = cpu.stride(plane).unwrap();
        let data = cpu.plane(plane).unwrap();
        for row in 0..height {
            bytes.extend_from_slice(&data[row * stride..row * stride + width]);
        }
    }
    bytes
}

fn check_fixture(path: &Path, width: u32, height: u32, expected_frames: usize) {
    let oracle = testkit::oracle::command("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(path)
        .args([
            "-map", "0:v:0", "-pix_fmt", "yuv420p", "-f", "rawvideo", "-",
        ])
        .output()
        .unwrap();
    assert!(oracle.status.success());
    assert_eq!(
        testkit::oracle::decoded_frames(path, "v:0"),
        expected_frames as u64
    );
    let frame_size = (width * height * 3 / 2) as usize;
    assert_eq!(oracle.stdout.len(), expected_frames * frame_size);
    let check = |frame: VideoFrame, index: usize| {
        assert_eq!((frame.width(), frame.height()), (width, height));
        assert_eq!(
            frame.pts().unwrap(),
            Some(
                FrameRate::new(30, 1)
                    .unwrap()
                    .tick_of(index as i64)
                    .unwrap()
            )
        );
        let offset = index * frame_size;
        assert_eq!(pixels(&frame), oracle.stdout[offset..offset + frame_size]);
    };
    for preference in [DecodePreference::Software, DecodePreference::Hardware] {
        let mut packets = PacketReader::open(path).unwrap();
        let stream = packets
            .probe()
            .streams
            .iter()
            .find(|s| s.kind == StreamKind::Video)
            .unwrap()
            .clone();
        let mut decoder = VideoDecoder::new(&stream, preference).unwrap();
        let mut count = 0;
        while let Some(packet) = packets.next_packet().unwrap() {
            if packet.stream() == stream.index {
                decoder.push_packet(&packet).unwrap();
                while let Some(frame) = decoder.pull_frame().unwrap() {
                    check(frame, count);
                    count += 1;
                }
            }
        }
        decoder.finish_input().unwrap();
        while let Some(frame) = decoder.pull_frame().unwrap() {
            check(frame, count);
            count += 1;
        }
        assert_eq!(count, expected_frames, "{preference:?}");
        assert!(decoder.flush().unwrap().is_empty());
        for (width, height) in [(0, 180), (320, 0), (8192, 8194), (u32::MAX, u32::MAX)] {
            let mut invalid = stream.clone();
            let info = invalid.video.as_mut().unwrap();
            info.width = width;
            info.height = height;
            assert!(matches!(
                VideoDecoder::new(&invalid, preference),
                Err(MediaError::TooLarge("video dimensions"))
            ));
        }
    }
    let mut reader = VideoReader::open(path).unwrap();
    let mut count = 0;
    while let Some(frame) = reader.next_frame().unwrap() {
        check(frame, count);
        count += 1;
    }
    assert_eq!(count, expected_frames, "VideoReader");
    let seek_frame = expected_frames / 2;
    let target = FrameRate::new(30, 1)
        .unwrap()
        .tick_of(seek_frame as i64)
        .unwrap();
    reader.seek(target).unwrap();
    check(reader.next_frame().unwrap().unwrap(), seek_frame);
    reader.seek(Tick::ZERO).unwrap();
    check(reader.next_frame().unwrap().unwrap(), 0);
}

#[test]
fn mpeg4_preview_decodes_all_frames_with_identical_pixels() {
    let Some(path) = testkit::oracle::generate(
        &testkit::fixture::dir("media"),
        "MPEG-4 preview decode",
        "decode_mpeg4_320x180.mp4",
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=320x180:rate=30",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000",
            "-t",
            "12",
            "-c:v",
            "mpeg4",
            "-q:v",
            "5",
            "-c:a",
            "aac",
        ],
    ) else {
        return;
    };
    check_fixture(&path, 320, 180, 360);
}

#[test]
fn mpeg4_small_and_unaligned_frames_with_reordering() {
    for (width, height) in [(16, 16), (318, 178), (320, 192)] {
        let source = format!("testsrc2=size={width}x{height}:rate=30:duration=1");
        let name = format!("decode_mpeg4_{width}x{height}_bframes.mp4");
        let Some(path) = testkit::oracle::generate(
            &testkit::fixture::dir("media"),
            "MPEG-4 decoder boundaries",
            &name,
            &[
                "-f", "lavfi", "-i", &source, "-c:v", "mpeg4", "-q:v", "5", "-bf", "2",
            ],
        ) else {
            return;
        };
        check_fixture(&path, width, height, 30);
    }
}

/// Three seconds at 10 fps with a one-second GOP and B-frames, so sampled
/// positions land on keyframes and inside reordered groups.
fn reordered_thumbnail_fixture() -> Option<std::path::PathBuf> {
    testkit::oracle::generate(
        &testkit::fixture::dir("media"),
        "thumbnail position pass",
        "thumbnail-pass.mp4",
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=96x54:rate=10:duration=3",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-crf",
            "18",
            "-g",
            "10",
            "-keyint_min",
            "10",
            "-sc_threshold",
            "0",
            "-bf",
            "2",
            "-pix_fmt",
            "yuv420p",
        ],
    )
}

#[test]
fn thumbnail_pass_matches_one_decode_per_position() {
    let Some(path) = reordered_thumbnail_fixture() else {
        return;
    };
    let positions: Vec<Tick> = (0..12)
        .map(|index| Tick(TICKS_PER_SECOND * index / 4))
        .collect();
    let cancelled = Arc::new(AtomicBool::new(false));
    let mut pass = Vec::new();
    thumbnails_at(
        &path,
        &positions,
        THUMBNAIL_MAX_EDGE,
        cancelled.clone(),
        |at, image| pass.push((at, image)),
    )
    .unwrap();
    assert_eq!(pass.len(), positions.len());
    let asset = media::SourceAsset::import(media::AssetId(1), path.clone()).unwrap();
    let mut frames = Vec::new();
    for (index, at) in positions.iter().enumerate() {
        let single = thumbnail(&asset, *at, THUMBNAIL_MAX_EDGE, cancelled.clone()).unwrap();
        assert_eq!(pass[index].0, *at);
        assert_eq!(pass[index].1, single, "tile at {at:?}");
        frames.push(single.frame);
    }
    // The fixture moves, so the sampled frames differ across the range.
    assert!(frames.windows(2).any(|pair| pair[0] != pair[1]));
}

#[test]
fn thumbnail_pass_refuses_hostile_positions_and_cancels_between_frames() {
    let Some(path) = reordered_thumbnail_fixture() else {
        return;
    };
    let available = Arc::new(AtomicBool::new(false));
    // The cache payload ceiling is 1024 px; zero and above it refuse before I/O.
    for edge in [0, 1025] {
        assert!(matches!(
            thumbnails_at(&path, &[Tick::ZERO], edge, available.clone(), |_, _| {}),
            Err(MediaError::Invalid("thumbnail max edge"))
        ));
    }
    let hostile: [Vec<Tick>; 3] = [
        Vec::new(),
        vec![Tick(TICKS_PER_SECOND), Tick(TICKS_PER_SECOND / 2)],
        vec![Tick(TICKS_PER_SECOND / 2), Tick(TICKS_PER_SECOND / 2)],
    ];
    for positions in hostile {
        assert!(matches!(
            thumbnails_at(
                &path,
                &positions,
                THUMBNAIL_MAX_EDGE,
                available.clone(),
                |_, _| {}
            ),
            Err(MediaError::Invalid("thumbnail positions"))
        ));
    }
    // A position past the last frame refuses exactly like one call per position.
    let past = Tick(TICKS_PER_SECOND * 10);
    let asset = media::SourceAsset::import(media::AssetId(2), path.clone()).unwrap();
    assert!(matches!(
        thumbnail(&asset, past, THUMBNAIL_MAX_EDGE, available.clone()),
        Err(MediaError::Unsupported("video has no decodable frame"))
    ));
    assert!(matches!(
        thumbnails_at(
            &path,
            &[past],
            THUMBNAIL_MAX_EDGE,
            available.clone(),
            |_, _| {}
        ),
        Err(MediaError::Unsupported("video has no decodable frame"))
    ));
    // A flag set by the first sample stops the pass before the second tile.
    let cancelled = Arc::new(AtomicBool::new(false));
    let mut delivered = 0;
    let result = thumbnails_at(
        &path,
        &[
            Tick::ZERO,
            Tick(TICKS_PER_SECOND),
            Tick(TICKS_PER_SECOND * 2),
        ],
        THUMBNAIL_MAX_EDGE,
        cancelled.clone(),
        |_, _| {
            delivered += 1;
            cancelled.store(true, Ordering::Relaxed);
        },
    );
    assert!(matches!(result, Err(MediaError::Cancelled)));
    assert_eq!(delivered, 1);
}

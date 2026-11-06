//! Decoder allocation limits exercised through the public streaming interfaces.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use media::{
    DecodePreference, MediaError, PacketReader, StreamKind, VideoDecoder, VideoFrame, VideoReader,
};
use std::path::Path;
use time::{FrameRate, Tick};

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
    let oracle = crate::support::command("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(path)
        .args([
            "-map", "0:v:0", "-pix_fmt", "yuv420p", "-f", "rawvideo", "-",
        ])
        .output()
        .unwrap();
    assert!(oracle.status.success());
    assert_eq!(
        crate::support::decoded_frames(path, "v:0"),
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
    let Some(path) = crate::support::generate(
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
        let Some(path) = crate::support::generate(
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

//! Hostile-input and path-handling tests that need no external oracle.
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::fixtures_dir;
use media::{
    AacConfig, AudioTrackConfig, AvcConfig, Brand, MediaError, MuxOptions, Muxer, PacketReader,
    StreamKind, VideoTrackConfig, WritePacket, probe,
};
use std::path::Path;

fn sample_file(path: &Path) {
    let avc = AvcConfig::new(vec![vec![0x67, 0x42, 0, 0x1E]], vec![vec![0x68, 0xCE]], 4);
    let aac = AacConfig::from_asc(vec![0x11, 0x90]);
    let mut muxer = Muxer::create(
        path,
        MuxOptions {
            brand: Brand::Mp4,
            movie_timescale: 1000,
        },
    )
    .expect("create muxer");
    let video = muxer
        .add_video(VideoTrackConfig {
            width: 320,
            height: 180,
            timescale: 30_000,
            avc,
        })
        .expect("add video");
    let audio = muxer
        .add_audio(AudioTrackConfig {
            aac,
            channels: 2,
            sample_rate: 48_000,
            timescale: 48_000,
            priming: 1024,
        })
        .expect("add audio");
    for index in 0..5u32 {
        muxer
            .write_packet(
                video,
                WritePacket {
                    dts: i64::from(index) * 1001,
                    pts: i64::from(index) * 1001,
                    duration: 1001,
                    keyframe: index == 0,
                    data: &[index as u8; 16],
                },
            )
            .expect("video packet");
        muxer
            .write_packet(
                audio,
                WritePacket {
                    dts: i64::from(index) * 1024,
                    pts: i64::from(index) * 1024,
                    duration: 1024,
                    keyframe: true,
                    data: &[index as u8; 8],
                },
            )
            .expect("audio packet");
    }
    muxer.finish().expect("finish");
}

#[test]
fn unicode_and_space_paths_work() {
    let dir = fixtures_dir().join("thư mục có dấu cách/输出 ★");
    std::fs::create_dir_all(&dir).expect("fixture directory");
    let path = dir.join("clip có dấu.mp4");
    sample_file(&path);

    let probed = probe(&path).expect("probe unicode path");
    assert_eq!(probed.streams.len(), 2);
    assert_eq!(
        probed.streams.first().map(|stream| stream.kind),
        Some(StreamKind::Video)
    );
    assert_eq!(
        probed.streams.first().map(|stream| stream.sample_count),
        Some(5)
    );

    let mut reader = PacketReader::open(&path).expect("open unicode path");
    let mut packets = 0;
    while reader.next_packet().expect("packet").is_some() {
        packets += 1;
    }
    assert_eq!(packets, 10);
    std::fs::remove_dir_all(fixtures_dir().join("thư mục có dấu cách")).expect("cleanup");
}

#[test]
fn hostile_inputs_fail_with_typed_errors_without_panicking() {
    let dir = fixtures_dir();
    std::fs::create_dir_all(&dir).expect("fixture directory");
    let valid_path = dir.join("hostile-source.mp4");
    sample_file(&valid_path);
    let valid = std::fs::read(&valid_path).expect("read sample file");
    let hostile_path = dir.join("hostile-case.bin");

    let mut cases: Vec<Vec<u8>> = Vec::new();
    // Truncations across the whole file.
    let step = (valid.len() / 120).max(1);
    for length in (0..valid.len()).step_by(step) {
        cases.push(valid.get(..length).unwrap_or(&[]).to_vec());
    }
    // Deterministic bit flips, plus a few whole-header corruptions.
    let mut state = 0x1234_5678_9ABC_DEF0u64;
    for _ in 0..240 {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        let mut mutated = valid.clone();
        if mutated.is_empty() {
            break;
        }
        let at = (state as usize) % mutated.len();
        let bit = ((state >> 32) % 8) as u8;
        if let Some(byte) = mutated.get_mut(at) {
            *byte ^= 1 << bit;
        }
        cases.push(mutated);
    }
    cases.push(vec![0xFF; 64]);
    cases.push(vec![
        0, 0, 0, 1, b'm', b'd', b'a', b't', 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
    ]);
    cases.push(b"not an mp4".to_vec());

    for (index, case) in cases.iter().enumerate() {
        std::fs::write(&hostile_path, case).expect("write hostile case");
        let outcome = std::panic::catch_unwind(|| {
            if let Ok(probed) = probe(&hostile_path) {
                let _ = probed.streams.len();
            }
            if let Ok(mut reader) = PacketReader::open(&hostile_path) {
                let mut packets = 0u32;
                while let Ok(Some(_packet)) = reader.next_packet() {
                    packets += 1;
                    assert!(packets < 10_000, "reader did not terminate");
                }
            }
        });
        assert!(outcome.is_ok(), "panic on hostile case {index}");
    }

    std::fs::write(&hostile_path, b"".as_slice()).expect("write empty file");
    assert!(matches!(probe(&hostile_path), Err(MediaError::NoMoov)));
    assert!(matches!(
        probe(Path::new("definitely missing.mp4")),
        Err(MediaError::Io(_))
    ));
    std::fs::remove_file(hostile_path).expect("cleanup");
    std::fs::remove_file(valid_path).expect("cleanup");
}

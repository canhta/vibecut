//! Import metadata and thumbnail oracles against ffprobe.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
use super::common;

use media::{
    Decodability, MediaKind, MediaMetadata, PacketReader, ProbeCapabilities, SourceAsset,
    SourceProbe, StreamKind, THUMBNAIL_MAX_EDGE, UnsupportedReason, thumbnail_at_with_read_bytes,
};
use std::path::{Path, PathBuf};
use time::Tick;

fn inspect(path: &Path) -> (SourceProbe, MediaMetadata, ProbeCapabilities) {
    let reader = PacketReader::open(path).unwrap();
    let probe = reader.probe().clone();
    let capabilities = ProbeCapabilities::from_probe(&probe);
    drop(reader);
    let metadata = MediaMetadata::from_probe(&SourceProbe::from(&probe)).unwrap();
    (SourceProbe::from(&probe), metadata, capabilities)
}

fn encoder_available(name: &str) -> bool {
    let output = common::command("ffmpeg")
        .args(["-hide_banner", "-encoders"])
        .output()
        .expect("ffmpeg -encoders");
    output.status.success()
        && String::from_utf8_lossy(&output.stdout)
            .lines()
            .any(|line| line.split_whitespace().nth(1) == Some(name))
}

fn h264_audio() -> Option<PathBuf> {
    common::generate(
        "metadata matrix",
        "metadata-h264-aac.mp4",
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=128x72:rate=30000/1001:duration=1",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=997:sample_rate=48000:duration=1",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
            "-ac",
            "2",
            "-shortest",
        ],
    )
}

fn h264_23976() -> Option<PathBuf> {
    common::generate(
        "metadata matrix",
        "metadata-h264-23976.mp4",
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=96x54:rate=24000/1001:duration=1",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
        ],
    )
}

fn image(name: &str, codec: &str) -> Option<PathBuf> {
    common::generate(
        "metadata matrix",
        name,
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=64x48:rate=1",
            "-frames:v",
            "1",
            "-c:v",
            codec,
            "-pix_fmt",
            "yuvj420p",
        ],
    )
}

fn audio(name: &str, codec: &str, extra: &[&str]) -> Option<PathBuf> {
    let mut args = vec![
        "-f",
        "lavfi",
        "-i",
        "sine=frequency=440:sample_rate=44100:duration=1",
        "-c:a",
        codec,
    ];
    args.extend_from_slice(extra);
    common::generate("metadata matrix", name, &args)
}

fn assert_decodable(capabilities: &ProbeCapabilities, kind: StreamKind, what: &str) {
    let stream = capabilities
        .streams
        .iter()
        .find(|stream| stream.kind == kind)
        .unwrap_or_else(|| panic!("{what}: missing {kind:?} capability"));
    assert!(
        stream.decodability.is_decodable(),
        "{what}: {kind:?} reported {:?}",
        stream.decodability
    );
}

#[test]
fn video_metadata_matches_ffprobe_including_rational_frame_rate() {
    let Some(path) = h264_audio() else {
        return;
    };
    let (probe, metadata, capabilities) = inspect(&path);
    assert_eq!(metadata.kind, MediaKind::Video);
    assert_eq!(metadata.width, Some(128));
    assert_eq!(metadata.height, Some(72));
    assert!(metadata.has_audio);
    assert!(metadata.needs_thumbnail());
    let exact = metadata.display_frame_rate.unwrap();
    assert_eq!((exact.exact.num(), exact.exact.den()), (30_000, 1_001));
    assert_eq!(exact.rounded, 30);
    assert_eq!(
        common::stream_field(&path, "v:0", "r_frame_rate"),
        "30000/1001"
    );
    assert_eq!(common::stream_field(&path, "v:0", "width"), "128");
    assert_eq!(common::stream_field(&path, "v:0", "height"), "72");
    assert_eq!(common::stream_field(&path, "v:0", "codec_name"), "h264");
    assert_eq!(common::stream_field(&path, "a:0", "codec_name"), "aac");
    assert_eq!(common::stream_field(&path, "a:0", "sample_rate"), "48000");
    assert_decodable(&capabilities, StreamKind::Video, "h264");
    assert_decodable(&capabilities, StreamKind::Audio, "aac");
    let observed = metadata.duration.unwrap().seconds();
    common::approx(observed, common::duration(&path), 0.05, "duration");
    assert!(
        probe
            .streams
            .iter()
            .any(|stream| stream.kind == StreamKind::Audio)
    );
}

#[test]
fn display_rounding_keeps_the_exact_rational_for_23976_and_2997() {
    let Some(path) = h264_23976() else {
        return;
    };
    let (_, metadata, _) = inspect(&path);
    let rate = metadata.display_frame_rate.unwrap();
    assert_eq!((rate.exact.num(), rate.exact.den()), (24_000, 1_001));
    assert_eq!(rate.rounded, 24);
    assert_eq!(
        common::stream_field(&path, "v:0", "r_frame_rate"),
        "24000/1001"
    );
    let Some(path) = h264_audio() else {
        return;
    };
    let (_, metadata, _) = inspect(&path);
    let rate = metadata.display_frame_rate.unwrap();
    assert_eq!((rate.exact.num(), rate.exact.den()), (30_000, 1_001));
    assert_eq!(rate.rounded, 30);
}

fn assert_still_import(path: &Path, codec: &str, width: u32, height: u32) {
    let opened = PacketReader::open(path);
    let probe = match opened {
        Ok(reader) => reader.probe().clone(),
        Err(error) => panic!("{path:?}: {error}"),
    };
    let capabilities = ProbeCapabilities::from_probe(&probe);
    let metadata = MediaMetadata::from_probe(&SourceProbe::from(&probe)).unwrap();
    assert_eq!(metadata.kind, MediaKind::Image, "{path:?}");
    assert_eq!(metadata.width, Some(width), "{path:?}");
    assert_eq!(metadata.height, Some(height), "{path:?}");
    assert!(!metadata.has_audio, "{path:?}");
    assert!(metadata.needs_thumbnail(), "{path:?}");
    assert_decodable(&capabilities, StreamKind::Video, codec);
    let (thumbnail, _) = thumbnail_at_with_read_bytes(
        path,
        Tick::ZERO,
        THUMBNAIL_MAX_EDGE,
        std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
    )
    .unwrap();
    assert!(thumbnail.width <= THUMBNAIL_MAX_EDGE);
    assert!(thumbnail.height <= THUMBNAIL_MAX_EDGE);
    assert_eq!(
        thumbnail.pixels.len(),
        (thumbnail.width * thumbnail.height * 4) as usize
    );
}

#[test]
fn png_still_probes_and_thumbnails_on_every_platform() {
    // A committed PNG keeps the decoder check independent of external oracles;
    // every production recipe builds zlib, so a missing decoder fails here.
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/app-icon/vibecut-1024.png");
    assert_still_import(&path, "png", 1024, 1024);
}

#[test]
fn jpeg_still_records_dimensions_and_thumbnail() {
    let Some(path) = image("metadata-still.jpg", "mjpeg") else {
        return;
    };
    assert_still_import(&path, "mjpeg", 64, 48);
    assert_eq!(common::stream_field(&path, "v:0", "width"), "64");
    assert_eq!(common::stream_field(&path, "v:0", "height"), "48");
}

#[test]
fn audio_metadata_matches_ffprobe() {
    let fixtures = [
        ("metadata-tone.wav", "pcm_s16le", &[][..]),
        ("metadata-tone.mp3", "libmp3lame", &["-b:a", "128k"][..]),
    ];
    for (name, codec, extra) in fixtures {
        if codec == "libmp3lame" && !encoder_available(codec) {
            eprintln!("skipping {name}: ffmpeg has no {codec} encoder");
            continue;
        }
        let Some(path) = audio(name, codec, extra) else {
            return;
        };
        let (_, metadata, capabilities) = inspect(&path);
        assert_eq!(metadata.kind, MediaKind::Audio, "{name}");
        assert!(metadata.has_audio, "{name}");
        assert_eq!(metadata.width, None, "{name}");
        assert_eq!(metadata.display_frame_rate, None, "{name}");
        assert!(!metadata.needs_thumbnail(), "{name}");
        assert!(
            common::stream_field(&path, "a:0", "sample_rate") == "44100",
            "{name}"
        );
        assert_decodable(&capabilities, StreamKind::Audio, name);
        common::approx(
            metadata.duration.unwrap().seconds(),
            common::duration(&path),
            0.1,
            name,
        );
    }
}

#[test]
fn hevc_capability_is_a_typed_supported_or_unsupported_result() {
    if !encoder_available("libx265") {
        eprintln!("skipping HEVC matrix row: ffmpeg has no libx265 encoder");
        return;
    }
    let Some(path) = common::generate(
        "metadata matrix",
        "metadata-hevc.mp4",
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=128x72:rate=25:duration=1",
            "-c:v",
            "libx265",
            "-tag:v",
            "hvc1",
            "-pix_fmt",
            "yuv420p",
        ],
    ) else {
        return;
    };
    let (_, metadata, capabilities) = inspect(&path);
    assert_eq!(metadata.kind, MediaKind::Video);
    let video = capabilities.video().expect("HEVC video capability");
    assert_eq!(video.codec, "hevc");
    match video.decodability {
        Decodability::Decodable => {}
        Decodability::Unsupported(reason) => {
            assert!(matches!(
                reason,
                UnsupportedReason::DecoderUnavailable | UnsupportedReason::OpenFailed
            ));
            assert!(!reason.label().is_empty());
        }
    }
}

#[test]
fn thumbnail_position_decodes_forward_past_the_preceding_keyframe() {
    let Some(path) = h264_audio() else {
        return;
    };
    let at = Tick(time::TICKS_PER_SECOND / 2);
    let (thumbnail, _) = thumbnail_at_with_read_bytes(
        &path,
        at,
        THUMBNAIL_MAX_EDGE,
        std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
    )
    .unwrap();
    assert!(thumbnail.frame >= at, "returned the preceding keyframe");
    assert!(thumbnail.frame.0 - at.0 < time::TICKS_PER_SECOND / 20);
}

#[test]
fn probe_and_thumbnail_read_a_bounded_prefix_of_a_large_file() {
    let Some(path) = common::generate(
        "metadata bounds",
        "metadata-large.mp4",
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=1280x720:rate=30:duration=12",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-crf",
            "0",
            "-pix_fmt",
            "yuv420p",
        ],
    ) else {
        return;
    };
    let length = std::fs::metadata(&path).unwrap().len();
    assert!(length > 1 << 20, "large fixture is only {length} bytes");
    let reader = PacketReader::open(&path).unwrap();
    let probe_bytes = reader.probe().bytes_read;
    drop(reader);
    assert!(
        probe_bytes < length,
        "probe read {probe_bytes} of {length} bytes"
    );
    assert!(probe_bytes <= 32 << 20, "probe read budget exceeded");
    let (thumbnail, decode_bytes) = thumbnail_at_with_read_bytes(
        &path,
        Tick::ZERO,
        THUMBNAIL_MAX_EDGE,
        std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
    )
    .unwrap();
    assert!(
        decode_bytes < length,
        "first-frame thumbnail read {decode_bytes} of {length} bytes"
    );
    assert!(thumbnail.width <= THUMBNAIL_MAX_EDGE);
    assert!(thumbnail.height <= THUMBNAIL_MAX_EDGE);
    assert!(thumbnail.pixels.len() <= (THUMBNAIL_MAX_EDGE * THUMBNAIL_MAX_EDGE * 4) as usize);
    // Source/native allocations are admitted separately; packed output is bounded
    // independently of input file length. Peak RSS is measured in the fix receipt.
    assert!(thumbnail.pixels.len() <= 320 * 320 * 4);
}

#[test]
fn imported_source_metadata_survives_project_round_trip_without_transient_state() {
    let Some(path) = h264_audio() else {
        return;
    };
    let imported = SourceAsset::import_with_summary(
        media::AssetId(7),
        path,
        std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
    )
    .unwrap();
    let metadata = imported.metadata().unwrap();
    let bytes = serde_json::to_vec(&imported.asset).unwrap();
    let restored: SourceAsset = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(restored.id(), media::AssetId(7));
    assert_eq!(restored.probe(), imported.asset.probe());
    assert_eq!(
        MediaMetadata::from_probe(restored.probe()).unwrap(),
        metadata
    );
    // Capabilities are host state and never enter the persisted asset.
    assert!(
        !String::from_utf8_lossy(&bytes).contains("capabilities"),
        "transient decode state leaked into the asset"
    );
}

#[test]
fn thumbnail_native_geometry_boundary_uses_small_output_or_a_typed_refusal() {
    for (width, admitted) in [(2048, true), (2049, false)] {
        let name = format!("thumbnail-native-bound-{width}.jpg");
        let input = format!("color=blue:size={width}x1024,format=yuv444p");
        let Some(path) = common::generate(
            "thumbnail native bounds",
            &name,
            &[
                "-f",
                "lavfi",
                "-i",
                &input,
                "-frames:v",
                "1",
                "-c:v",
                "mjpeg",
                "-pix_fmt",
                "yuvj444p",
            ],
        ) else {
            return;
        };
        let result = thumbnail_at_with_read_bytes(
            &path,
            Tick::ZERO,
            THUMBNAIL_MAX_EDGE,
            std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        );
        if admitted {
            let (image, _) = result.unwrap();
            assert_eq!((image.width, image.height), (320, 160));
            assert_eq!(image.pixels.len(), 320 * 160 * 4);
        } else {
            assert!(
                matches!(result, Err(media::MediaError::TooLarge(_))),
                "{result:?}"
            );
        }
    }
}

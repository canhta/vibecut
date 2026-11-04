//! ffmpeg/ffprobe oracle tests for probing, packet reading and muxing.
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod common;

use common::{approx, decoded_frames, decodes, duration, generate, stream_field};
use media::{
    AudioTrackConfig, MuxOptions, Muxer, PacketReader, Probe, StreamInfo, StreamKind,
    VideoTrackConfig, WritePacket, probe,
};
use std::path::{Path, PathBuf};
use time::Rounding;

const SOURCE: &str = "h264_aac_1080p_2997.mp4";
const VFR: &str = "vfr_30_15_640x360.mp4";

/// 10 s, 1080p H.264 + AAC at 29.97 fps.
fn source() -> Option<PathBuf> {
    generate(
        "h264/aac fixture",
        SOURCE,
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=1920x1080:rate=30000/1001:duration=10",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000:duration=10",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-crf",
            "30",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
            "-b:a",
            "96k",
            "-ac",
            "2",
            "-shortest",
        ],
    )
}

/// Variable frame durations: 30 fps for 45 frames, then 15 fps.
fn vfr() -> Option<PathBuf> {
    generate(
        "VFR fixture",
        VFR,
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=640x360:rate=30:duration=3",
            "-vf",
            "setpts='if(lt(N,45),N/30/TB,1.5+(N-45)/15/TB)'",
            "-fps_mode",
            "vfr",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-crf",
            "30",
            "-pix_fmt",
            "yuv420p",
        ],
    )
}

fn kind(probe: &Probe, kind: StreamKind) -> &StreamInfo {
    probe
        .streams
        .iter()
        .find(|stream| stream.kind == kind)
        .expect("expected stream kind")
}

fn packets(path: &Path) -> Vec<media::Packet> {
    let mut reader = PacketReader::open(path).expect("open reader");
    let mut packets = Vec::new();
    while let Some(packet) = reader.next_packet().expect("packet") {
        packets.push(packet);
    }
    packets
}

#[test]
fn probe_and_packets_match_ffprobe_for_h264_aac() {
    let Some(path) = source() else { return };
    let probed = probe(&path).expect("probe");
    assert_eq!(probed.streams.len(), 2);
    let video = kind(&probed, StreamKind::Video);
    let audio = kind(&probed, StreamKind::Audio);

    assert_eq!(video.codec, b"avc1");
    assert_eq!(stream_field(&path, "v:0", "codec_name"), "h264");
    let video_info = video.video.expect("video size");
    assert_eq!(
        video_info.width.to_string(),
        stream_field(&path, "v:0", "width")
    );
    assert_eq!(
        video_info.height.to_string(),
        stream_field(&path, "v:0", "height")
    );
    assert_eq!(stream_field(&path, "v:0", "r_frame_rate"), "30000/1001");
    assert!(!video.vfr, "constant-rate source must not report VFR");
    let avc = video.avc.as_ref().expect("avcC");
    assert!(!avc.sps.is_empty() && !avc.pps.is_empty());
    assert_eq!(video.timescale, 30_000);

    assert_eq!(audio.codec, b"mp4a");
    assert_eq!(stream_field(&path, "a:0", "codec_name"), "aac");
    let audio_info = audio.audio.expect("audio params");
    assert_eq!(
        audio_info.sample_rate.to_string(),
        stream_field(&path, "a:0", "sample_rate")
    );
    assert_eq!(
        audio_info.channels.to_string(),
        stream_field(&path, "a:0", "channels")
    );
    assert!(audio.aac.as_ref().is_some_and(|aac| !aac.asc.is_empty()));
    assert!(
        !audio.edits.is_empty(),
        "ffmpeg writes an AAC priming edit list"
    );

    approx(
        probed.duration.seconds(),
        duration(&path),
        0.05,
        "movie duration",
    );
    assert_eq!(video.sample_count as u64, decoded_frames(&path, "v:0"));

    let packets = packets(&path);
    let video_packets: Vec<_> = packets.iter().filter(|packet| packet.stream == 0).collect();
    let audio_packets: Vec<_> = packets.iter().filter(|packet| packet.stream == 1).collect();
    assert_eq!(video_packets.len(), video.sample_count);
    assert_eq!(audio_packets.len(), audio.sample_count);
    assert_eq!(
        video_packets
            .iter()
            .filter(|packet| packet.keyframe)
            .count(),
        video.keyframe_count
    );
    for pair in video_packets.windows(2) {
        assert!(pair[0].dts <= pair[1].dts, "decode order must advance");
    }
    let total: i64 = video_packets.iter().map(|packet| packet.duration.0).sum();
    approx(
        total as f64 / time::TICKS_PER_SECOND as f64,
        video_packets.len() as f64 * 1001.0 / 30_000.0,
        0.001,
        "video duration from packets",
    );
    assert_eq!(
        audio_packets.first().map(|packet| packet.dts),
        Some(time::Tick(0))
    );
}

#[test]
fn vfr_source_reports_non_uniform_durations() {
    let Some(path) = vfr() else { return };
    let probed = probe(&path).expect("probe");
    let video = kind(&probed, StreamKind::Video);
    assert!(video.vfr, "expected VFR");
    let packets = packets(&path);
    let mut durations: Vec<i64> = packets.iter().map(|packet| packet.duration.0).collect();
    durations.dedup();
    assert!(
        durations.len() > 1,
        "expected more than one packet duration, got {durations:?}"
    );
    assert_eq!(video.sample_count as u64, decoded_frames(&path, "v:0"));
}

#[test]
fn remuxed_h264_aac_matches_source_stream_parameters() {
    let Some(source) = source() else { return };
    let mut reader = PacketReader::open(&source).expect("open source");
    let probed = reader.probe().clone();
    let video = kind(&probed, StreamKind::Video).clone();
    let audio = kind(&probed, StreamKind::Audio).clone();
    let video_index = probed
        .streams
        .iter()
        .position(|stream| stream.kind == StreamKind::Video)
        .expect("video index");
    let audio_index = probed
        .streams
        .iter()
        .position(|stream| stream.kind == StreamKind::Audio)
        .expect("audio index");

    let output = common::fixtures_dir().join("remuxed_1080p.mp4");
    let mut muxer = Muxer::create(&output, MuxOptions::default()).expect("create muxer");
    let video_track = muxer
        .add_video(VideoTrackConfig {
            width: video.video.expect("source video size").width,
            height: video.video.expect("source video size").height,
            timescale: video.timescale,
            avc: video.avc.clone().expect("source avcC"),
        })
        .expect("add video");
    let priming = audio
        .edits
        .iter()
        .find(|edit| edit.media_time > 0)
        .map_or(0, |edit| edit.media_time as u32);
    let audio_track = muxer
        .add_audio(AudioTrackConfig {
            aac: audio.aac.clone().expect("source esds"),
            channels: audio.audio.expect("source audio params").channels,
            sample_rate: audio.audio.expect("source audio params").sample_rate,
            timescale: audio.timescale,
            priming,
        })
        .expect("add audio");

    while let Some(packet) = reader.next_packet().expect("source packet") {
        let stream = probed.streams.get(packet.stream).expect("stream");
        let timescale = i64::from(stream.timescale);
        let track = if packet.stream == video_index {
            video_track
        } else if packet.stream == audio_index {
            audio_track
        } else {
            panic!("unexpected stream {}", packet.stream);
        };
        let duration = packet
            .duration
            .to_units(timescale, Rounding::Nearest)
            .expect("duration units");
        muxer
            .write_packet(
                track,
                WritePacket {
                    dts: packet
                        .dts
                        .to_units(timescale, Rounding::Nearest)
                        .expect("dts units"),
                    pts: packet
                        .pts
                        .to_units(timescale, Rounding::Nearest)
                        .expect("pts units"),
                    duration: u32::try_from(duration).expect("duration fits"),
                    keyframe: packet.keyframe,
                    data: &packet.data,
                },
            )
            .expect("write packet");
    }
    muxer.finish().expect("finish muxer");

    let ours = probe(&output).expect("probe output");
    assert_eq!(ours.streams.len(), 2);
    let our_video = kind(&ours, StreamKind::Video);
    let our_audio = kind(&ours, StreamKind::Audio);
    assert_eq!(our_video.codec, b"avc1");
    assert_eq!(our_audio.codec, b"mp4a");
    for (select, expected) in [("v:0", "h264"), ("a:0", "aac")] {
        assert_eq!(stream_field(&output, select, "codec_name"), expected);
    }
    assert_eq!(
        stream_field(&output, "v:0", "width"),
        stream_field(&source, "v:0", "width")
    );
    assert_eq!(
        stream_field(&output, "v:0", "height"),
        stream_field(&source, "v:0", "height")
    );
    assert_eq!(
        stream_field(&output, "v:0", "r_frame_rate"),
        stream_field(&source, "v:0", "r_frame_rate")
    );
    assert_eq!(
        stream_field(&output, "a:0", "sample_rate"),
        stream_field(&source, "a:0", "sample_rate")
    );
    assert_eq!(
        stream_field(&output, "a:0", "channels"),
        stream_field(&source, "a:0", "channels")
    );
    assert_eq!(our_video.sample_count, video.sample_count);
    assert_eq!(our_audio.sample_count, audio.sample_count);
    assert_eq!(
        decoded_frames(&output, "v:0"),
        decoded_frames(&source, "v:0"),
        "video frame count"
    );
    assert_eq!(
        our_audio.edits.iter().map(|edit| edit.media_time).next(),
        Some(i64::from(priming)),
        "AAC priming is carried into the edit list"
    );
    let expected = duration(&source) - f64::from(priming) / 48_000.0;
    approx(duration(&output), expected, 0.05, "remuxed duration");
    assert!(decodes(&output), "ffmpeg decodes the remuxed file cleanly");
}

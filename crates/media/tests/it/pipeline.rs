//! Media behavior exercised through the public FFmpeg-backed interfaces.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use media::{
    AudioDecoder, Container, DecodePreference, Muxer, PacketReader, ProxyLimits, RgbaConverter,
    StreamKind, VideoDecoder, VideoEncoder, VideoEncoderSettings, VideoReader,
};
use time::Tick;

pub fn source() -> Option<std::path::PathBuf> {
    static SOURCE: std::sync::OnceLock<Option<std::path::PathBuf>> = std::sync::OnceLock::new();
    SOURCE
        .get_or_init(|| {
            testkit::oracle::generate(
                &testkit::fixture::dir("media"),
                "FFmpeg media",
                "ffmpeg_source.mp4",
                &[
                    "-f",
                    "lavfi",
                    "-i",
                    "testsrc2=size=128x96:rate=30000/1001:duration=1",
                    "-f",
                    "lavfi",
                    "-i",
                    "sine=frequency=997:sample_rate=48000:duration=1",
                    "-c:v",
                    "libx264",
                    "-pix_fmt",
                    "yuv420p",
                    "-bf",
                    "2",
                    "-g",
                    "15",
                    "-c:a",
                    "aac",
                    "-ac",
                    "2",
                    "-shortest",
                ],
            )
        })
        .clone()
}

#[test]
fn probe_decode_seek_and_stream_copy() {
    let Some(path) = source() else {
        return;
    };
    let mut reader = PacketReader::open(&path).unwrap();
    let probe = reader.probe().clone();
    assert_eq!(probe.streams.len(), 2);
    let video = probe
        .streams
        .iter()
        .find(|s| s.kind == StreamKind::Video)
        .unwrap();
    let audio = probe
        .streams
        .iter()
        .find(|s| s.kind == StreamKind::Audio)
        .unwrap();
    assert_eq!(video.codec, "h264");
    assert_eq!(audio.codec, "aac");
    assert_eq!(video.video.as_ref().unwrap().width, 128);
    assert_eq!(audio.audio.as_ref().unwrap().sample_rate, 48_000);
    assert_eq!(video.frame_rate.unwrap().num(), 30_000);
    assert_eq!(video.frame_rate.unwrap().den(), 1001);
    let mut decoder = VideoDecoder::new(video, DecodePreference::Software).unwrap();
    let mut audio_decoder = AudioDecoder::new(audio).unwrap();
    let mut pts = Vec::new();
    let mut samples = 0usize;
    let output = testkit::fixture::dir("media").join("ffmpeg_remux.mp4");
    let mut mux = Muxer::create(&output, Container::Mp4).unwrap();
    for stream in &probe.streams {
        mux.add_stream(stream).unwrap();
    }
    while let Some(packet) = reader.next_packet().unwrap() {
        mux.write_packet(packet.stream(), &packet).unwrap();
        if packet.stream() == video.index {
            for frame in decoder.decode(&packet).unwrap() {
                assert!(!frame.is_hardware());
                pts.push(frame.pts().unwrap().unwrap());
            }
        } else {
            for frame in audio_decoder.decode(&packet).unwrap() {
                samples += frame.samples();
                let planes = frame.planar_f32().unwrap();
                assert_eq!(planes.len(), 2);
                assert!(planes.iter().flatten().all(|v| v.is_finite()));
            }
        }
    }
    for frame in decoder.flush().unwrap() {
        pts.push(frame.pts().unwrap().unwrap());
    }
    for frame in audio_decoder.flush().unwrap() {
        samples += frame.samples();
    }
    assert!(decoder.flush().unwrap().is_empty());
    assert!(audio_decoder.flush().unwrap().is_empty());
    mux.finish().unwrap();
    assert_eq!(
        pts.len(),
        testkit::oracle::decoded_frames(&path, "v:0") as usize
    );
    assert!(pts.windows(2).all(|p| p[0] < p[1]));
    assert!(samples >= 48_000);
    assert!(testkit::oracle::decodes(&output));
    assert_eq!(
        testkit::oracle::stream_field(&output, "v:0", "codec_name"),
        "h264"
    );
    assert_eq!(
        testkit::oracle::stream_field(&output, "a:0", "sample_rate"),
        "48000"
    );
    testkit::oracle::approx(
        testkit::oracle::duration(&output),
        testkit::oracle::duration(&path),
        0.05,
        "remux duration",
    );
    let target = pts[20];
    reader.seek(target).unwrap();
    decoder.reset();
    let mut selected = None;
    while let Some(packet) = reader.next_packet().unwrap() {
        if packet.stream() == video.index {
            for frame in decoder.decode(&packet).unwrap() {
                let p = frame.pts().unwrap().unwrap();
                if p >= target {
                    selected.get_or_insert(p);
                }
            }
            if selected.is_some() {
                break;
            }
        }
    }
    if selected.is_none() {
        for frame in decoder.flush().unwrap() {
            let p = frame.pts().unwrap().unwrap();
            if p >= target {
                selected.get_or_insert(p);
            }
        }
    }
    assert_eq!(selected, Some(target));
    println!(
        "{} video frames; {samples} source-rate audio samples; frame-accurate seek at {target:?}; stream copy verified",
        pts.len()
    );
}

#[test]
fn hardware_preference_encodes_and_preserves_color() {
    let Some(path) = source() else {
        return;
    };
    let mut reader = PacketReader::open(&path).unwrap();
    let stream = reader
        .probe()
        .streams
        .iter()
        .find(|s| s.kind == StreamKind::Video)
        .unwrap()
        .clone();
    let mut decoder = VideoDecoder::new(&stream, DecodePreference::Hardware).unwrap();
    let mut frames = Vec::new();
    while let Some(packet) = reader.next_packet().unwrap() {
        if packet.stream() == stream.index {
            frames.extend(decoder.decode(&packet).unwrap());
        }
    }
    frames.extend(decoder.flush().unwrap());
    let first = &frames[0];
    let hardware = first.is_hardware();
    let settings = VideoEncoderSettings::h264(128, 96, stream.frame_rate.unwrap(), first.color());
    let mut encoder = VideoEncoder::new(settings, first).unwrap();
    if cfg!(windows) {
        assert_eq!(encoder.name(), "h264_mf (software)");
    }
    let output = testkit::fixture::dir("media").join("ffmpeg_encoded.mp4");
    let mut mux = Muxer::create(&output, Container::Mp4).unwrap();
    let track = mux
        .add_parameters(&encoder.parameters(), encoder.time_base())
        .unwrap();
    for frame in &frames {
        for packet in encoder.encode(frame).unwrap() {
            mux.write_packet(track, &packet).unwrap();
        }
    }
    for packet in encoder.flush().unwrap() {
        mux.write_packet(track, &packet).unwrap();
    }
    mux.finish().unwrap();
    assert_eq!(
        testkit::oracle::decoded_frames(&output, "v:0"),
        frames.len() as u64
    );
    assert!(testkit::oracle::decodes(&output));
    assert_eq!(
        testkit::oracle::stream_field(&output, "v:0", "color_space"),
        "bt709"
    );
    assert_eq!(
        testkit::oracle::stream_field(&output, "v:0", "color_primaries"),
        "bt709"
    );
    assert_eq!(
        testkit::oracle::stream_field(&output, "v:0", "color_transfer"),
        "bt709"
    );
    println!(
        "hardware frames: {hardware}; encoder: {}; {} frames",
        encoder.name(),
        frames.len()
    );
    let check = testkit::oracle::command("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(&output)
        .args(["-f", "null", "-"])
        .output()
        .unwrap();
    assert!(check.status.success());
}

// Retaining every decoded frame (as the short color oracle above does) hides
// decoder-pool reuse while an asynchronous encoder still consumes its input.
#[cfg(windows)]
#[test]
fn streaming_h264_preserves_every_picture_identity() {
    use media::{EncodePreference, VideoReader};
    use std::time::{Duration, Instant};

    let mut filters = vec!["drawbox=x=0:y=0:w=704:h=64:color=black:t=fill".to_owned()];
    for bit in 0..11 {
        filters.push(format!(
            "drawbox=x={}:y=0:w=64:h=64:color=white:t=fill:enable=bitand(n\\,{})",
            bit * 64,
            1 << bit
        ));
    }
    let filter = filters.join(",");
    let Some(path) = testkit::oracle::generate(
        &testkit::fixture::dir("media"),
        "streaming H.264 picture identity",
        "ffmpeg_picture_identity_1080p60s.mp4",
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=1920x1080:rate=30:duration=60",
            "-vf",
            &filter,
            "-an",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-crf",
            "18",
            "-pix_fmt",
            "yuv420p",
            "-color_range",
            "tv",
            "-colorspace",
            "bt709",
            "-color_trc",
            "bt709",
            "-color_primaries",
            "bt709",
        ],
    ) else {
        return;
    };
    for (label, preference) in [
        ("default", None),
        ("hardware", Some(EncodePreference::Hardware)),
    ] {
        let start = Instant::now();
        let mut reader = VideoReader::open(&path).unwrap();
        let first = reader.next_frame().unwrap().unwrap();
        if !first.is_hardware() {
            eprintln!(
                "skipping sustained D3D11 picture identity: no hardware decoder on this host"
            );
            return;
        }
        let settings =
            VideoEncoderSettings::h264(1920, 1080, time::FrameRate::FPS_30, first.color());
        let mut encoder = match preference {
            Some(preference) => {
                VideoEncoder::with_preference(settings, &first, preference).unwrap()
            }
            None => VideoEncoder::new(settings, &first).unwrap(),
        };
        if preference.is_none() {
            assert_eq!(encoder.name(), "h264_mf (software)");
        }
        let output = testkit::fixture::dir("media").join(format!("picture-identity-{label}.mp4"));
        let mut mux = Muxer::create(&output, Container::Mp4).unwrap();
        let track = mux
            .add_parameters(&encoder.parameters(), encoder.time_base())
            .unwrap();
        let mut hardware = 0;
        let mut count = 0;
        let mut encode_time = Duration::ZERO;
        let mut next = Some(first);
        while let Some(frame) = next {
            hardware += usize::from(frame.is_hardware());
            assert_eq!(
                frame.pts().unwrap(),
                Some(time::FrameRate::FPS_30.tick_of(count).unwrap())
            );
            let mark = Instant::now();
            let packets = encoder.encode(&frame).unwrap();
            encode_time += mark.elapsed();
            for packet in packets {
                mux.write_packet(track, &packet).unwrap();
            }
            count += 1;
            next = reader.next_frame().unwrap();
        }
        let mark = Instant::now();
        let packets = encoder.flush().unwrap();
        encode_time += mark.elapsed();
        for packet in packets {
            mux.write_packet(track, &packet).unwrap();
        }
        mux.finish().unwrap();
        assert_eq!(count, 1800);
        assert_eq!(hardware, 1800);
        println!(
            "picture_identity mode={label} encoder={} frames={count} hardware_frames={hardware} encode_seconds={:.6} export_seconds={:.6}",
            encoder.name(),
            encode_time.as_secs_f64(),
            start.elapsed().as_secs_f64()
        );
        let mut decoded = VideoReader::open(&output).unwrap();
        let mut checked = 0;
        while let Some(frame) = decoded.next_frame().unwrap() {
            assert_eq!(
                frame.pts().unwrap(),
                Some(time::FrameRate::FPS_30.tick_of(checked).unwrap())
            );
            let cpu = frame.download().unwrap();
            let stride = cpu.stride(0).unwrap();
            let luma = cpu.plane(0).unwrap();
            let identity = (0..11).fold(0, |identity, bit| {
                identity | (i64::from(luma[32 * stride + bit * 64 + 32] > 128) << bit)
            });
            assert_eq!(
                identity, checked,
                "{label}: wrong picture at output frame {checked}"
            );
            checked += 1;
        }
        assert_eq!(checked, 1800);
        assert_eq!(
            testkit::oracle::stream_field(&output, "v:0", "color_range"),
            "tv"
        );
        for tag in ["color_space", "color_primaries", "color_transfer"] {
            assert_eq!(testkit::oracle::stream_field(&output, "v:0", tag), "bt709");
        }
    }
}

#[test]
fn timestamps_do_not_round_trip_through_float() {
    let base = media::TimeBase::new(1001, 30_000).unwrap();
    for i in [i64::MIN / 1001, -30, 0, 1, 30, 108_000] {
        if let Ok(tick) = base.ticks(i) {
            assert_eq!(base.units(tick, time::Rounding::Nearest).unwrap(), i);
        }
    }
    assert!(media::TimeBase::new(0, 1).is_err());
    assert!(media::TimeBase::new(1, 0).is_err());
    let _ = Tick::ZERO;
}

#[test]
fn long_1080p_encode_drains_every_delayed_frame() {
    if !testkit::oracle::require("long encoder drain") {
        return;
    }
    let rate = time::FrameRate::new(30, 1).unwrap();
    let first = media::VideoFrame::from_rgba8(
        1920,
        1080,
        Tick::ZERO,
        media::ColorInfo::default(),
        &vec![128; 1920 * 1080 * 4],
    )
    .unwrap();
    let settings = VideoEncoderSettings::h264(1920, 1080, rate, first.color());
    let mut encoder =
        VideoEncoder::with_preference(settings, &first, media::EncodePreference::Software).unwrap();
    std::fs::create_dir_all(testkit::fixture::dir("media")).unwrap();
    let output = testkit::fixture::dir("media").join("long_encoder_drain.mp4");
    let mut mux = Muxer::create(&output, Container::Mp4).unwrap();
    let track = mux
        .add_parameters(&encoder.parameters(), encoder.time_base())
        .unwrap();
    let mut packets = 0;
    for index in 0..1800 {
        let frame = first
            .with_pts(Tick::from_units(index, 30, time::Rounding::Nearest).unwrap())
            .unwrap();
        for packet in encoder.encode(&frame).unwrap() {
            mux.write_packet(track, &packet).unwrap();
            packets += 1;
        }
    }
    let delayed = encoder.flush().unwrap();
    println!("{}: {} delayed packets", encoder.name(), delayed.len());
    for packet in delayed {
        mux.write_packet(track, &packet).unwrap();
        packets += 1;
    }
    assert_eq!(packets, 1800);
    assert!(encoder.flush().unwrap().is_empty());
    assert!(matches!(
        encoder.encode(&first),
        Err(media::MediaError::Invalid(_))
    ));
    mux.finish().unwrap();
    assert_eq!(testkit::oracle::decoded_frames(&output, "v:0"), 1800);
    testkit::oracle::approx(
        testkit::oracle::duration(&output),
        60.0,
        1.0 / 30.0,
        "drained duration",
    );
    assert!(testkit::oracle::decodes(&output));
}

#[test]
fn audio_encode_and_resample_preserve_clock_and_sample_counts() {
    let Some(path) = source() else {
        return;
    };
    let mut reader = PacketReader::open(&path).unwrap();
    let stream = reader
        .probe()
        .streams
        .iter()
        .find(|s| s.kind == StreamKind::Audio)
        .unwrap()
        .clone();
    let mut decoder = AudioDecoder::new(&stream).unwrap();
    let mut encoder =
        media::AudioEncoder::new(media::AudioEncoderSettings::aac(48_000, 2)).unwrap();
    let output = testkit::fixture::dir("media").join("ffmpeg_audio.m4a");
    let mut mux = Muxer::create(&output, Container::Mp4).unwrap();
    let track = mux
        .add_parameters(&encoder.parameters(), encoder.time_base())
        .unwrap();
    let mut resampler = None;
    let mut mono = Vec::new();
    let mut input_samples = 0;
    let mut consume = |frame: media::AudioFrame| {
        input_samples += frame.samples();
        for packet in encoder.encode(&frame).unwrap() {
            mux.write_packet(track, &packet).unwrap();
        }
        if resampler.is_none() {
            resampler = Some(
                media::AudioResampler::new(&frame, 16_000, 1, media::AudioSampleFormat::PlanarF32)
                    .unwrap(),
            );
        }
        for converted in resampler.as_mut().unwrap().convert(&frame).unwrap() {
            assert_eq!(
                converted.pts().unwrap(),
                Some(Tick::from_units(mono.len() as i64, 16_000, time::Rounding::Nearest).unwrap())
            );
            mono.extend(converted.planar_f32().unwrap().into_iter().next().unwrap());
        }
    };
    while let Some(packet) = reader.next_packet().unwrap() {
        if packet.stream() == stream.index {
            for frame in decoder.decode(&packet).unwrap() {
                consume(frame);
            }
        }
    }
    for frame in decoder.flush().unwrap() {
        consume(frame);
    }
    drop(consume);
    while let Some(frame) = resampler.as_mut().unwrap().flush().unwrap() {
        mono.extend(frame.planar_f32().unwrap().into_iter().next().unwrap());
    }
    for packet in encoder.flush().unwrap() {
        mux.write_packet(track, &packet).unwrap();
    }
    mux.finish().unwrap();
    assert_eq!(input_samples, 48_000);
    assert_eq!(mono.len(), 16_000);
    testkit::oracle::approx(
        testkit::oracle::duration(&output),
        1.0,
        1.0 / 48_000.0,
        "AAC duration including priming",
    );
    assert!(testkit::oracle::decodes(&output));
    // FFmpeg before 9 decodes the trailing AAC frame padding that the edit list
    // excludes (fixed in 68b53a8dbfdf). Bound the oracle to the fixture's 1 s
    // presentation interval so every admitted version reports the same samples.
    let reference = testkit::oracle::command("ffmpeg")
        .args(["-v", "error", "-t", "1.0", "-i"])
        .arg(&path)
        .args(["-vn", "-ac", "1", "-ar", "16000", "-f", "f32le", "-"])
        .output()
        .unwrap();
    assert!(reference.status.success());
    let samples: Vec<f32> = reference
        .stdout
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| f32::from_le_bytes(*b))
        .collect();
    assert_eq!(samples.len(), mono.len());
    let maximum = mono
        .iter()
        .zip(samples)
        .map(|(a, b)| (a - b).abs())
        .fold(0f32, f32::max);
    assert!(maximum < 1e-5, "resampling error {maximum}");
    println!(
        "AAC source-rate encoding and 48k->16k mono: {input_samples} -> {} samples, max PCM error {maximum:e}",
        mono.len()
    );
}

#[test]
fn webm_vp9_scaling_has_bt709_tags_and_complete_frames() {
    let Some(path) = source() else {
        return;
    };
    let mut reader = PacketReader::open(&path).unwrap();
    let stream = reader
        .probe()
        .streams
        .iter()
        .find(|s| s.kind == StreamKind::Video)
        .unwrap()
        .clone();
    let mut decoder = VideoDecoder::new(&stream, DecodePreference::Software).unwrap();
    let mut frames = Vec::new();
    while let Some(packet) = reader.next_packet().unwrap() {
        if packet.stream() == stream.index {
            frames.extend(decoder.decode(&packet).unwrap());
        }
    }
    frames.extend(decoder.flush().unwrap());
    let mut settings =
        VideoEncoderSettings::h264(64, 48, stream.frame_rate.unwrap(), frames[0].color());
    settings.webm = true;
    let mut encoder = VideoEncoder::new(settings, &frames[0]).unwrap();
    let path = testkit::fixture::dir("media").join("ffmpeg_scaled.webm");
    let mut mux = Muxer::create(&path, Container::WebM).unwrap();
    let track = mux
        .add_parameters(&encoder.parameters(), encoder.time_base())
        .unwrap();
    for frame in &frames {
        for packet in encoder.encode(frame).unwrap() {
            mux.write_packet(track, &packet).unwrap();
        }
    }
    for packet in encoder.flush().unwrap() {
        mux.write_packet(track, &packet).unwrap();
    }
    mux.finish().unwrap();
    assert_eq!(
        testkit::oracle::decoded_frames(&path, "v:0"),
        frames.len() as u64
    );
    assert_eq!(
        testkit::oracle::stream_field(&path, "v:0", "codec_name"),
        "vp9"
    );
    assert_eq!(testkit::oracle::stream_field(&path, "v:0", "width"), "64");
    assert_eq!(
        testkit::oracle::stream_field(&path, "v:0", "color_space"),
        "bt709"
    );
    assert!(testkit::oracle::decodes(&path));
}

#[test]
fn opus_reframes_aac_without_changing_duration() {
    let Some(path) = source() else {
        return;
    };
    let mut reader = PacketReader::open(&path).unwrap();
    let stream = reader
        .probe()
        .streams
        .iter()
        .find(|s| s.kind == StreamKind::Audio)
        .unwrap()
        .clone();
    let mut decoder = AudioDecoder::new(&stream).unwrap();
    let mut settings = media::AudioEncoderSettings::aac(48_000, 2);
    settings.webm = true;
    let mut encoder = media::AudioEncoder::new(settings).unwrap();
    let output = testkit::fixture::dir("media").join("ffmpeg_opus.webm");
    let mut mux = Muxer::create(&output, Container::WebM).unwrap();
    let track = mux
        .add_parameters(&encoder.parameters(), encoder.time_base())
        .unwrap();
    while let Some(packet) = reader.next_packet().unwrap() {
        if packet.stream() == stream.index {
            for frame in decoder.decode(&packet).unwrap() {
                for packet in encoder.encode(&frame).unwrap() {
                    mux.write_packet(track, &packet).unwrap();
                }
            }
        }
    }
    for frame in decoder.flush().unwrap() {
        for packet in encoder.encode(&frame).unwrap() {
            mux.write_packet(track, &packet).unwrap();
        }
    }
    for packet in encoder.flush().unwrap() {
        mux.write_packet(track, &packet).unwrap();
    }
    mux.finish().unwrap();
    assert_eq!(
        testkit::oracle::stream_field(&output, "a:0", "codec_name"),
        "opus"
    );
    testkit::oracle::approx(
        testkit::oracle::duration(&output),
        1.0,
        0.021,
        "Opus duration including padding",
    );
    assert!(testkit::oracle::decodes(&output));
}

#[test]
fn hdr_is_preserved_by_stream_copy_and_refused_by_sdr_encoder() {
    let Some(path) = testkit::oracle::generate(
        &testkit::fixture::dir("media"),
        "HDR test",
        "ffmpeg_hdr_pq10.mp4",
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=64x48:rate=25",
            "-frames:v",
            "3",
            "-an",
            "-c:v",
            "libx265",
            "-pix_fmt",
            "yuv420p10le",
            "-x265-params",
            "log-level=error:colorprim=9:transfer=16:colormatrix=9",
            "-color_primaries",
            "bt2020",
            "-color_trc",
            "smpte2084",
            "-colorspace",
            "bt2020nc",
        ],
    ) else {
        return;
    };
    let mut reader = PacketReader::open(&path).unwrap();
    let stream = reader.probe().streams[0].clone();
    let mut decoder = VideoDecoder::new(&stream, DecodePreference::Software).unwrap();
    let output = testkit::fixture::dir("media").join("ffmpeg_hdr_copy.mp4");
    let mut mux = Muxer::create(&output, Container::Mp4).unwrap();
    let track = mux.add_stream(&stream).unwrap();
    let mut frames = Vec::new();
    while let Some(packet) = reader.next_packet().unwrap() {
        mux.write_packet(track, &packet).unwrap();
        frames.extend(decoder.decode(&packet).unwrap());
    }
    frames.extend(decoder.flush().unwrap());
    assert_eq!(frames.len(), 3);
    assert_eq!(frames[0].bit_depth(), Some(10));
    assert!(frames[0].color().is_hdr());
    let settings =
        VideoEncoderSettings::h264(64, 48, stream.frame_rate.unwrap(), frames[0].color());
    assert!(matches!(
        VideoEncoder::new(settings, &frames[0]),
        Err(media::MediaError::Unsupported(_))
    ));
    mux.finish().unwrap();
    assert_eq!(
        testkit::oracle::stream_field(&output, "v:0", "color_transfer"),
        "smpte2084"
    );
    assert_eq!(
        testkit::oracle::stream_field(&output, "v:0", "color_primaries"),
        "bt2020"
    );
    assert_eq!(testkit::oracle::decoded_frames(&output, "v:0"), 3);
}

#[test]
fn speech_pcm_constructor_and_short_final_aac_frame() {
    assert!(media::AudioFrame::from_planar_f32(Tick::ZERO, 16_000, &[vec![f32::NAN]]).is_err());
    let mut encoder =
        media::AudioEncoder::new(media::AudioEncoderSettings::aac(16_000, 1)).unwrap();
    let path = testkit::fixture::dir("media").join("speech-contract.m4a");
    std::fs::create_dir_all(testkit::fixture::dir("media")).unwrap();
    let mut mux = Muxer::create(&path, Container::Mp4).unwrap();
    let track = mux
        .add_parameters(&encoder.parameters(), encoder.time_base())
        .unwrap();
    for (start, count) in [(0, 700), (700, 1300), (2000, 333)] {
        let pcm = vec![vec![0.0; count]];
        let frame = media::AudioFrame::from_planar_f32(
            Tick::from_units(start, 16_000, time::Rounding::Nearest).unwrap(),
            16_000,
            &pcm,
        )
        .unwrap();
        for packet in encoder.encode(&frame).unwrap() {
            mux.write_packet(track, &packet).unwrap();
        }
    }
    for packet in encoder.flush().unwrap() {
        mux.write_packet(track, &packet).unwrap();
    }
    mux.finish().unwrap();
    assert!(media::PacketReader::open(&path).is_ok());
    if testkit::oracle::require("speech contract") {
        testkit::oracle::approx(
            testkit::oracle::duration(&path),
            2333.0 / 16_000.0,
            1.0 / 16_000.0,
            "short final AAC frame",
        );
        assert!(testkit::oracle::decodes(&path));
    }
}

#[test]
fn audio_file_writer_streams_blocks_into_one_decodable_file() {
    let path = testkit::fixture::dir("media").join("audio-file-writer.m4a");
    std::fs::create_dir_all(testkit::fixture::dir("media")).unwrap();
    let mut writer = media::AudioFileWriter::create(&path, 48_000, 1).unwrap();
    assert!(
        writer.write(&[vec![0.0], vec![0.0]]).is_err(),
        "channel count"
    );
    assert!(writer.write(&[vec![f32::NAN]]).is_err(), "non-finite PCM");
    for count in [4800, 1, 9600, media::AUDIO_FILE_BLOCK] {
        writer.write(&[vec![0.1; count]]).unwrap();
    }
    let total = 4800 + 1 + 9600 + media::AUDIO_FILE_BLOCK as u64;
    assert_eq!(writer.samples(), total);
    assert_eq!(writer.finish().unwrap(), total);
    assert!(media::PacketReader::open(&path).is_ok());
    if testkit::oracle::require("audio file writer") {
        testkit::oracle::approx(
            testkit::oracle::duration(&path),
            total as f64 / 48_000.0,
            2.0 / 48_000.0,
            "sample count is the duration",
        );
        assert!(testkit::oracle::decodes(&path));
    }
}

#[test]
fn cross_container_codec_tags() {
    let Some(path) = testkit::oracle::generate(
        &testkit::fixture::dir("media"),
        "AVI stream copy",
        "codec-tag.avi",
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=64x48:rate=25:duration=0.2",
            "-c:v",
            "libx264",
            "-bf",
            "0",
            "-an",
        ],
    ) else {
        return;
    };
    let mut reader = PacketReader::open(path).unwrap();
    let output = testkit::fixture::dir("media").join("codec-tag.mp4");
    let mut mux = Muxer::create(&output, Container::Mp4).unwrap();
    mux.add_stream(&reader.probe().streams[0]).unwrap();
    while let Some(packet) = reader.next_packet().unwrap() {
        mux.write_packet(0, &packet).unwrap();
    }
    mux.finish().unwrap();
    assert_eq!(testkit::oracle::decoded_frames(&output, "v:0"), 5);
    assert!(testkit::oracle::decodes(&output));
}

#[test]
fn maximum_audio_frames_resample_without_loss() {
    for (source_rate, rate) in [(48_000, 48_000), (48_000, 192_000), (8_000, 192_000)] {
        let mut converter = None;
        let mut samples = 0;
        for index in 0..3 {
            let input = media::AudioFrame::from_planar_f32(
                Tick::from_units(index * 65_536, source_rate, time::Rounding::Nearest).unwrap(),
                source_rate as u32,
                &[vec![0.25; 65_536]],
            )
            .unwrap();
            let converter = converter.get_or_insert_with(|| {
                media::AudioResampler::new(&input, rate, 1, media::AudioSampleFormat::PlanarF32)
                    .unwrap()
            });
            for output in converter.convert(&input).unwrap() {
                assert!(output.samples() <= 65_536);
                assert_eq!(
                    output.pts().unwrap().unwrap(),
                    Tick::from_units(samples, i64::from(rate), time::Rounding::Nearest).unwrap()
                );
                samples += output.samples() as i64;
            }
        }
        let converter = converter.as_mut().unwrap();
        while let Some(output) = converter.flush().unwrap() {
            assert!(output.samples() <= 65_536);
            samples += output.samples() as i64;
        }
        assert_eq!(samples, 3 * 65_536 * i64::from(rate / source_rate as u32));
        assert!(converter.flush().unwrap().is_none());
    }
}

#[test]
fn source_asset_import_retains_native_path_and_persistable_probe() {
    let Some(original) = source() else {
        return;
    };
    let path = testkit::fixture::dir("media").join("nguồn phim dự án.mp4");
    std::fs::copy(original, &path).unwrap();
    let asset = media::SourceAsset::import(media::AssetId(42), path.clone()).unwrap();
    assert_eq!(asset.path(), std::fs::canonicalize(&path).unwrap());
    assert_eq!(
        asset.content_hash(),
        media::ContentHash::read(&path).unwrap()
    );
    assert_eq!(
        asset.availability().unwrap(),
        media::AssetAvailability::Present
    );
    assert!(asset.probe().duration.is_some());
    let video = asset
        .probe()
        .streams
        .iter()
        .find(|s| s.kind == StreamKind::Video)
        .unwrap();
    assert_eq!(video.frame_rate, Some(time::FrameRate::FPS_29_97));
    assert_eq!(video.video.as_ref().unwrap().width, 128);
    let audio = asset
        .probe()
        .streams
        .iter()
        .find(|s| s.kind == StreamKind::Audio)
        .unwrap();
    assert_eq!(audio.audio.as_ref().unwrap().sample_rate, 48000);
    assert!(!format!("{asset:?}").contains("nguồn"));
}

#[cfg(target_os = "macos")]
#[test]
fn software_h264_open_rejection_reports_the_native_failure() {
    let color = media::ColorInfo::default();
    let first = media::VideoFrame::from_rgba8(2, 2, Tick::ZERO, color, &[0; 16]).unwrap();
    let mut settings = VideoEncoderSettings::h264(64, 64, time::FrameRate::FPS_30, color);
    // The native signed bitrate field rejects this value during codec open.
    settings.bit_rate = usize::MAX;
    let result = VideoEncoder::with_preference(settings, &first, media::EncodePreference::Software);
    assert!(matches!(
        result,
        Err(media::MediaError::Ffmpeg {
            operation: "open video encoder",
            ..
        })
    ));
}

#[test]
fn bounded_conversion_threads_preserve_all_decoded_rgba_rows() {
    for threads in [0, 9, u8::MAX] {
        assert!(matches!(
            media::RgbaConverter::with_threads(threads),
            Err(media::MediaError::Invalid(_))
        ));
    }
    let Some(path) = source() else {
        return;
    };
    for preference in [DecodePreference::Software, DecodePreference::Hardware] {
        let mut reader = PacketReader::open(&path).unwrap();
        let stream = reader
            .probe()
            .streams
            .iter()
            .find(|s| s.kind == StreamKind::Video)
            .unwrap()
            .clone();
        let mut decoder = VideoDecoder::new(&stream, preference).unwrap();
        let mut sequential = media::RgbaConverter::new().unwrap();
        let mut parallel = media::RgbaConverter::with_threads(4).unwrap();
        let mut frames = 0;
        let mut compare = |frame: media::VideoFrame| {
            let expected = sequential.convert(&frame).unwrap();
            let actual = parallel.convert(&frame).unwrap();
            assert_eq!(actual.pts().unwrap(), expected.pts().unwrap());
            assert_eq!(actual.color(), expected.color());
            assert_eq!(actual.premultiplied_alpha(), expected.premultiplied_alpha());
            let width = actual.width() as usize * 4;
            for (a, b) in actual
                .plane(0)
                .unwrap()
                .chunks(actual.stride(0).unwrap())
                .zip(
                    expected
                        .plane(0)
                        .unwrap()
                        .chunks(expected.stride(0).unwrap()),
                )
            {
                assert_eq!(&a[..width], &b[..width]);
            }
            frames += 1;
        };
        while let Some(packet) = reader.next_packet().unwrap() {
            if packet.stream() == stream.index {
                for frame in decoder.decode(&packet).unwrap() {
                    compare(frame);
                }
            }
        }
        for frame in decoder.flush().unwrap() {
            compare(frame);
        }
        assert!(frames >= 29);
    }
}

#[test]
fn aligned_horizontal_conversion_matches_full_frame_at_fractional_crop_neighbors() {
    let Some(path) = testkit::oracle::generate(
        &testkit::fixture::dir("media"),
        "conversion windows",
        "conversion-window.mp4",
        &[
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=770x130:rate=30:duration=0.1",
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-color_primaries",
            "bt709",
            "-color_trc",
            "bt709",
            "-colorspace",
            "bt709",
        ],
    ) else {
        return;
    };
    for preference in [DecodePreference::Software, DecodePreference::Hardware] {
        let mut reader = PacketReader::open(&path).unwrap();
        let stream = reader
            .probe()
            .streams
            .iter()
            .find(|s| s.kind == StreamKind::Video)
            .unwrap()
            .clone();
        let mut decoder = VideoDecoder::new(&stream, preference).unwrap();
        let mut full = media::RgbaConverter::new().unwrap();
        let mut windowed = media::RgbaConverter::with_threads(4).unwrap();
        let mut frames = Vec::new();
        while let Some(packet) = reader.next_packet().unwrap() {
            if packet.stream() == stream.index {
                frames.extend(decoder.decode(&packet).unwrap());
            }
        }
        frames.extend(decoder.flush().unwrap());
        assert_eq!(frames.len(), 3);
        for frame in frames {
            let reference = full.convert(&frame).unwrap();
            for range in [0..1, 1..2, 127..128, 128..129, 255..433, 769..770, 0..770] {
                let (actual, origin) = windowed.convert_horizontal(&frame, range.clone()).unwrap();
                assert!(origin <= range.start && origin + actual.width() >= range.end);
                assert_eq!(actual.pts().unwrap(), reference.pts().unwrap());
                assert_eq!(actual.color(), reference.color());
                for (a, b) in actual
                    .plane(0)
                    .unwrap()
                    .chunks(actual.stride(0).unwrap())
                    .zip(
                        reference
                            .plane(0)
                            .unwrap()
                            .chunks(reference.stride(0).unwrap()),
                    )
                {
                    assert_eq!(
                        &a[(range.start - origin) as usize * 4..(range.end - origin) as usize * 4],
                        &b[range.start as usize * 4..range.end as usize * 4]
                    );
                }
            }
            // Ranges are hostile API data, not iterator bounds.
            #[allow(clippy::reversed_empty_ranges)]
            for range in [0..0, 770..771, 2..1] {
                assert!(matches!(
                    windowed.convert_horizontal(&frame, range),
                    Err(media::MediaError::Invalid(_))
                ));
            }
        }
    }
}

struct GifFile {
    width: u16,
    height: u16,
    delays: Vec<u16>,
    images: usize,
    looped: bool,
    trailer: bool,
}
/// Walks the GIF block structure without decoding pixel data.
fn gif_file(bytes: &[u8]) -> GifFile {
    assert_eq!(&bytes[..6], b"GIF89a");
    let mut file = GifFile {
        width: u16::from_le_bytes([bytes[6], bytes[7]]),
        height: u16::from_le_bytes([bytes[8], bytes[9]]),
        delays: Vec::new(),
        images: 0,
        looped: false,
        trailer: false,
    };
    let mut at = 13;
    if bytes[10] & 0x80 != 0 {
        at += 3 << ((bytes[10] & 7) + 1);
    }
    let skip = |mut at: usize| loop {
        let size = bytes[at] as usize;
        at += 1;
        if size == 0 {
            return at;
        }
        at += size;
    };
    loop {
        match bytes[at] {
            0x3B => {
                file.trailer = true;
                return file;
            }
            0x21 => {
                let label = bytes[at + 1];
                at += 2;
                if label == 0xF9 {
                    file.delays
                        .push(u16::from_le_bytes([bytes[at + 2], bytes[at + 3]]));
                }
                if label == 0xFF && bytes[at + 1..].starts_with(b"NETSCAPE2.0") {
                    file.looped = true;
                }
                at = skip(at);
            }
            0x2C => {
                file.images += 1;
                let flags = bytes[at + 9];
                at += 10;
                if flags & 0x80 != 0 {
                    at += 3 << ((flags & 7) + 1);
                }
                at = skip(at + 1);
            }
            other => panic!("unexpected GIF block {other:#x}"),
        }
    }
}

#[test]
fn gif_encoder_keeps_frame_count_timing_loop_and_refuses_hostile_input() {
    use media::{GifDither, GifEncoder, GifEncoderSettings, GifOptions};
    use time::FrameRate;
    let (width, height) = (48u32, 32u32);
    let frames = |n: u32| -> Vec<u8> {
        (0..width * height)
            .flat_map(|p| {
                let (x, y) = (p % width, p / width);
                [(x * 5 + n * 7) as u8, (y * 7) as u8, (n * 13) as u8, 255]
            })
            .collect()
    };
    for (rate, count, expected) in [(FrameRate::FPS_25, 10u32, 4u16), (FrameRate::FPS_30, 30, 0)] {
        let settings = GifEncoderSettings {
            width,
            height,
            frame_rate: rate,
            options: GifOptions {
                max_colors: 16,
                dither: GifDither::Bayer,
            },
        };
        let mut encoder = GifEncoder::new(settings).unwrap();
        let path = testkit::fixture::dir("media").join(format!("gif_{}.gif", rate.num()));
        let mut mux = Muxer::create(&path, Container::Gif).unwrap();
        let track = mux
            .add_parameters(&encoder.parameters(), encoder.time_base())
            .unwrap();
        for n in 0..count {
            let pts = rate.tick_of(i64::from(n)).unwrap();
            for packet in encoder.encode_rgba8(&frames(n), pts).unwrap() {
                mux.write_packet(track, &packet).unwrap();
            }
        }
        for packet in encoder.flush().unwrap() {
            mux.write_packet(track, &packet).unwrap();
        }
        mux.finish().unwrap();
        let file = gif_file(&std::fs::read(&path).unwrap());
        assert_eq!((file.width, file.height), (48, 32));
        assert_eq!(file.images, count as usize);
        assert_eq!(file.delays.len(), count as usize);
        assert!(file.looped && file.trailer);
        if expected != 0 {
            assert!(
                file.delays.iter().all(|&d| d == expected),
                "{:?}",
                file.delays
            );
        } else {
            // 30 fps is 3.33 cs: delays alternate 3 and 4 and sum to one second.
            assert!(
                file.delays.iter().all(|&d| d == 3 || d == 4),
                "{:?}",
                file.delays
            );
            let total: u16 = file.delays.iter().sum();
            assert!((99..=101).contains(&total), "{total}");
        }
    }

    let settings = GifEncoderSettings {
        width,
        height,
        frame_rate: FrameRate::FPS_25,
        options: GifOptions::default(),
    };
    let mut encoder = GifEncoder::new(settings).unwrap();
    assert!(matches!(
        encoder.encode_rgba8(&frames(0)[1..], Tick::ZERO),
        Err(media::MediaError::Invalid(_))
    ));
    // The first frame sits at zero and later frames follow the grid exactly.
    assert!(matches!(
        encoder.encode_rgba8(&frames(0), FrameRate::FPS_25.tick_of(1).unwrap()),
        Err(media::MediaError::Invalid(_))
    ));
    encoder.encode_rgba8(&frames(0), Tick::ZERO).unwrap();
    assert!(matches!(
        encoder.encode_rgba8(&frames(1), FrameRate::FPS_25.tick_of(2).unwrap()),
        Err(media::MediaError::Invalid(_))
    ));
    encoder.flush().unwrap();
    assert!(matches!(
        encoder.encode_rgba8(&frames(1), FrameRate::FPS_25.tick_of(1).unwrap()),
        Err(media::MediaError::Invalid(_))
    ));
    assert!(media::export_container_available(Container::Gif));
    assert!(matches!(
        Container::from_extension("GIF"),
        Some(Container::Gif)
    ));
}

/// Recorded bound: mean absolute RGB difference per channel (of 255) between a
/// source frame and its proxy frame, both reduced to 32 px. An engineering bound
/// set before the first measurement; tighten it from the first recorded run.
const PROXY_FRAME_TOLERANCE: f64 = 12.0;

fn reduced_frames(path: &std::path::Path) -> Vec<(Tick, Vec<u8>)> {
    let mut reader = VideoReader::open(path).unwrap();
    let mut converter = RgbaConverter::new().unwrap();
    let mut frames = Vec::new();
    while let Some(frame) = reader.next_frame().unwrap() {
        let pts = frame.pts().unwrap().unwrap();
        let small = converter.convert_scaled(&frame, 32, 24).unwrap();
        let stride = small.stride(0).unwrap();
        let plane = small.plane(0).unwrap();
        let mut rgb = Vec::new();
        for row in 0..24 {
            for pixel in plane[row * stride..row * stride + 128].chunks(4) {
                rgb.extend_from_slice(&pixel[..3]);
            }
        }
        frames.push((pts, rgb));
    }
    frames
}

#[test]
fn proxy_is_small_intra_only_and_matches_source_frames_within_tolerance() {
    let Some(path) = source() else {
        return;
    };
    let output = testkit::fixture::dir("media").join("ffmpeg_proxy.mp4");
    let limits = ProxyLimits {
        max_edge: 64,
        max_bytes: 8 << 20,
    };
    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let report = media::write_proxy(&path, &output, limits, cancel, |_| {}).unwrap();
    assert_eq!((report.width, report.height), (64, 48));
    assert_eq!(testkit::oracle::stream_field(&output, "v:0", "width"), "64");
    assert_eq!(
        testkit::oracle::decoded_frames(&output, "v:0"),
        testkit::oracle::decoded_frames(&path, "v:0"),
        "the proxy keeps the source's frame grid"
    );
    let packets = testkit::oracle::command("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "packet=flags",
            "-of",
            "csv=p=0",
        ])
        .arg(&output)
        .output()
        .unwrap();
    let flags = String::from_utf8(packets.stdout).unwrap();
    assert!(
        flags.lines().all(|flag| flag.starts_with('K')),
        "every proxy picture is a keyframe: {flags}"
    );
    let (source_frames, proxy_frames) = (reduced_frames(&path), reduced_frames(&output));
    assert_eq!(source_frames.len(), proxy_frames.len());
    for ((source_at, source), (proxy_at, proxy)) in source_frames.iter().zip(&proxy_frames) {
        assert!(
            (source_at.0 - proxy_at.0).abs() <= 1,
            "{source_at:?} {proxy_at:?}"
        );
        let difference = source
            .iter()
            .zip(proxy)
            .map(|(a, b)| f64::from(a.abs_diff(*b)))
            .sum::<f64>()
            / source.len() as f64;
        assert!(
            difference <= PROXY_FRAME_TOLERANCE,
            "frame at {source_at:?} differs by {difference}"
        );
    }
}

#[test]
fn proxy_refuses_an_output_past_the_byte_limit_and_cancellation() {
    let Some(path) = source() else {
        return;
    };
    let output = testkit::fixture::dir("media").join("ffmpeg_proxy_refused.mp4");
    let idle = || std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let small = ProxyLimits {
        max_edge: 64,
        max_bytes: 16,
    };
    assert!(matches!(
        media::write_proxy(&path, &output, small, idle(), |_| {}),
        Err(media::MediaError::TooLarge("proxy bytes"))
    ));
    let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let limits = ProxyLimits {
        max_edge: 64,
        max_bytes: 8 << 20,
    };
    assert!(media::write_proxy(&path, &output, limits, cancelled, |_| {}).is_err());
}

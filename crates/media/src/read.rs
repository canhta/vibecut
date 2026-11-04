use crate::{
    CodecParameters, MediaError, Packet, Probe, Result, StreamInfo, StreamKind, TimeBase, error,
    ffi, init,
};
use ffmpeg_next as ff;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use time::{FrameRate, Rounding, Tick};

const MAX_PACKET: usize = 64 << 20;
struct Input {
    file: File,
    bytes: Arc<AtomicU64>,
    budget: Arc<AtomicU64>,
}
impl Read for Input {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        let remaining = self.budget.load(Ordering::Relaxed);
        if remaining == 0 {
            return Err(io::Error::other("media probe read budget exhausted"));
        }
        let take = bytes.len().min(remaining.min(usize::MAX as u64) as usize);
        let n = self.file.read(
            bytes
                .get_mut(..take)
                .ok_or_else(|| io::Error::other("read bounds"))?,
        )?;
        self.bytes.fetch_add(n as u64, Ordering::Relaxed);
        self.budget.fetch_sub(n as u64, Ordering::Relaxed);
        Ok(n)
    }
}
impl Seek for Input {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.file.seek(pos)
    }
}

pub struct PacketReader {
    pub(crate) input: ff::format::context::Input,
    probe: Probe,
    cancel: Arc<AtomicBool>,
    bytes: Arc<AtomicU64>,
}
impl PacketReader {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_with_cancel(path, Arc::new(AtomicBool::new(false)))
    }
    /// Opens native paths through Rust File + seekable AVIO (32 KiB buffer), with
    /// at most 32 MiB read while probing. The token also interrupts FFmpeg probing.
    pub fn open_with_cancel(path: impl AsRef<Path>, cancel: Arc<AtomicBool>) -> Result<Self> {
        init()?;
        if cancel.load(Ordering::Relaxed) {
            return Err(MediaError::Cancelled);
        }
        let bytes = Arc::new(AtomicU64::new(0));
        let budget = Arc::new(AtomicU64::new(32 << 20));
        let io = ff::format::context::StreamIo::from_read_seek(Input {
            file: File::open(path)?,
            bytes: bytes.clone(),
            budget: budget.clone(),
        })
        .map_err(error("allocate input I/O"))?;
        let token = cancel.clone();
        let mut options = ff::Dictionary::new();
        options.set("max_streams", "256");
        options.set("probesize", "33554432");
        options.set("analyzeduration", "5000000");
        options.set("max_index_size", "8388608");
        let input =
            ff::format::input_from_stream_with_interrupt(io, None, Some(options), move || {
                token.load(Ordering::Relaxed)
            })
            .map_err(|source| {
                if cancel.load(Ordering::Relaxed) {
                    MediaError::Cancelled
                } else {
                    error("probe input")(source)
                }
            })?;
        budget.store(u64::MAX, Ordering::Relaxed);
        if input.nb_streams() > 256 {
            return Err(MediaError::TooLarge("stream count"));
        }
        let mut streams = Vec::new();
        for stream in input.streams() {
            let parameters = stream.parameters();
            let time_base = TimeBase::from_ff(stream.time_base())?;
            let (video, audio) = ffi::parameter_info(&parameters)?;
            let rate = stream.avg_frame_rate();
            streams.push(StreamInfo {
                index: stream.index(),
                kind: match parameters.medium() {
                    ff::media::Type::Video => StreamKind::Video,
                    ff::media::Type::Audio => StreamKind::Audio,
                    _ => StreamKind::Other,
                },
                codec: parameters.id().name().to_owned(),
                time_base,
                duration: if stream.duration() > 0 {
                    Some(time_base.ticks(stream.duration())?)
                } else {
                    None
                },
                start: if stream.start_time() == i64::MIN {
                    None
                } else {
                    Some(time_base.ticks(stream.start_time())?)
                },
                frame_rate: FrameRate::new(
                    i64::from(rate.numerator()),
                    i64::from(rate.denominator()),
                )
                .ok(),
                frame_count: u64::try_from(stream.frames()).ok().filter(|n| *n > 0),
                variable_frame_rate: None,
                video,
                audio,
                parameters: CodecParameters(parameters.clone()),
            });
        }
        let duration = if input.duration() > 0 {
            Some(Tick::from_units(
                input.duration(),
                1_000_000,
                Rounding::Nearest,
            )?)
        } else {
            None
        };
        let probe = Probe {
            format: input.format().name().to_owned(),
            duration,
            streams,
            bytes_read: bytes.load(Ordering::Relaxed),
        };
        Ok(Self {
            input,
            probe,
            cancel,
            bytes,
        })
    }
    pub fn probe(&self) -> &Probe {
        &self.probe
    }
    pub fn bytes_read(&self) -> u64 {
        self.bytes.load(Ordering::Relaxed)
    }
    pub fn next_packet(&mut self) -> Result<Option<Packet>> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(MediaError::Cancelled);
        }
        let mut inner = ff::Packet::empty();
        match inner.read(&mut self.input) {
            Ok(()) => {}
            Err(ff::Error::Eof) => return Ok(None),
            Err(source) => {
                return Err(if self.cancel.load(Ordering::Relaxed) {
                    MediaError::Cancelled
                } else {
                    error("read packet")(source)
                });
            }
        }
        if inner.size() > MAX_PACKET {
            return Err(MediaError::TooLarge("packet bytes"));
        }
        let stream = self
            .probe
            .streams
            .get(inner.stream())
            .ok_or(MediaError::Invalid("packet stream index"))?;
        Ok(Some(Packet {
            inner,
            time_base: stream.time_base,
        }))
    }
    /// Seek backward to a keyframe at or before this presentation time. Reset any
    /// decoder and decode forward, comparing frame tick PTS for an exact cut.
    pub fn seek(&mut self, time: Tick) -> Result<()> {
        let micros = time.to_units(1_000_000, Rounding::Floor)?;
        self.input
            .seek(micros, ..micros)
            .map_err(error("seek to preceding keyframe"))
    }
}
pub fn probe(path: impl AsRef<Path>) -> Result<Probe> {
    Ok(PacketReader::open(path)?.probe)
}

/// Sequential video access with automatic hardware-to-software recovery. On a
/// hardware decode failure, seeks to the last emitted frame's preceding keyframe
/// and decodes forward in software, suppressing already emitted presentation PTS.
/// Owns one bounded frame queue; it does not own an engine job or retry policy.
pub struct VideoReader {
    packets: PacketReader,
    decoder: crate::VideoDecoder,
    stream: StreamInfo,
    last_pts: Option<Tick>,
    minimum: Option<Tick>,
    restart: Tick,
    recovering: bool,
    software: bool,
    finished: bool,
}
impl VideoReader {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_with_cancel(path, Arc::new(AtomicBool::new(false)))
    }
    /// The shared token interrupts opening, packet reads and frame delivery,
    /// including buffered frames and software recovery. Cancellation is terminal.
    pub fn open_with_cancel(path: impl AsRef<Path>, cancel: Arc<AtomicBool>) -> Result<Self> {
        let packets = PacketReader::open_with_cancel(path, cancel)?;
        let stream = packets
            .probe()
            .streams
            .iter()
            .find(|s| s.kind == StreamKind::Video)
            .ok_or(MediaError::Unsupported("no video stream"))?
            .clone();
        let decoder = crate::VideoDecoder::new(&stream, crate::DecodePreference::Hardware)?;
        Ok(Self {
            packets,
            decoder,
            stream,
            last_pts: None,
            minimum: None,
            restart: Tick::ZERO,
            recovering: false,
            software: false,
            finished: false,
        })
    }
    pub fn stream(&self) -> &StreamInfo {
        &self.stream
    }
    pub fn seek(&mut self, target: Tick) -> Result<()> {
        self.packets.seek(target)?;
        self.decoder.reset();
        self.minimum = Some(target);
        self.restart = target;
        self.last_pts = None;
        self.recovering = false;
        self.finished = false;
        Ok(())
    }
    /// Decode from the preceding keyframe without suppressing earlier PTS. The
    /// compositor selects the frame whose presentation interval contains target.
    pub fn seek_keyframe(&mut self, target: Tick) -> Result<()> {
        self.seek(target)?;
        self.minimum = None;
        Ok(())
    }
    pub fn next_frame(&mut self) -> Result<Option<crate::VideoFrame>> {
        loop {
            if self.packets.cancel.load(Ordering::Relaxed) {
                return Err(MediaError::Cancelled);
            }
            let result = (|| {
                if let Some(frame) = self.decoder.pull_frame()? {
                    let pts = frame.pts()?.ok_or(MediaError::Unsupported(
                        "video presentation timestamp unavailable",
                    ))?;
                    if self.minimum.is_some_and(|min| pts < min)
                        || (self.recovering && self.last_pts.is_some_and(|last| pts <= last))
                    {
                        return Ok(None);
                    }
                    self.last_pts = Some(pts);
                    self.minimum = None;
                    self.recovering = false;
                    return Ok(Some(Some(frame)));
                }
                if self.finished {
                    return Ok(Some(None));
                }
                match self.packets.next_packet()? {
                    Some(packet) if packet.stream() == self.stream.index => {
                        self.decoder.push_packet(&packet)?
                    }
                    Some(_) => {}
                    None => {
                        self.finished = true;
                        self.decoder.finish_input()?;
                    }
                }
                Ok(None)
            })();
            match result {
                Ok(Some(frame)) => return Ok(frame),
                Ok(None) => {}
                Err(MediaError::Ffmpeg {
                    operation: "send video packet" | "receive video frame" | "flush video decoder",
                    ..
                }) if !self.software => {
                    let target = self.last_pts.or(self.minimum).unwrap_or(self.restart);
                    self.packets.seek(target)?;
                    self.decoder =
                        crate::VideoDecoder::new(&self.stream, crate::DecodePreference::Software)?;
                    self.software = true;
                    self.finished = false;
                    self.recovering = true;
                }
                Err(error) => return Err(error),
            }
        }
    }
}

#[cfg(test)]
#[path = "../tests/common/mod.rs"]
pub(crate) mod oracle_common;

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
pub(crate) mod acceleration_tests {
    use super::*;
    use crate::{DecodePreference, RgbaConverter, VideoDecoder, VideoFrame, decode::DecodeFault};

    pub(crate) fn fixture() -> Option<std::path::PathBuf> {
        super::oracle_common::generate(
            "accelerated recovery",
            "accelerated-recovery-bframes.mp4",
            &[
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=192x108:rate=24:duration=2",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:sample_rate=48000:duration=2",
                "-c:v",
                "libx264",
                "-bf",
                "3",
                "-g",
                "12",
                "-pix_fmt",
                "yuv420p",
                "-color_primaries",
                "bt709",
                "-color_trc",
                "bt709",
                "-colorspace",
                "bt709",
                "-color_range",
                "tv",
                "-chroma_sample_location",
                "left",
                "-c:a",
                "aac",
            ],
        )
    }

    fn software(path: &Path) -> VideoReader {
        let mut reader = VideoReader::open(path).unwrap();
        reader.decoder = VideoDecoder::new(&reader.stream, DecodePreference::Software).unwrap();
        reader.software = true;
        reader
    }

    fn fingerprint(frame: &VideoFrame) -> (Tick, crate::ColorInfo, Vec<u8>) {
        let mut converter = RgbaConverter::new().unwrap();
        let rgba = converter.convert(frame).unwrap();
        let stride = rgba.stride(0).unwrap();
        let pixels = rgba
            .plane(0)
            .unwrap()
            .chunks(stride)
            .take(frame.height() as usize)
            .flat_map(|row| &row[..frame.width() as usize * 4])
            .copied()
            .collect();
        (frame.pts().unwrap().unwrap(), frame.color(), pixels)
    }

    fn frames(reader: &mut VideoReader) -> Vec<(Tick, crate::ColorInfo, Vec<u8>)> {
        let mut output = Vec::new();
        while let Some(frame) = reader.next_frame().unwrap() {
            assert_eq!(frame.bit_depth(), Some(8));
            output.push(fingerprint(&frame));
        }
        output
    }

    #[test]
    fn decoder_loss_reseeks_without_dropping_or_repeating_presentation_frames() {
        let Some(path) = fixture() else { return };
        let expected = frames(&mut software(&path));
        assert_eq!(expected.len(), 48);
        for stage in [DecodeFault::Send, DecodeFault::Receive, DecodeFault::Drain] {
            let positions: &[usize] = if stage == DecodeFault::Receive {
                &[0, 1, 11, 12, 13, 46, 48]
            } else {
                &[0, 1, 11, 12, 13]
            };
            for &emitted in positions {
                let mut reader = VideoReader::open(&path).unwrap();
                let mut actual = Vec::new();
                for _ in 0..emitted {
                    actual.push(fingerprint(&reader.next_frame().unwrap().unwrap()));
                }
                reader.decoder.fault = Some(stage);
                actual.extend(frames(&mut reader));
                assert!(
                    reader.software,
                    "injected decoder failure must exercise recovery"
                );
                assert_eq!(actual.len(), expected.len());
                let mut maximum = 0;
                for (actual, expected) in actual.iter().zip(&expected) {
                    assert_eq!(actual.0, expected.0);
                    assert_eq!(actual.1, expected.1);
                    for (a, b) in actual.2.iter().zip(&expected.2) {
                        maximum = maximum.max(a.abs_diff(*b));
                    }
                }
                assert!(
                    maximum <= 4,
                    "hardware/software RGBA channel difference {maximum}"
                );
            }
        }
    }

    #[test]
    fn decoder_loss_preserves_seek_minimum_and_software_errors_are_terminal() {
        let Some(path) = fixture() else { return };
        let expected = frames(&mut software(&path));
        for target in [0, 1, 12, 47] {
            let pts = expected[target].0;
            let mut reader = VideoReader::open(&path).unwrap();
            reader.seek(pts).unwrap();
            reader.decoder.fault = Some(DecodeFault::Receive);
            let actual = frames(&mut reader);
            assert_eq!(actual.len(), expected.len() - target);
            assert!(
                actual == expected[target..],
                "recovery changed exact seek output"
            );
            reader.seek(pts).unwrap();
            reader.decoder.fault = Some(DecodeFault::Receive);
            assert!(matches!(
                reader.next_frame(),
                Err(MediaError::Ffmpeg { .. })
            ));
        }
    }

    #[test]
    fn recovery_follows_reverse_keyframe_seeks_and_in_between_frame_targets() {
        let Some(path) = fixture() else { return };
        for index in [46, 13, 1, 0] {
            let mut baseline = software(&path);
            let target = Tick::from_units(index, 24, Rounding::Nearest).unwrap();
            baseline.seek_keyframe(target).unwrap();
            let expected = frames(&mut baseline);
            let mut reader = VideoReader::open(&path).unwrap();
            reader.seek_keyframe(target).unwrap();
            reader.decoder.fault = Some(DecodeFault::Receive);
            let actual = frames(&mut reader);
            assert_eq!(actual.len(), expected.len());
            assert!(actual == expected, "recovery changed keyframe seek output");
            reader.seek(Tick(target.0 + 1)).unwrap();
            let actual = frames(&mut reader);
            assert!(actual.first().unwrap().0 > target);
        }
    }

    #[test]
    fn missing_presentation_timestamps_keep_the_typed_error_without_recovery() {
        let Some(path) = super::oracle_common::generate(
            "missing video timestamps",
            "accelerated-no-pts.h264",
            &[
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=64x64:rate=24:duration=1",
                "-c:v",
                "libx264",
                "-f",
                "h264",
            ],
        ) else {
            return;
        };
        let mut reader = VideoReader::open(path).unwrap();
        assert!(matches!(
            reader.next_frame(),
            Err(MediaError::Unsupported(
                "video presentation timestamp unavailable"
            ))
        ));
        assert!(!reader.software);
    }

    #[test]
    fn retained_frame_converts_for_the_compositor_after_its_reader_drops() {
        let Some(path) = fixture() else { return };
        let mut reader = VideoReader::open(path).unwrap();
        let first = reader.next_frame().unwrap().unwrap();
        let mut expected = fingerprint(&first);
        let pts = Tick(999);
        let retained = first.with_pts(pts).unwrap();
        assert_eq!(retained.is_hardware(), first.is_hardware());
        drop(first);
        drop(reader);
        expected.0 = pts;
        assert!(
            fingerprint(&retained) == expected,
            "retained frame pixels or metadata changed"
        );
    }

    #[test]
    fn cancellation_refuses_buffered_frames_and_does_not_restart_decoder() {
        let Some(path) = fixture() else { return };
        let cancel = Arc::new(AtomicBool::new(false));
        let mut reader = VideoReader::open_with_cancel(&path, cancel.clone()).unwrap();
        assert!(reader.next_frame().unwrap().is_some());
        cancel.store(true, Ordering::Relaxed);
        assert!(matches!(reader.next_frame(), Err(MediaError::Cancelled)));
        assert!(!reader.software);
        assert!(matches!(
            VideoReader::open_with_cancel(&path, cancel),
            Err(MediaError::Cancelled)
        ));
    }
}

#[cfg(all(test, target_os = "macos"))]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod recovery_benchmark {
    use super::*;
    use crate::{Container, Muxer, VideoEncoder, VideoEncoderSettings, decode::DecodeFault};
    use std::time::{Duration, Instant};

    #[test]
    #[ignore = "manual 60-second hardware recovery throughput and quality measurement"]
    fn transcode_60_seconds_with_optional_decoder_loss() {
        let input = std::env::var_os("VIBECUT_ACCEL_BENCH_INPUT").expect("benchmark input");
        let output = std::env::var_os("VIBECUT_ACCEL_BENCH_OUTPUT").expect("benchmark output");
        let loss = std::env::var_os("VIBECUT_ACCEL_BENCH_LOSS").is_some();
        let start = Instant::now();
        let mut reader = VideoReader::open(input).unwrap();
        let mut encoder: Option<VideoEncoder> = None;
        let mut mux = Muxer::create(output, Container::Mp4).unwrap();
        let mut decode_time = Duration::ZERO;
        let mut encode_time = Duration::ZERO;
        let mut count = 0;
        let mut hardware = 0;
        loop {
            if loss && count == 900 {
                reader.decoder.fault = Some(DecodeFault::Receive);
            }
            let mark = Instant::now();
            let frame = reader.next_frame().unwrap();
            decode_time += mark.elapsed();
            let Some(frame) = frame else { break };
            assert_eq!(
                frame.pts().unwrap().unwrap(),
                Tick::from_units(count, 30, Rounding::Nearest).unwrap()
            );
            assert_eq!(frame.bit_depth(), Some(8));
            hardware += usize::from(frame.is_hardware());
            if encoder.is_none() {
                let settings = VideoEncoderSettings::h264(
                    frame.width(),
                    frame.height(),
                    FrameRate::new(30, 1).unwrap(),
                    frame.color(),
                );
                let enc = VideoEncoder::new(settings, &frame).unwrap();
                mux.add_parameters(&enc.parameters(), enc.time_base())
                    .unwrap();
                encoder = Some(enc);
            }
            let mark = Instant::now();
            let packets = encoder.as_mut().unwrap().encode(&frame).unwrap();
            encode_time += mark.elapsed();
            for packet in packets {
                mux.write_packet(0, &packet).unwrap();
            }
            count += 1;
        }
        let mark = Instant::now();
        let tail = encoder.as_mut().unwrap().flush().unwrap();
        encode_time += mark.elapsed();
        for packet in tail {
            mux.write_packet(0, &packet).unwrap();
        }
        mux.finish().unwrap();
        assert_eq!(count, 1800);
        assert_eq!(hardware, if loss { 900 } else { 1800 });
        eprintln!(
            "recovery_loss={loss} frames={count} hardware_frames={hardware} decode_seconds={:.6} decode_fps={:.3} encode_seconds={:.6} encode_fps={:.3} export_seconds={:.6}",
            decode_time.as_secs_f64(),
            count as f64 / decode_time.as_secs_f64(),
            encode_time.as_secs_f64(),
            count as f64 / encode_time.as_secs_f64(),
            start.elapsed().as_secs_f64()
        );
    }
}

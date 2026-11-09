//! Opens containers through bounded I/O and delivers their timed packets.

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
    pub(crate) cancel: Arc<AtomicBool>,
    bytes: Arc<AtomicU64>,
    budget: Arc<AtomicU64>,
    pub(crate) recovered_thumbnail: Option<crate::Thumbnail>,
}
impl PacketReader {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_with_cancel(path, Arc::new(AtomicBool::new(false)))
    }
    /// Opens native paths through Rust File + seekable AVIO (32 KiB buffer), with
    /// at most 32 MiB read while probing. The token also interrupts FFmpeg probing.
    pub fn open_with_cancel(path: impl AsRef<Path>, cancel: Arc<AtomicBool>) -> Result<Self> {
        Self::open_with_read_limit(path, cancel, u64::MAX)
    }
    pub(crate) fn open_with_read_limit(
        path: impl AsRef<Path>,
        cancel: Arc<AtomicBool>,
        max_bytes: u64,
    ) -> Result<Self> {
        Self::open_prepared(path.as_ref(), cancel, max_bytes, None)
    }
    pub(crate) fn open_prepared(
        path: &Path,
        cancel: Arc<AtomicBool>,
        max_bytes: u64,
        known: Option<&crate::SourceProbe>,
    ) -> Result<Self> {
        init()?;
        if cancel.load(Ordering::Relaxed) {
            return Err(MediaError::Cancelled);
        }
        let bytes = Arc::new(AtomicU64::new(0));
        let budget = Arc::new(AtomicU64::new(max_bytes.min(32 << 20)));
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
                } else if budget.load(Ordering::Relaxed) == 0 {
                    MediaError::TooLarge("input read bytes")
                } else {
                    error("probe input")(source)
                }
            })?;
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
        let mut reader = Self {
            input,
            probe,
            cancel,
            bytes,
            budget,
            recovered_thumbnail: None,
        };
        if let Some(known) = known {
            for stream in &mut reader.probe.streams {
                if stream.video.is_none() {
                    stream.video = known
                        .streams
                        .iter()
                        .find(|s| s.index == stream.index && s.codec == stream.codec)
                        .and_then(|s| s.video.clone());
                }
            }
        }
        reader.recover_video_dimensions()?;
        reader.budget.store(
            max_bytes.saturating_sub(reader.bytes_read()),
            Ordering::Relaxed,
        );
        Ok(reader)
    }
    /// Some image demuxers report coded dimensions only after the first frame.
    /// Decode exactly that frame, record the real geometry, and rewind the input
    /// so packet consumers still start at the beginning.
    fn recover_video_dimensions(&mut self) -> Result<()> {
        let missing: Vec<usize> = self
            .probe
            .streams
            .iter()
            .filter(|stream| stream.kind == StreamKind::Video && stream.video.is_none())
            .map(|stream| stream.index)
            .collect();
        if missing.is_empty() {
            return Ok(());
        }
        // Recovery owns the opening boundary too: even a prematurely restored
        // streaming window cannot grant it reads beyond the aggregate prefix.
        self.budget.store(
            self.budget
                .load(Ordering::Relaxed)
                .min((32_u64 << 20).saturating_sub(self.bytes_read())),
            Ordering::Relaxed,
        );
        let primary_video = self
            .probe
            .streams
            .iter()
            .find(|stream| stream.kind == StreamKind::Video)
            .map(|stream| stream.index);
        for index in missing {
            let stream = self
                .probe
                .streams
                .iter()
                .find(|stream| stream.index == index)
                .cloned()
                .ok_or(MediaError::Invalid("probe stream"))?;
            let mut decoder = crate::VideoDecoder::for_thumbnail(&stream)?;
            let mut geometry = None;
            for _ in 0..4096 {
                let Some(packet) = self.next_packet()? else {
                    break;
                };
                if packet.stream() != stream.index {
                    continue;
                }
                decoder.push_packet(&packet)?;
                if let Some(frame) = decoder.pull_frame()? {
                    // Consumers thumbnail the first video stream. Secondary
                    // geometry recovery must neither supply nor replace it.
                    if primary_video == Some(index) {
                        self.recovered_thumbnail = super::thumbnail::from_frame(
                            &frame,
                            Tick::ZERO,
                            crate::THUMBNAIL_MAX_EDGE,
                        )
                        .ok();
                    }
                    geometry = Some((
                        frame.width(),
                        frame.height(),
                        frame.color(),
                        frame.bit_depth(),
                    ));
                    break;
                }
            }
            let Some((width, height, color, bit_depth)) = geometry else {
                return Err(MediaError::Unsupported(
                    "video dimensions unavailable before first frame",
                ));
            };
            if width == 0 || height == 0 || u64::from(width) * u64::from(height) > 1 << 26 {
                return Err(MediaError::TooLarge("video dimensions"));
            }
            let stream = self
                .probe
                .streams
                .iter_mut()
                .find(|stream| stream.index == index)
                .ok_or(MediaError::Invalid("probe stream"))?;
            stream.video = Some(crate::VideoInfo {
                width,
                height,
                color,
                bit_depth,
            });
        }
        self.seek(Tick::ZERO)?;
        self.probe.bytes_read = self.bytes.load(Ordering::Relaxed);
        Ok(())
    }
    pub fn probe(&self) -> &Probe {
        &self.probe
    }
    pub fn bytes_read(&self) -> u64 {
        self.bytes.load(Ordering::Relaxed)
    }
    #[cfg(test)]
    pub(crate) fn remaining_read_budget(&self) -> u64 {
        self.budget.load(Ordering::Relaxed)
    }
    // Preparation checkpoints occur only between complete packets, before the
    // current read window is exhausted; native parser state stays intact.
    pub(crate) fn start_read_window(&mut self, max_bytes: u64) {
        self.budget.store(max_bytes, Ordering::Relaxed);
    }
    pub fn next_packet(&mut self) -> Result<Option<Packet>> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(MediaError::Cancelled);
        }
        let mut inner = ff::Packet::empty();
        match inner.read(&mut self.input) {
            Ok(()) => {}
            Err(ff::Error::Eof) => {
                return if self.budget.load(Ordering::Relaxed) == 0 {
                    Err(MediaError::TooLarge("input read bytes"))
                } else {
                    Ok(None)
                };
            }
            Err(source) => {
                return Err(if self.cancel.load(Ordering::Relaxed) {
                    MediaError::Cancelled
                } else if self.budget.load(Ordering::Relaxed) == 0 {
                    MediaError::TooLarge("input read bytes")
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod import_regressions {
    use super::*;
    #[test]
    fn recovered_thumbnail_belongs_to_the_first_video_stream() {
        let Some(path) = testkit::oracle::generate(
            &testkit::fixture::dir("media"),
            "multi-stream recovery thumbnail",
            "recovery-two-videos.mkv",
            &[
                "-f",
                "lavfi",
                "-i",
                "color=blue:size=64x48:rate=1:duration=2",
                "-f",
                "lavfi",
                "-i",
                "color=red:size=32x24:rate=1:duration=2",
                "-map",
                "0:v",
                "-map",
                "1:v",
                "-c:v",
                "mjpeg",
            ],
        ) else {
            return;
        };
        let expected = crate::thumbnail_at(
            &path,
            Tick::ZERO,
            crate::THUMBNAIL_MAX_EDGE,
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
        for first_missing in [false, true] {
            let mut reader = PacketReader::open(&path).unwrap();
            assert_eq!(reader.probe.streams.len(), 2);
            if first_missing {
                reader.probe.streams[0].video = None;
            }
            reader.probe.streams[1].video = None;
            reader.recover_video_dimensions().unwrap();
            assert!(
                reader
                    .probe
                    .streams
                    .iter()
                    .all(|stream| stream.video.is_some())
            );
            if first_missing {
                assert_eq!(reader.recovered_thumbnail.take().unwrap(), expected);
            } else {
                assert!(
                    reader.recovered_thumbnail.is_none(),
                    "a secondary stream cannot supply the primary thumbnail"
                );
            }
        }
    }

    #[test]
    fn jpeg_cold_import_delivers_one_frame_and_cached_probe_delivers_none() {
        let Some(path) = testkit::oracle::generate(
            &testkit::fixture::dir("media"),
            "image decode reuse",
            "import-reuse.jpg",
            &[
                "-f",
                "lavfi",
                "-i",
                "color=blue:size=640x480",
                "-frames:v",
                "1",
                "-c:v",
                "mjpeg",
            ],
        ) else {
            return;
        };
        crate::read::decode::FRAME_DELIVERIES.with(|count| count.set(0));
        let (cold, image) = crate::SourceAsset::import_prepared(
            crate::AssetId(1),
            path.clone(),
            Arc::new(AtomicBool::new(false)),
            |_| None,
            |_, _| {},
        )
        .unwrap();
        assert_eq!(
            (
                cold.metadata().unwrap().width,
                cold.metadata().unwrap().height
            ),
            (Some(640), Some(480))
        );
        let image = image.unwrap_or_else(|| {
            crate::thumbnail(
                &cold.asset,
                Tick::ZERO,
                crate::THUMBNAIL_MAX_EDGE,
                Arc::new(AtomicBool::new(false)),
            )
            .unwrap()
        });
        assert_eq!((image.width, image.height), (320, 240));
        assert_eq!(
            crate::read::decode::FRAME_DELIVERIES.with(|count| count.get()),
            1
        );
        let (_, image) = crate::SourceAsset::import_prepared(
            crate::AssetId(2),
            path.clone(),
            Arc::new(AtomicBool::new(false)),
            |hash| {
                assert_eq!(hash, cold.asset.content_hash());
                Some(cold.asset.probe().clone())
            },
            |_, _| {},
        )
        .unwrap();
        assert!(image.is_none());
        assert_eq!(
            crate::read::decode::FRAME_DELIVERIES.with(|count| count.get()),
            1,
            "warm metadata must not deliver another decoder frame"
        );
        // Exercise demuxers that omit geometry even when this build's JPEG
        // parser happened to report it. Recovery must hand off that same frame.
        let mut reader = PacketReader::open(&path).unwrap();
        reader.probe.streams[0].video = None;
        crate::read::decode::FRAME_DELIVERIES.with(|count| count.set(0));
        reader.recover_video_dimensions().unwrap();
        let image = reader.recovered_thumbnail.take().unwrap();
        assert_eq!((image.width, image.height), (320, 240));
        assert_eq!(reader.probe.streams[0].video.as_ref().unwrap().width, 640);
        assert_eq!(
            crate::read::decode::FRAME_DELIVERIES.with(|count| count.get()),
            1,
            "geometry recovery must return its already decoded thumbnail"
        );
    }

    #[test]
    fn geometry_recovery_without_deliverable_packets_keeps_its_finite_read_window() {
        let Some(path) = testkit::oracle::generate(
            &testkit::fixture::dir("media"),
            "recovery bounds",
            "recovery-bounds.mp4",
            &[
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=1280x720:rate=30:duration=2",
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
                "-crf",
                "0",
            ],
        ) else {
            return;
        };
        let mut reader = PacketReader::open(&path).unwrap();
        // No packet can match this geometry-less video stream. Recovery must
        // stop at its read allowance, rather than scanning the remainder.
        reader.probe.streams[0].index = usize::MAX;
        reader.probe.streams[0].video = None;
        reader.seek(Tick::ZERO).unwrap();
        // Simulate probe analysis having consumed nearly all its allowance,
        // followed by the hostile premature streaming-window reset from review.
        reader.bytes.store((32 << 20) - 1024, Ordering::Relaxed);
        reader.start_read_window(u64::MAX);
        let before = reader.bytes_read();
        let result = reader.recover_video_dimensions();
        assert!(matches!(
            result,
            Err(MediaError::TooLarge("input read bytes"))
        ));
        assert!(reader.bytes_read() - before <= 1024);
        let thumbnail =
            crate::VideoReader::open_thumbnail(&path, Arc::new(AtomicBool::new(false))).unwrap();
        assert_eq!(
            thumbnail.remaining_read_budget(),
            (64 << 20) - thumbnail.bytes_read(),
            "thumbnail decode must retain a finite total input allowance"
        );
    }

    #[test]
    fn thumbnail_decoder_refuses_geometry_above_its_native_allowance() {
        let Some(path) = crate::read::decode::recovery_tests::fixture() else {
            return;
        };
        let reader = PacketReader::open(path).unwrap();
        let mut stream = reader.probe.streams[0].clone();
        for (width, height, admitted) in [
            (2048, 1024, true),
            (2049, 1024, false),
            (u32::MAX, u32::MAX, false),
        ] {
            stream.video.as_mut().unwrap().width = width;
            stream.video.as_mut().unwrap().height = height;
            assert_eq!(
                crate::VideoDecoder::for_thumbnail(&stream).is_ok(),
                admitted
            );
        }
    }
}

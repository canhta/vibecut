//! Writes a low-resolution intra-frame stand-in for a source's video.

use crate::{
    Container, MediaError, Muxer, Result, VideoEncoder, VideoEncoderSettings, VideoFrame,
    VideoReader,
};
use std::{
    path::Path,
    sync::{Arc, atomic::AtomicBool, atomic::Ordering},
};
use time::{Rounding, Tick};

/// Longest proxy edge the app asks for. An engineering default, not a measured
/// reference value.
pub const PROXY_MAX_EDGE: u32 = 960;
/// Frames one proxy may hold; a bogus timestamp cannot request an endless file.
const MAX_FRAMES: u64 = 2_000_000;

/// Limits one proxy write; the output is refused past either bound.
#[derive(Clone, Copy, Debug)]
pub struct ProxyLimits {
    /// Longest edge of the proxy raster. The source is never enlarged.
    pub max_edge: u32,
    /// Encoded bytes the file may reach.
    pub max_bytes: u64,
}

/// What a finished proxy holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProxyReport {
    pub width: u32,
    pub height: u32,
    pub frames: u64,
    pub bytes: u64,
}

/// Even raster that fits `max_edge`, keeping the source aspect and never
/// exceeding the source size.
pub fn proxy_size(width: u32, height: u32, max_edge: u32) -> (u32, u32) {
    let longest = width.max(height).max(1);
    let scale = (f64::from(max_edge) / f64::from(longest)).min(1.0);
    let even = |edge: u32| ((f64::from(edge) * scale / 2.0).round() as u32).max(1) * 2;
    (even(width), even(height))
}

struct Output {
    encoder: VideoEncoder,
    mux: Muxer,
    track: usize,
    next: i64,
    frames: u64,
    bytes: u64,
    limit: u64,
}

impl Output {
    /// Repeat `frame` on every grid instant strictly before `before`. A source
    /// frame therefore serves the instants from its own timestamp up to its
    /// successor, the same choice the compositor makes when it reads a source.
    fn hold(&mut self, frame: &VideoFrame, before: Tick) -> Result<()> {
        loop {
            let at = self.encoder.time_base().ticks(self.next)?;
            // The encoder accepts a one-tick rounding difference on its grid.
            if at.0.saturating_add(1) >= before.0 {
                return Ok(());
            }
            if self.frames >= MAX_FRAMES {
                return Err(MediaError::TooLarge("proxy frames"));
            }
            self.send(&frame.with_pts(at)?)?;
            self.next += 1;
            self.frames += 1;
        }
    }

    fn send(&mut self, frame: &VideoFrame) -> Result<()> {
        for packet in self.encoder.encode(frame)? {
            self.write(&packet)?;
        }
        Ok(())
    }

    fn write(&mut self, packet: &crate::Packet) -> Result<()> {
        self.bytes = self.bytes.saturating_add(packet.inner.size() as u64);
        if self.bytes > self.limit {
            return Err(MediaError::TooLarge("proxy bytes"));
        }
        self.mux.write_packet(self.track, packet)
    }
}

/// Transcode the first video stream of `source` to `output` as constant-rate
/// intra-only H.264 at most `limits.max_edge` wide or tall. Audio is not
/// carried: preview audio keeps reading the source. Frames land on the source's
/// own rate grid at their source timestamps, so a proxy frame matches the
/// source frame at the same presentation time. The caller owns the temporary
/// path, cancellation, promotion and cleanup after an error.
pub fn write_proxy(
    source: &Path,
    output: &Path,
    limits: ProxyLimits,
    cancel: Arc<AtomicBool>,
    mut progress: impl FnMut(f32),
) -> Result<ProxyReport> {
    if limits.max_edge < 2 || limits.max_bytes == 0 {
        return Err(MediaError::Invalid("proxy limits"));
    }
    let mut reader = VideoReader::open_with_cancel(source, cancel.clone())?;
    let stream = reader.stream().clone();
    let info = stream
        .video
        .clone()
        .ok_or(MediaError::Unsupported("proxy needs a video stream"))?;
    let rate = stream
        .frame_rate
        .ok_or(MediaError::Unsupported("proxy needs a frame rate"))?;
    let step = rate.frame_duration()?;
    let (width, height) = proxy_size(info.width, info.height, limits.max_edge);
    let span = stream.duration.map(|d| d.0.max(1) as f32);
    let origin = stream.start.unwrap_or(Tick::ZERO);
    let mut out: Option<Output> = None;
    let mut current: Option<VideoFrame> = None;
    let mut last = Tick::ZERO;
    while let Some(frame) = reader.next_frame()? {
        if cancel.load(Ordering::Relaxed) {
            return Err(MediaError::Cancelled);
        }
        let pts = frame.pts()?.ok_or(MediaError::Unsupported(
            "video presentation timestamp unavailable",
        ))?;
        let frame = frame.download()?;
        if out.is_none() {
            let bits = (f64::from(width) * f64::from(height) * rate.num() as f64
                / rate.den() as f64
                * 1.5) as usize;
            let settings = VideoEncoderSettings {
                bit_rate: bits.clamp(2_000_000, 100_000_000),
                quality: 75,
                intra_only: true,
                ..VideoEncoderSettings::h264(width, height, rate, frame.color())
            };
            let encoder = VideoEncoder::new(settings, &frame)?;
            let mut mux = Muxer::create(output, Container::Mp4)?;
            let track = mux.add_parameters(&encoder.parameters(), encoder.time_base())?;
            let next = encoder.time_base().units(pts, Rounding::Nearest)?;
            out = Some(Output {
                encoder,
                mux,
                track,
                next,
                frames: 0,
                bytes: 0,
                limit: limits.max_bytes,
            });
        }
        if let (Some(out), Some(held)) = (out.as_mut(), current.as_ref()) {
            out.hold(held, pts)?;
        }
        if let Some(span) = span {
            progress(((pts.0 - origin.0) as f32 / span).clamp(0.0, 1.0));
        }
        last = pts;
        current = Some(frame);
    }
    let (Some(mut out), Some(held)) = (out, current) else {
        return Err(MediaError::Unsupported("proxy source has no video frames"));
    };
    out.hold(
        &held,
        last.checked_add(step)
            .ok_or(MediaError::Invalid("proxy end time"))?,
    )?;
    for packet in out.encoder.flush()? {
        out.write(&packet)?;
    }
    let report = ProxyReport {
        width,
        height,
        frames: out.frames,
        bytes: out.bytes,
    };
    out.mux.finish()?;
    progress(1.0);
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::proxy_size;

    #[test]
    fn proxy_size_keeps_aspect_stays_even_and_does_not_enlarge() {
        for (source, edge, expected) in [
            ((1920, 1080), 960, (960, 540)),
            ((1080, 1920), 960, (540, 960)),
            ((128, 96), 960, (128, 96)),
            ((1001, 563), 500, (500, 282)),
        ] {
            let (width, height) = proxy_size(source.0, source.1, edge);
            assert_eq!((width, height), expected, "{source:?} at {edge}");
            assert!(width % 2 == 0 && height % 2 == 0);
        }
    }
}

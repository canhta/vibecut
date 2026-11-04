//! Pure Rust MP4/MOV containers: probe, streamed packet reading and H.264/AAC muxing.
//!
//! [`probe`] reads only the top-level box headers and the `moov` box, so opening a
//! multi-gigabyte file costs metadata, not media. [`PacketReader`] then reads one
//! sample at a time with bounded buffers. The muxer writes a progressive MP4 or MOV
//! with an `avcC` H.264 track, an `esds` AAC track and an edit list for encoder
//! priming.
//!
//! Times leave the crate as [`tick`](time::Tick)s at
//! [`time::TICKS_PER_SECOND`], converted from each track's media timescale. Edit
//! lists stay in their container units because they map source to presentation
//! coordinates; a caller applies them.

mod bitstream;
mod mp4;

pub use mp4::codec::{AacConfig, AvcConfig};
pub use mp4::demux::{Packet, PacketReader, probe};
pub use mp4::mux::{AudioTrackConfig, Brand, MuxOptions, Muxer, VideoTrackConfig, WritePacket};
pub use mp4::{AudioInfo, Edit, FourCc, Probe, StreamInfo, StreamKind, VideoInfo};

use std::io;

/// Failure from probing, reading or writing a container.
#[derive(Debug, thiserror::Error)]
pub enum MediaError {
    /// Reading or writing the underlying file failed.
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    /// A box or structure ended before all required fields were read.
    #[error("truncated data while reading {0}")]
    Truncated(&'static str),
    /// Structurally invalid data.
    #[error("invalid data: {0}")]
    Invalid(String),
    /// Valid but unsupported input or capability.
    #[error("unsupported: {0}")]
    Unsupported(String),
    /// A count or size exceeded the limit for the file.
    #[error("size limit exceeded: {0}")]
    TooLarge(&'static str),
    /// The file has no `moov` box.
    #[error("no moov box found")]
    NoMoov,
    /// A track index was out of range.
    #[error("track index {0} out of range")]
    NoSuchTrack(usize),
    /// A sample index was out of range.
    #[error("sample index {0} out of range")]
    NoSuchSample(usize),
    /// The muxer rejected its configuration or a packet.
    #[error("muxer: {0}")]
    Mux(String),
    /// A container time could not be converted to ticks.
    #[error(transparent)]
    Time(#[from] time::TimeError),
}

/// Result alias for container operations.
pub type Result<T> = std::result::Result<T, MediaError>;

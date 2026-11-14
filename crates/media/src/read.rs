//! Reads containers, decodes their packets and renders bounded thumbnails.

mod decode;
mod demux;
mod thumbnail;

#[cfg(test)]
pub(crate) use decode::recovery_tests;
pub use decode::{
    AudioDecoder, AudioFrame, DecodePreference, RgbaConverter, VideoDecoder, VideoFrame,
    VideoReader,
};
pub use demux::{PacketReader, probe};
pub(crate) use thumbnail::MAX_SOURCE_PIXELS;
pub use thumbnail::{THUMBNAIL_MAX_EDGE, Thumbnail, thumbnail, thumbnails_at};

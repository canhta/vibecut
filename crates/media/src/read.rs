//! Reads containers, decodes their packets and renders bounded thumbnails.

mod decode;
mod demux;
mod thumbnail;

pub use decode::{
    AudioDecoder, AudioFrame, DecodePreference, RgbaConverter, VideoDecoder, VideoFrame,
    VideoReader,
};
pub use demux::{PacketReader, probe};
pub(crate) use thumbnail::MAX_SOURCE_PIXELS;
pub use thumbnail::{
    THUMBNAIL_MAX_EDGE, Thumbnail, thumbnail, thumbnail_at, thumbnail_at_with_read_bytes,
    thumbnails_at,
};

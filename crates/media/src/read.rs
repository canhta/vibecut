//! Reads containers, decodes their packets and renders bounded thumbnails.

mod decode;
mod demux;
mod thumbnail;

pub use decode::{
    AudioDecoder, AudioFrame, DecodePreference, RgbaConverter, VideoDecoder, VideoFrame,
    VideoReader, decoder_threads, video_decode_bytes,
};
pub use demux::{PROBE_BYTES, PROBE_MEMORY_BYTES, PacketReader, probe};
pub(crate) use thumbnail::MAX_SOURCE_PIXELS;
pub use thumbnail::{THUMBNAIL_MAX_EDGE, Thumbnail, thumbnail, thumbnails_at};

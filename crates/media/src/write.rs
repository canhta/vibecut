//! Encodes frames and writes containers for one export.

mod encode;
mod gif;
mod mux;
mod quantize;

pub use encode::{
    AudioEncoder, AudioEncoderSettings, EncodePreference, H264Profile, VideoEncoder,
    VideoEncoderSettings, export_container_available, h264_encoder_available,
};
pub use gif::{
    GIF_MAX_COLORS, GIF_MAX_EDGE, GIF_MAX_FPS, GIF_MAX_FRAMES, GIF_MAX_PIXELS, GIF_MAX_SECONDS,
    GIF_MIN_COLORS, GifDither, GifEncoder, GifEncoderSettings, GifLimit, GifOptions,
};
pub use mux::{Container, MuxSink, Muxer};

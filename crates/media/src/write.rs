//! Encodes frames and writes containers for exports and proxies.

mod audio_file;
mod encode;
mod gif;
mod mux;
mod quantize;
mod proxy;

pub use audio_file::{AUDIO_FILE_BLOCK, AudioFileWriter};
pub use encode::{
    AudioEncoder, AudioEncoderSettings, EncodePreference, H264Profile, VideoEncoder,
    VideoEncoderSettings, audio_export_container_available, export_container_available,
    h264_encoder_available,
};
pub use gif::{
    GIF_MAX_COLORS, GIF_MAX_EDGE, GIF_MAX_FPS, GIF_MAX_FRAMES, GIF_MAX_PIXELS, GIF_MAX_SECONDS,
    GIF_MIN_COLORS, GifDither, GifEncoder, GifEncoderSettings, GifLimit, GifOptions,
};
pub use mux::{Container, MuxSink, Muxer};
pub use proxy::{PROXY_MAX_EDGE, ProxyLimits, ProxyReport, proxy_size, write_proxy};

//! Encodes frames and writes containers for exports and proxies.

mod encode;
mod mux;
mod proxy;

pub use encode::{
    AudioEncoder, AudioEncoderSettings, EncodePreference, H264Profile, VideoEncoder,
    VideoEncoderSettings, export_container_available, h264_encoder_available,
};
pub use mux::{Container, MuxSink, Muxer};
pub use proxy::{PROXY_MAX_EDGE, ProxyLimits, ProxyReport, proxy_size, write_proxy};

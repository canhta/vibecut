//! Encodes frames and writes containers for one export.

mod audio_file;
mod encode;
mod mux;

pub use audio_file::{AUDIO_FILE_BLOCK, AudioFileWriter};
pub use encode::{
    AudioEncoder, AudioEncoderSettings, EncodePreference, H264Profile, VideoEncoder,
    VideoEncoderSettings, export_container_available, h264_encoder_available,
};
pub use mux::{Container, MuxSink, Muxer};

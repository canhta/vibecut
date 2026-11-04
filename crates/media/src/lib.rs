//! Headless streaming media I/O through one dynamically linked LGPL FFmpeg backend.
//! FFmpeg owns codec parameters, demux presentation timing, priming and reordering.

mod asset;
mod audio_extract;
mod decode;
mod encode;
#[allow(unsafe_code)]
mod ffi;
mod model;
mod mux;
mod read;
mod resample;

pub use asset::{
    AssetAvailability, AssetData, AssetId, ContentHash, SourceAsset, SourceProbe, SourceStream,
};
pub use audio_extract::{
    AudioChunk, SPEECH_SAMPLE_RATE, SpeechExtractSettings, extract_speech_audio,
    extract_speech_audio_with, extract_speech_audio_with_cancel,
};
pub use decode::{AudioDecoder, AudioFrame, DecodePreference, VideoDecoder, VideoFrame};
pub use encode::{AudioEncoder, AudioEncoderSettings, VideoEncoder, VideoEncoderSettings};
pub use ffmpeg_next::format::Pixel as PixelFormat;
pub use model::{
    AudioInfo, CodecParameters, ColorInfo, Packet, Probe, StreamInfo, StreamKind, TimeBase,
    VideoInfo,
};
pub use mux::{Container, Muxer};
pub use read::{PacketReader, VideoReader, probe};
pub use resample::{AudioResampler, AudioSampleFormat};

/// Typed failures from the media boundary. Provider paths and media text are not
/// included in error messages; the FFmpeg source retains the numeric error category.
#[derive(Debug, thiserror::Error)]
pub enum MediaError {
    #[error("media I/O failed")]
    Io(#[from] std::io::Error),
    #[error("invalid media input: {0}")]
    Invalid(&'static str),
    #[error("media input has no audio stream")]
    NoAudioStream,
    #[error("unsupported media capability: {0}")]
    Unsupported(&'static str),
    #[error("media limit exceeded: {0}")]
    TooLarge(&'static str),
    #[error("media operation cancelled")]
    Cancelled,
    #[error("{operation}: {source}")]
    Ffmpeg {
        operation: &'static str,
        #[source]
        source: ffmpeg_next::Error,
    },
    #[error(transparent)]
    Time(#[from] time::TimeError),
}

pub type Result<T> = std::result::Result<T, MediaError>;

fn error(operation: &'static str) -> impl FnOnce(ffmpeg_next::Error) -> MediaError {
    move |source| match source {
        ffmpeg_next::Error::DecoderNotFound => {
            MediaError::Unsupported("decoder unavailable in this FFmpeg build")
        }
        ffmpeg_next::Error::EncoderNotFound => {
            MediaError::Unsupported("encoder unavailable in this FFmpeg build")
        }
        _ => MediaError::Ffmpeg { operation, source },
    }
}

fn init() -> Result<()> {
    // FFmpeg's default logger may include filenames and user metadata.
    static INITIALIZED: std::sync::OnceLock<std::result::Result<(), ffmpeg_next::Error>> =
        std::sync::OnceLock::new();
    INITIALIZED
        .get_or_init(|| {
            ffi::allocation_limit();
            ffmpeg_next::log::set_level(ffmpeg_next::log::Level::Quiet);
            ffmpeg_next::init()
        })
        .as_ref()
        .map_err(|source| error("initialize FFmpeg")(*source))?;
    let config = ffmpeg_next::format::configuration();
    if config.contains("--enable-gpl") || config.contains("--enable-nonfree") {
        return Err(MediaError::Unsupported(
            "FFmpeg must be an LGPL-only build; run cargo xtask ffmpeg",
        ));
    }
    Ok(())
}

fn drained(error: ffmpeg_next::Error) -> bool {
    error == ffmpeg_next::Error::Eof
        || error
            == ffmpeg_next::Error::Other {
                errno: ffmpeg_next::error::EAGAIN,
            }
}

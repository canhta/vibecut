//! Headless streaming media I/O through one dynamically linked FFmpeg backend.
//! FFmpeg owns codec parameters, demux presentation timing, priming and reordering.

mod audio;
#[allow(unsafe_code)]
mod ffi;
mod read;
mod source;
mod vision;
mod write;

pub use audio::{
    AudioResampler, AudioSampleFormat, BucketPeak, PEAK_BUCKET_SAMPLES, PEAK_SUMMARY_MAX_BYTES,
    PeakSummary, SPEECH_SAMPLE_RATE, SpeechEnergyWindow, SpeechExtractSettings, SpeechWindow,
    analyze_speech_energy, extract_speech_windows_with_cancel, peak_summary,
};
pub use ffmpeg_next::format::Pixel as PixelFormat;
pub use read::{
    AudioDecoder, AudioFrame, DecodePreference, PROBE_BYTES, PROBE_MEMORY_BYTES, PacketReader,
    RgbaConverter, THUMBNAIL_MAX_EDGE, Thumbnail, VideoDecoder, VideoFrame, VideoReader,
    decoder_threads, probe, thumbnail, thumbnails_at, video_decode_bytes,
};
pub use source::{
    AssetAvailability, AssetData, AssetId, AudioInfo, CodecParameters, ColorInfo, ContentHash,
    Decodability, DisplayFrameRate, ImportedSource, MediaKind, MediaMetadata, Packet, Probe,
    ProbeCapabilities, SourceAsset, SourceProbe, SourceStream, StreamCapability, StreamInfo,
    StreamKind, TimeBase, UnsupportedReason, VideoInfo,
};
pub use vision::{
    FACE_CROP_SIZE, FaceBox, FaceCrop, FaceTensor, MFCC_COEFFICIENTS, MfccFrame, MfccProgress,
    VISION_SAMPLE_RATE, VisionFrame, VisionPrepLimits, decode_frame_at, decode_shot_frames,
    mfcc_range_with_progress,
};
pub use write::{
    AUDIO_FILE_BLOCK, AudioEncoder, AudioEncoderSettings, AudioFileWriter, Container,
    EncodePreference, GIF_MAX_COLORS, GIF_MAX_EDGE, GIF_MAX_FPS, GIF_MAX_FRAMES, GIF_MAX_PIXELS,
    GIF_MAX_SECONDS, GIF_MIN_COLORS, GifDither, GifEncoder, GifEncoderSettings, GifLimit,
    GifOptions, H264Profile, MuxSink, Muxer, PROXY_MAX_EDGE, ProxyLimits, ProxyReport,
    VideoEncoder, VideoEncoderSettings, audio_export_container_available, default_encoder_threads,
    export_container_available, h264_encoder_available, proxy_size, video_encode_bytes,
    write_proxy,
};

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
    #[error("requested software H.264 encoder is unavailable")]
    EncoderUnavailable {
        #[source]
        source: Option<ffmpeg_next::Error>,
    },
    #[error("media limit exceeded: {0}")]
    TooLarge(&'static str),
    #[error(transparent)]
    Gif(#[from] GifLimit),
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
    let gpl = config.contains("--enable-gpl");
    // Linux admits the GPL build for libx264; every other host stays LGPL-only.
    let admitted = !config.contains("--enable-nonfree") && (!gpl || cfg!(target_os = "linux"));
    if !admitted {
        return Err(MediaError::Unsupported(
            "FFmpeg build has an unadmitted licence configuration; run cargo xtask ffmpeg",
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

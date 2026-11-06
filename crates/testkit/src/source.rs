//! Builds synthetic source assets and streams for media, engine and UI tests.
use media::{
    AssetData, AssetId, AudioInfo, ColorInfo, ContentHash, SourceAsset, SourceProbe, SourceStream,
    StreamKind, TimeBase, VideoInfo,
};
use std::path::PathBuf;
use time::{FrameRate, Tick};

/// A stream under construction; defaults describe a common H.264/AAC source.
#[derive(Clone, Debug)]
pub struct StreamBuilder {
    stream: SourceStream,
}

/// Starts a stream of `kind` with the common fixture defaults.
pub fn stream(kind: StreamKind) -> StreamBuilder {
    StreamBuilder {
        stream: SourceStream {
            index: 0,
            kind,
            codec: match kind {
                StreamKind::Video => "h264",
                StreamKind::Audio => "aac",
                StreamKind::Other => "data",
            }
            .into(),
            time_base: TimeBase::new(1, 1_000).expect("valid fixture time base"),
            duration: None,
            start: None,
            frame_rate: (kind == StreamKind::Video).then_some(FrameRate::FPS_30),
            frame_count: None,
            variable_frame_rate: None,
            video: (kind == StreamKind::Video).then_some(VideoInfo {
                width: 1920,
                height: 1080,
                color: ColorInfo::default(),
                bit_depth: Some(8),
            }),
            audio: (kind == StreamKind::Audio).then_some(AudioInfo {
                sample_rate: 48_000,
                channels: 2,
                initial_padding: 0,
                trailing_padding: 0,
            }),
        },
    }
}

/// Starts a 1920x1080 H.264 stream at 30 fps.
pub fn video() -> StreamBuilder {
    stream(StreamKind::Video)
}

/// Starts a 48 kHz stereo AAC stream.
pub fn audio() -> StreamBuilder {
    stream(StreamKind::Audio)
}

impl StreamBuilder {
    pub fn index(mut self, index: usize) -> Self {
        self.stream.index = index;
        self
    }

    pub fn codec(mut self, codec: impl Into<String>) -> Self {
        self.stream.codec = codec.into();
        self
    }

    pub fn time_base(mut self, time_base: TimeBase) -> Self {
        self.stream.time_base = time_base;
        self
    }

    pub fn duration(mut self, duration: Tick) -> Self {
        self.stream.duration = Some(duration);
        self
    }

    pub fn start(mut self, start: Tick) -> Self {
        self.stream.start = Some(start);
        self
    }

    pub fn frame_rate(mut self, frame_rate: FrameRate) -> Self {
        self.stream.frame_rate = Some(frame_rate);
        self
    }

    pub fn no_frame_rate(mut self) -> Self {
        self.stream.frame_rate = None;
        self
    }

    pub fn frame_count(mut self, frame_count: u64) -> Self {
        self.stream.frame_count = Some(frame_count);
        self
    }

    pub fn variable_frame_rate(mut self, variable: bool) -> Self {
        self.stream.variable_frame_rate = Some(variable);
        self
    }

    /// Sets the video dimensions, creating the video description when absent.
    pub fn size(mut self, width: u32, height: u32) -> Self {
        let video = self.stream.video.get_or_insert(VideoInfo {
            width,
            height,
            color: ColorInfo::default(),
            bit_depth: Some(8),
        });
        video.width = width;
        video.height = height;
        self
    }

    pub fn bit_depth(mut self, bit_depth: u8) -> Self {
        self.video_mut().bit_depth = Some(bit_depth);
        self
    }

    pub fn color(mut self, color: ColorInfo) -> Self {
        self.video_mut().color = color;
        self
    }

    pub fn video_info(mut self, video: VideoInfo) -> Self {
        self.stream.video = Some(video);
        self
    }

    pub fn sample_rate(mut self, sample_rate: u32) -> Self {
        self.audio_mut().sample_rate = sample_rate;
        self
    }

    pub fn channels(mut self, channels: u16) -> Self {
        self.audio_mut().channels = channels;
        self
    }

    pub fn padding(mut self, initial: u32, trailing: u32) -> Self {
        let audio = self.audio_mut();
        audio.initial_padding = initial;
        audio.trailing_padding = trailing;
        self
    }

    pub fn audio_info(mut self, audio: AudioInfo) -> Self {
        self.stream.audio = Some(audio);
        self
    }

    pub fn build(self) -> SourceStream {
        self.stream
    }

    fn video_mut(&mut self) -> &mut VideoInfo {
        self.stream.video.get_or_insert(VideoInfo {
            width: 1920,
            height: 1080,
            color: ColorInfo::default(),
            bit_depth: Some(8),
        })
    }

    fn audio_mut(&mut self) -> &mut AudioInfo {
        self.stream.audio.get_or_insert(AudioInfo {
            sample_rate: 48_000,
            channels: 2,
            initial_padding: 0,
            trailing_padding: 0,
        })
    }
}

/// A source asset under construction; defaults describe an empty `mp4` probe.
#[derive(Clone)]
pub struct AssetBuilder {
    data: AssetData,
}

/// Starts an asset with `id` and `path`; the path must be absolute.
pub fn asset(id: u64, path: impl Into<PathBuf>) -> AssetBuilder {
    AssetBuilder {
        data: AssetData {
            ephemeral: false,
            id: AssetId(id),
            path: path.into(),
            content_hash: ContentHash([0; 32]),
            probe: SourceProbe {
                format: "mp4".into(),
                duration: None,
                streams: Vec::new(),
            },
        },
    }
}

impl AssetBuilder {
    pub fn ephemeral(mut self, ephemeral: bool) -> Self {
        self.data.ephemeral = ephemeral;
        self
    }

    pub fn format(mut self, format: impl Into<String>) -> Self {
        self.data.probe.format = format.into();
        self
    }

    pub fn duration(mut self, duration: Tick) -> Self {
        self.data.probe.duration = Some(duration);
        self
    }

    pub fn content_hash(mut self, hash: ContentHash) -> Self {
        self.data.content_hash = hash;
        self
    }

    pub fn hash(mut self, hash: [u8; 32]) -> Self {
        self.data.content_hash = ContentHash(hash);
        self
    }

    pub fn stream(mut self, stream: SourceStream) -> Self {
        self.data.probe.streams.push(stream);
        self
    }

    pub fn streams(mut self, streams: impl IntoIterator<Item = SourceStream>) -> Self {
        self.data.probe.streams.extend(streams);
        self
    }

    pub fn build(self) -> SourceAsset {
        SourceAsset::try_from(self.data).expect("valid source fixture")
    }
}

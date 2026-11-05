#![allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    dead_code
)]
use captions::{Fingerprint, LanguageTag, Segment, Transcript};
use engine::{
    ByteBudget, Command, Session, clip_boundaries::ClipBoundarySettings, subtitles::Cancellation,
};
use engine::{ClipProposal, ClipRole, ClipSource, ClipSourceSegment};
use media::{
    AssetData, AssetId, ColorInfo, ContentHash, SourceAsset, SourceProbe, SourceStream, StreamKind,
    TimeBase, VideoInfo,
};
use project::{Project, ProjectId};
use std::sync::{Arc, Mutex};
use time::{FrameRate, SourceRange, TICKS_PER_SECOND, Tick};
use vibecut::{
    catalog::Preferences,
    playback::Sources,
    shorts::{Candidate, GenerationRequest, Generator, ReviewSource, Shorts, TaskEvents},
};
pub fn tick(n: i64) -> Tick {
    Tick(n * TICKS_PER_SECOND)
}
pub fn range(start: i64, end: i64) -> SourceRange {
    SourceRange::new(tick(start), tick(end)).unwrap()
}
pub fn source() -> ReviewSource {
    source_at(0)
}
pub fn source_at(offset: i64) -> ReviewSource {
    let asset = SourceAsset::try_from(AssetData {
        ephemeral: false,
        id: AssetId(1),
        path: std::env::temp_dir().join("vibecut-shorts-missing.mp4"),
        content_hash: ContentHash([0; 32]),
        probe: SourceProbe {
            format: "fixture".into(),
            duration: Some(tick(offset + 20)),
            streams: vec![SourceStream {
                index: 0,
                kind: StreamKind::Video,
                codec: "h264".into(),
                time_base: TimeBase::new(1, 30).unwrap(),
                duration: Some(tick(offset + 20)),
                start: Some(Tick::ZERO),
                frame_rate: Some(FrameRate::FPS_30),
                frame_count: Some(((offset + 20) * 30) as u64),
                variable_frame_rate: Some(false),
                video: Some(VideoInfo {
                    width: 320,
                    height: 180,
                    color: ColorInfo::default(),
                    bit_depth: Some(8),
                }),
                audio: None,
            }],
        },
    })
    .unwrap();
    let mut session = Session::new(
        Project::empty(ProjectId(1), "Interview".into()).unwrap(),
        100,
    )
    .unwrap();
    session.apply(Command::AddAsset { asset }).unwrap();
    let language = LanguageTag::parse("vi").unwrap();
    let transcript = Transcript {
        source_lang: language,
        fingerprint: Fingerprint {
            provider: "fixture".into(),
            model: "fixture".into(),
            settings: "1".into(),
        },
        segments: vec![
            Segment {
                id: "context".into(),
                range: range(offset + 1, offset + 3),
                text: "Một câu chuyện.".into(),
                words: Vec::new(),
                spans: None,
            },
            Segment {
                id: "hook".into(),
                range: range(offset + 10, offset + 12),
                text: "Điều đáng nhớ.".into(),
                words: Vec::new(),
                spans: None,
            },
        ],
    };
    let pieces = vec![
        ClipSourceSegment {
            range: range(offset + 10, offset + 13),
            role: ClipRole::Hook,
            source_order: 1,
            repair: None,
        },
        ClipSourceSegment {
            range: range(offset, offset + 4),
            role: ClipRole::Context,
            source_order: 0,
            repair: None,
        },
    ];
    let cues = captions::CueTrack {
        cues: transcript
            .segments
            .iter()
            .map(|segment| captions::Cue {
                id: segment.id.clone(),
                range: segment.range,
                source_lang: transcript.source_lang.clone(),
                texts: std::collections::BTreeMap::from([(
                    transcript.source_lang.clone(),
                    captions::CueText {
                        text: segment.text.clone(),
                        edited: false,
                        translation_ref: None,
                    },
                )]),
            })
            .collect(),
    };
    ReviewSource {
        project: session.project().as_ref().clone(),
        asset: AssetId(1),
        transcript,
        cues,
        energy: Vec::new(),
        boundaries: ClipBoundarySettings {
            source_range: range(0, offset + 20),
            frame_rate: FrameRate::FPS_30,
            tolerance: Tick(TICKS_PER_SECOND / 2),
            silence_rms: 0.01,
            min_duration: tick(1),
            max_duration: tick(20),
            min_segment_duration: Tick(TICKS_PER_SECOND / 2),
            max_segments: 4,
        },
        candidates: vec![
            Candidate {
                proposal: ClipProposal {
                    segments: Vec::new(),
                    title: "Điều đáng nhớ từ cuộc trò chuyện".into(),
                    hook_text: "Điều đáng nhớ.".into(),
                    reason: "A complete thought, led by the later hook.".into(),
                    score: 0.8,
                },
                segments: pieces.clone(),
                source: ClipSource::Heuristic,
                duration: tick(7),
                adjustments: Vec::new(),
            },
            Candidate {
                proposal: ClipProposal {
                    segments: Vec::new(),
                    title: "A second idea".into(),
                    hook_text: "".into(),
                    reason: "Another complete thought.".into(),
                    score: 0.6,
                },
                segments: vec![ClipSourceSegment {
                    source_order: 0,
                    ..pieces[0].clone()
                }],
                source: ClipSource::Llm,
                duration: tick(3),
                adjustments: Vec::new(),
            },
        ],
    }
}
pub struct Once(pub Mutex<Option<ReviewSource>>);
impl Generator for Once {
    fn run(
        &self,
        _: GenerationRequest,
        _: ByteBudget,
        _: &TaskEvents,
        _: &Cancellation,
    ) -> engine::Result<ReviewSource> {
        self.0
            .lock()
            .unwrap()
            .take()
            .ok_or(engine::EditError::Invalid("fixture already consumed"))
    }
}
pub struct Pixels;
impl Sources for Pixels {
    fn create(&self) -> Box<dyn render::SourceProvider> {
        Box::new(Pixels)
    }
}
impl render::SourceProvider for Pixels {
    fn status(
        &mut self,
        _: &SourceAsset,
        _: bool,
        _: &dyn render::ExecutionHooks,
    ) -> render::Result<Option<render::UnavailableReason>> {
        Ok(None)
    }
    fn video(
        &mut self,
        _: &SourceAsset,
        _: project::ItemId,
        at: Tick,
        hooks: &dyn render::ExecutionHooks,
    ) -> render::Result<Option<render::Image>> {
        let color = if at >= tick(10) {
            [0.08, 0.35, 0.16, 1.0]
        } else {
            [0.12, 0.18, 0.42, 1.0]
        };
        Ok(Some(render::Image::new(320, 180, color, hooks)?))
    }
    fn audio(
        &mut self,
        _: &SourceAsset,
        _: project::ItemId,
        _: i64,
        frames: usize,
        rate: u32,
        hooks: &dyn render::ExecutionHooks,
    ) -> render::Result<render::AudioBlock> {
        render::AudioBlock::new(rate, 2, frames, render::MemoryKind::Decoded, hooks)
    }
}
pub fn app() -> Shorts {
    let mut app = Shorts::with_generator(
        Preferences::default(),
        ByteBudget::new(512 << 20).unwrap(),
        Arc::new(Once(Mutex::new(Some(source())))),
        Arc::new(Pixels),
    )
    .unwrap();
    app.captions = false;
    app.pick_video(std::env::temp_dir().join("interview.mp4"))
        .unwrap();
    app
}
pub fn wait(app: &mut Shorts) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while app.busy() {
        app.tick();
        assert!(std::time::Instant::now() < deadline, "shorts timeout");
        std::thread::yield_now();
    }
    app.tick();
}
pub fn ready() -> Shorts {
    let mut a = app();
    a.generate().unwrap();
    wait(&mut a);
    assert!(a.error().is_none(), "{:?}", a.error());
    a
}

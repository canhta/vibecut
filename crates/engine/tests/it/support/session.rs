//! Session fixtures shared by the command-family tests.
#![allow(clippy::unwrap_used, reason = "validated test fixtures and assertions")]
use engine::{EditError, NewItem, Outcome, Session};
use media::{AssetId, ContentHash, SourceAsset};
use project::{
    Color, Item, ItemId, ItemParams, OverlayParams, Project, ProjectId, Track, TrackId, TrackKind,
};
use time::{SourceRange, Tick};

/// The wire-semantics entry for tests that assert string decoding and wire
/// validation: it is the control adapter's `run_wire`, not a `Session` method.
pub(crate) trait Wire {
    fn wire(&mut self, id: &str, params: serde_json::Value) -> Result<Outcome, EditError>;
}
impl Wire for Session {
    fn wire(&mut self, id: &str, params: serde_json::Value) -> Result<Outcome, EditError> {
        engine::control::run_wire(self, id, params)
    }
}

pub(crate) fn range(a: i64, b: i64) -> SourceRange {
    SourceRange::new(Tick(a), Tick(b)).unwrap()
}
pub(crate) fn content() -> ItemParams {
    ItemParams::Overlay(OverlayParams {
        color: Color([1, 2, 3, 255]),
    })
}
pub(crate) fn new_item(start: i64, duration: i64) -> NewItem {
    NewItem {
        animations: Default::default(),
        source_asset: None,
        source_range: range(100, 100 + duration),
        start: Tick(start),
        retime: Default::default(),
        content: content(),
    }
}
pub(crate) fn project() -> Project {
    let mut data = Project::empty(ProjectId(0), "Tiếng Việt".into())
        .unwrap()
        .into_data();
    for id in [1, 0] {
        data.scene_mut().timeline.insert_track(Track {
            id: TrackId(id + 1),
            name: "track".into(),
            kind: TrackKind::Video,
            muted: false,
            hidden: false,
            locked: false,
            items: Vec::new(),
        });
    }
    data.scene_mut()
        .timeline
        .overlays
        .first_mut()
        .unwrap()
        .items = vec![
        testkit::item::item(ItemId(10), content())
            .source(range(100, 140))
            .start(Tick(0))
            .build(),
        testkit::item::item(ItemId(11), content())
            .source(range(200, 240))
            .start(Tick(60))
            .build(),
    ];
    Project::new(data).unwrap()
}
pub(crate) fn session() -> Session {
    Session::new(project(), 1000).unwrap()
}
pub(crate) fn get(s: &Session, id: u64) -> &Item {
    get_project(s.project(), id)
}
pub(crate) fn get_project(project: &Project, id: u64) -> &Item {
    project
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|t| &t.items)
        .find(|i| i.id == ItemId(id))
        .unwrap()
}
pub(crate) fn get_id(s: &Session, id: ItemId) -> &Item {
    get(s, id.0)
}
pub(crate) fn get_project_id(project: &Project, id: ItemId) -> &Item {
    get_project(project, id.0)
}
// Pause removal: reviewed output-time spans cut every track and close their tails.

pub(crate) fn pause_audio_asset() -> SourceAsset {
    let duration = Tick(12 * time::TICKS_PER_SECOND);
    testkit::source::asset(1, std::env::temp_dir().join("pause-audio.wav"))
        .format("wav")
        .duration(duration)
        .content_hash(ContentHash::from_bytes(b"pause audio"))
        .stream(
            testkit::source::audio()
                .codec("pcm_f32le")
                .time_base(media::TimeBase::new(1, 48000).unwrap())
                .duration(duration)
                .build(),
        )
        .build()
}
pub(crate) fn pause_video_asset() -> SourceAsset {
    let duration = Tick(12 * time::TICKS_PER_SECOND);
    testkit::source::asset(2, std::env::temp_dir().join("pause-video.mp4"))
        .format("mp4")
        .duration(duration)
        .content_hash(ContentHash::from_bytes(b"pause video"))
        .stream(
            testkit::source::video()
                .time_base(media::TimeBase::new(1, 30000).unwrap())
                .duration(duration)
                .size(64, 36)
                .build(),
        )
        .build()
}
pub(crate) fn five_clip_pairs() -> Session {
    let edge = |seconds: i64| Tick(seconds * time::TICKS_PER_SECOND);
    let edges = [0, 2, 4, 6, 8, 12];
    let mut data = project().into_data();
    data.assets = vec![pause_audio_asset(), pause_video_asset()];
    data.scene_mut().timeline.overlays.clear();
    data.scene_mut().timeline.audio.clear();
    data.scene_mut().timeline.main.items = edges
        .windows(2)
        .enumerate()
        .map(|(index, pair)| {
            let start = pair.first().copied().unwrap_or_default();
            let end = pair.last().copied().unwrap_or_default();
            testkit::item::video(ItemId(index as u64 + 1))
                .asset(AssetId(2))
                .source(range(edge(start).0, edge(end).0))
                .start(edge(start))
                .build()
        })
        .collect();
    data.scene_mut().timeline.insert_track(Track {
        id: TrackId(5),
        name: "Audio".into(),
        kind: TrackKind::Audio,
        muted: false,
        hidden: false,
        locked: false,
        items: edges
            .windows(2)
            .enumerate()
            .map(|(index, pair)| {
                let start = pair.first().copied().unwrap_or_default();
                let end = pair.last().copied().unwrap_or_default();
                testkit::item::audio(ItemId(index as u64 + 11))
                    .asset(AssetId(1))
                    .source(range(edge(start).0, edge(end).0))
                    .start(edge(start))
                    .build()
            })
            .collect(),
    });
    Session::new(Project::new(data).unwrap(), 100).unwrap()
}
// --- Transcript word cuts -------------------------------------------------

/// Word-by-word measured proof of `text`: one native record per range, with bytes
/// on the space-separated words (surplus ranges cover the whole text). No ranges
/// leaves the cue unverified.
pub(crate) fn measured(
    language: &captions::LanguageTag,
    text: &str,
    words: &[SourceRange],
) -> captions::CueWordTiming {
    if words.is_empty() {
        return captions::CueWordTiming::Unverified;
    }
    let mut at = 0;
    let mut spans = Vec::new();
    for word in text.split(' ') {
        if !word.is_empty() {
            spans.push(at..at + word.len());
        }
        at += word.len() + 1;
    }
    captions::CueWordTiming::Words {
        language: language.clone(),
        text: text.into(),
        records: words
            .iter()
            .enumerate()
            .map(|(index, range)| captions::VerifiedCueWord {
                bytes: spans.get(index).cloned().unwrap_or(0..text.len()),
                source_range: *range,
                projection: captions::WordProjection::Visible(*range),
                provenance: captions::TimingProvenance::Native,
            })
            .collect(),
    }
}

pub(crate) fn word_cue(
    id: &str,
    start: i64,
    end: i64,
    words: &[(i64, i64)],
    edited: bool,
) -> captions::Cue {
    let language = captions::LanguageTag::parse("vi").unwrap();
    let text = ["Một", "hai", "ba", "bốn"]
        .get(..words.len())
        .unwrap_or_default()
        .join(" ");
    let ranges: Vec<_> = words.iter().map(|&(a, b)| range(a, b)).collect();
    captions::Cue {
        word_timing: measured(&language, &text, &ranges),
        id: id.into(),
        range: range(start, end),
        source_lang: language.clone(),
        texts: std::collections::BTreeMap::from([(
            language,
            captions::CueText {
                text: text.into(),
                edited,
                translation_ref: None,
            },
        )]),
        style: None,
    }
}
pub(crate) fn caption_words_item(
    id: u64,
    start: i64,
    source: (i64, i64),
    cues: Vec<captions::Cue>,
) -> Item {
    testkit::item::item(
        ItemId(id),
        ItemParams::Caption(project::CaptionParams {
            emphasis: Default::default(),
            word_highlight: false,
            preset: None,
            overrides: captions::CaptionOverrides::default(),
            cues: captions::CueTrack {
                styles: Vec::new(),
                cues,
            },
        }),
    )
    .asset(AssetId(7))
    .source(range(source.0, source.1))
    .start(Tick(start))
    .build()
}
pub(crate) fn clip_item(id: u64, start: i64, source: (i64, i64)) -> Item {
    testkit::item::item(ItemId(id), content())
        .source(range(source.0, source.1))
        .start(Tick(start))
        .build()
}
pub(crate) fn word_asset(data: &mut project::ProjectData, duration: i64) {
    data.assets.push(
        testkit::source::asset(7, std::env::temp_dir().join("transcript-words.mp4"))
            .format("mp4")
            .duration(Tick(duration))
            .content_hash(ContentHash::from_bytes(b"words"))
            .build(),
    );
}
pub(crate) fn assert_unchanged(s: &Session, bytes: &[u8], revision: &engine::control::Revision) {
    assert_eq!(project::encode(s.project(), false).unwrap(), bytes);
    assert_eq!(&s.revision(), revision);
    assert_eq!(s.history_len(), (0, 0));
}

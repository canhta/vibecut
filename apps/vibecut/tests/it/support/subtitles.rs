#![allow(clippy::unwrap_used)]
//! Shared synthetic captions for real editor tests and offscreen audit scenes.
use captions::{Cue, CueText, CueTrack, LanguageTag};
use engine::{ByteBudget, Command, NewItem, Session};
use project::{ItemParams, Project, ProjectId};
use std::{collections::BTreeMap, sync::Arc};
use time::{SourceRange, TICKS_PER_SECOND, Tick};
use vibecut::{
    AssetsTab, Editor,
    preferences::Preferences,
    services::playback::{Files, Playback},
};
pub fn lang(tag: &str) -> LanguageTag {
    LanguageTag::parse(tag).unwrap()
}
pub fn cues(source: &str) -> CueTrack {
    CueTrack {
        styles: Vec::new(),
        cues: (0..3)
            .map(|i| Cue {
                word_timing: captions::CueWordTiming::Unverified,
                id: format!("cue-{i}").into(),
                range: SourceRange::new(
                    Tick((1 + i * 3) * TICKS_PER_SECOND),
                    Tick((3 + i * 3) * TICKS_PER_SECOND),
                )
                .unwrap(),
                source_lang: lang(source),
                texts: BTreeMap::from([(
                    lang(source),
                    CueText {
                        text: match source {
                            "vi" => "Cha\u{0300}o ba\u{0323}n",
                            "en" => "Hello friend",
                            _ => "你好朋友",
                        }
                        .into(),
                        edited: false,
                        translation_ref: None,
                    },
                )]),
                style: None,
            })
            .collect(),
    }
}
pub fn verified_cues() -> CueTrack {
    let mut track = cues("vi");
    for cue in &mut track.cues {
        let text = cue.texts.get(&cue.source_lang).unwrap().text.clone();
        let split = text.as_str().find(' ').unwrap();
        let middle = cue
            .range
            .start()
            .checked_add(Tick(TICKS_PER_SECOND))
            .unwrap();
        cue.texts.insert(
            lang("en"),
            CueText {
                text: "Hello friend".into(),
                edited: false,
                translation_ref: Some("synthetic-fixture".into()),
            },
        );
        let left = SourceRange::new(cue.range.start(), middle).unwrap();
        let right = SourceRange::new(middle, cue.range.end()).unwrap();
        cue.word_timing = captions::CueWordTiming::Words {
            language: cue.source_lang.clone(),
            text: text.clone(),
            records: vec![
                captions::VerifiedCueWord {
                    bytes: 0..split,
                    source_range: left,
                    projection: captions::WordProjection::Visible(left),
                    provenance: captions::TimingProvenance::Native,
                },
                captions::VerifiedCueWord {
                    bytes: split + 1..text.as_str().len(),
                    source_range: right,
                    projection: captions::WordProjection::Visible(right),
                    provenance: captions::TimingProvenance::Forced,
                },
            ],
        };
    }
    track
}

pub fn insert(track: CueTrack) -> Command {
    Command::InsertItem {
        placement: engine::Placement::Auto,
        item: NewItem {
            retime: Default::default(),
            animations: Default::default(),
            source_asset: Some(media::AssetId(1)),
            source_range: SourceRange::new(Tick::ZERO, Tick(10 * TICKS_PER_SECOND)).unwrap(),
            start: Tick::ZERO,
            content: ItemParams::Caption(project::CaptionParams {
                emphasis: Default::default(),
                word_highlight: false,
                preset: None,
                overrides: captions::CaptionOverrides::default(),
                cues: track,
            }),
        },
    }
}
pub fn editor(source: &str) -> Editor {
    editor_with_cues(source, cues(source))
}
pub fn editor_with_cues(source: &str, track: CueTrack) -> Editor {
    let mut session =
        Session::new(Project::empty(ProjectId(9), "Captions".into()).unwrap(), 30).unwrap();
    session
        .apply(Command::AddAsset {
            asset: media::SourceAsset::try_from(media::AssetData {
                ephemeral: true,
                id: media::AssetId(1),
                path: std::env::temp_dir().join("caption-fixture.srt"),
                content_hash: media::ContentHash::from_bytes(b"fixture"),
                probe: media::SourceProbe {
                    format: "srt".into(),
                    duration: None,
                    streams: Vec::new(),
                },
            })
            .unwrap(),
        })
        .unwrap();
    session.apply(insert(track)).unwrap();
    let playback = Playback::new(
        &session,
        ByteBudget::new(256 << 20).unwrap(),
        Arc::new(Files),
    )
    .unwrap();
    let mut preferences = Preferences {
        assets_tab: AssetsTab::Captions,
        ..Default::default()
    };
    preferences.subtitles.source = source.into();
    let mut editor = Editor::new(session, playback, preferences);
    editor.prepare_subtitles().unwrap();
    editor
}

/// Two time-adjacent English cues, "Hello world" (1-3 s) and "good day" (3-5 s), each
/// with two measured words. `edited_first` protects the first cue's source text.
pub fn transcript_cues(edited_first: bool) -> CueTrack {
    CueTrack {
        styles: Vec::new(),
        cues: vec![
            verified_english_cue("cue-0", 1, "Hello world", 5, edited_first),
            verified_english_cue("cue-1", 3, "good day", 4, false),
        ],
    }
}

fn verified_english_cue(id: &str, start: i64, text: &str, split: usize, edited: bool) -> Cue {
    let source = lang("en");
    let at = |seconds: i64| Tick(seconds * TICKS_PER_SECOND);
    let left = SourceRange::new(at(start), at(start + 1)).unwrap();
    let right = SourceRange::new(at(start + 1), at(start + 2)).unwrap();
    Cue {
        word_timing: captions::CueWordTiming::Words {
            language: source.clone(),
            text: text.into(),
            records: vec![
                captions::VerifiedCueWord {
                    bytes: 0..split,
                    source_range: left,
                    projection: captions::WordProjection::Visible(left),
                    provenance: captions::TimingProvenance::Native,
                },
                captions::VerifiedCueWord {
                    bytes: split + 1..text.len(),
                    source_range: right,
                    projection: captions::WordProjection::Visible(right),
                    provenance: captions::TimingProvenance::Native,
                },
            ],
        },
        id: id.into(),
        range: SourceRange::new(at(start), at(start + 2)).unwrap(),
        source_lang: source.clone(),
        texts: BTreeMap::from([(
            source,
            CueText {
                text: text.into(),
                edited,
                translation_ref: None,
            },
        )]),
        style: None,
    }
}

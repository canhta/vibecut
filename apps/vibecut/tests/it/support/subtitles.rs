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

/// Three English cues of measured words, 1-3 s, 4-6 s and 7-9 s: the first and last open with a
/// repeated word, and a one second silence separates each pair.
pub fn review_cues() -> CueTrack {
    let mut track = cues("en");
    for (cue, text) in track.cues.iter_mut().zip(["go go now", "good day", "so so far"]) {
        let language = cue.source_lang.clone();
        cue.texts.insert(
            language.clone(),
            CueText {
                text: text.into(),
                edited: false,
                translation_ref: None,
            },
        );
        let words: Vec<&str> = text.split(' ').collect();
        let step = (cue.range.end().0 - cue.range.start().0) / words.len() as i64;
        let mut at = 0;
        let mut records = Vec::new();
        for (index, word) in words.iter().enumerate() {
            let start = Tick(cue.range.start().0 + step * index as i64);
            let range = SourceRange::new(start, Tick(start.0 + step)).unwrap();
            records.push(captions::VerifiedCueWord {
                bytes: at..at + word.len(),
                source_range: range,
                projection: captions::WordProjection::Visible(range),
                provenance: captions::TimingProvenance::Native,
            });
            at += word.len() + 1;
        }
        cue.word_timing = captions::CueWordTiming::Words {
            language,
            text: text.into(),
            records,
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

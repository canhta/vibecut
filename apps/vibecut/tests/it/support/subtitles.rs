#![allow(clippy::unwrap_used)]
//! Shared synthetic captions for real editor tests and offscreen audit scenes.
use captions::{Cue, CueText, CueTrack, LanguageTag};
use engine::{ByteBudget, Command, NewItem, Session};
use project::{ItemParams, Project, ProjectId};
use std::{collections::BTreeMap, sync::Arc};
use time::{SourceRange, TICKS_PER_SECOND, Tick};
use vibecut::{
    Editor,
    catalog::Preferences,
    panels::AssetsTab,
    playback::{Files, Playback},
};
pub fn lang(tag: &str) -> LanguageTag {
    LanguageTag::parse(tag).unwrap()
}
pub fn cues(source: &str) -> CueTrack {
    CueTrack {
        cues: (0..3)
            .map(|i| Cue {
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
pub fn insert(track: CueTrack) -> Command {
    Command::InsertItem {
        placement: engine::Placement::Auto,
        item: NewItem {
            retime: Default::default(),
            animations: Default::default(),
            source_asset: Some(media::AssetId(1)),
            source_range: SourceRange::new(Tick::ZERO, Tick(10 * TICKS_PER_SECOND)).unwrap(),
            start: Tick::ZERO,
            content: ItemParams::Caption(project::CaptionParams { cues: track }),
        },
    }
}
pub fn editor(source: &str) -> Editor {
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
    session.apply(insert(cues(source))).unwrap();
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

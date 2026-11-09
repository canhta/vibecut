//! Verifies the caption cue and word read model.
#![allow(clippy::unwrap_used, reason = "validated test fixtures and assertions")]
use captions::{Cue, CueText, CueTrack, LanguageTag, NfcText};
use engine::Session;
use media::{AssetId, ContentHash};
use project::{
    CaptionParams, Item, ItemId, ItemParams, Project, ProjectId, Track, TrackId, TrackKind,
};
use std::collections::BTreeMap;
use time::{SourceRange, Tick};

fn range(start: i64, end: i64) -> SourceRange {
    SourceRange::new(Tick(start), Tick(end)).unwrap()
}

fn language(tag: &str) -> LanguageTag {
    LanguageTag::parse(tag).unwrap()
}

fn cue(id: &str, start: i64, end: i64, text: &str, words: &[(i64, i64)]) -> Cue {
    let source = language("vi");
    let mut texts = BTreeMap::from([(
        source.clone(),
        CueText {
            text: text.into(),
            edited: false,
            translation_ref: None,
        },
    )]);
    texts.insert(
        language("en"),
        CueText {
            text: "translation".into(),
            edited: false,
            translation_ref: Some("translation/1".into()),
        },
    );
    Cue {
        id: id.into(),
        range: range(start, end),
        source_lang: source,
        texts,
        word_ranges: words.iter().map(|&(a, b)| range(a, b)).collect(),
        style: None,
    }
}

fn caption_item(id: u64, start: i64, source: (i64, i64), cues: Vec<Cue>) -> Item {
    Item {
        hidden: false,
        blend_mode: Default::default(),
        retime: Default::default(),
        masks: Vec::new(),
        effects: Vec::new(),
        animations: Default::default(),
        id: ItemId(id),
        source_asset: Some(AssetId(7)),
        source_range: range(source.0, source.1),
        start: Tick(start),
        content: ItemParams::Caption(CaptionParams {
            cues: CueTrack { cues },
        }),
    }
}

fn track(id: u64, locked: bool, items: Vec<Item>) -> Track {
    Track {
        id: TrackId(id),
        name: "Captions".into(),
        kind: TrackKind::Text,
        muted: false,
        hidden: false,
        locked,
        items,
    }
}

fn session(tracks: Vec<Track>) -> Session {
    let mut data = Project::empty(ProjectId(0), "Phụ đề".into())
        .unwrap()
        .into_data();
    data.assets.push(
        testkit::source::asset(7, std::env::temp_dir().join("transcript-words.mp4"))
            .format("mp4")
            .duration(Tick(2000))
            .content_hash(ContentHash::from_bytes(b"words"))
            .build(),
    );
    for track in tracks {
        data.scene_mut().timeline.insert_track(track);
    }
    Session::new(Project::new(data).unwrap(), 10).unwrap()
}

#[test]
fn cues_and_words_report_output_time_text_and_flags() {
    let mut protected = cue("đã-sửa", 100, 200, "Xin chào", &[(100, 200)]);
    protected.texts.get_mut(&language("vi")).unwrap().edited = true;
    let s = session(vec![
        track(
            1,
            false,
            vec![caption_item(
                4,
                1000,
                (0, 1000),
                vec![cue("câu-1", 100, 400, "Một hai", &[(100, 200), (250, 350)])],
            )],
        ),
        track(
            2,
            true,
            vec![caption_item(6, 2000, (0, 1000), vec![protected])],
        ),
    ]);
    let before = s.project().clone();

    let view = s.transcript().unwrap();
    assert_eq!(view.cues.len(), 2);
    let first = &view.cues[0];
    assert_eq!(first.item, ItemId(4));
    assert_eq!(first.track, TrackId(1));
    assert_eq!(first.output, range(1100, 1400));
    assert_eq!(&first.id, &NfcText::from("câu-1"));
    assert!(!first.protected);
    assert!(!first.locked);
    assert_eq!(
        first.texts.get(&language("vi")).unwrap().text.as_str(),
        "Một hai"
    );
    assert_eq!(first.words.len(), 2);
    assert_eq!(first.words[0].id.item, ItemId(4));
    assert_eq!(&first.words[0].id.cue, &NfcText::from("câu-1"));
    assert_eq!(first.words[0].id.word, 0);
    assert_eq!(first.words[0].output, range(1100, 1200));
    assert_eq!(
        first.words[0].text.as_ref().unwrap().as_str(),
        "Một",
        "measured words pair their source text token"
    );
    assert_eq!(first.words[1].output, range(1250, 1350));

    let second = &view.cues[1];
    assert!(second.protected, "an edited source text protects its words");
    assert!(second.locked, "the read model marks a locked track");
    assert!(second.words[0].protected && second.words[0].locked);
    assert_eq!(second.words[0].output, range(2100, 2200));
    assert_eq!(
        second.words[0].text, None,
        "mismatched token and timing counts never invent word text"
    );
    assert_eq!(*s.project(), before, "the read model changes nothing");
}

#[test]
fn repeated_spans_trims_and_missing_captions_map_without_duplication() {
    let shared = cue("câu-1", 100, 400, "Một hai", &[(100, 200), (250, 350)]);
    let s = session(vec![track(
        1,
        false,
        vec![
            caption_item(4, 0, (0, 1000), vec![shared.clone()]),
            caption_item(6, 2000, (0, 1000), vec![shared]),
            // A trim clips the item window and the retained cue together.
            caption_item(
                8,
                3000,
                (150, 1000),
                vec![cue("câu-1", 150, 400, "Một hai", &[(100, 200)])],
            ),
        ],
    )]);
    let view = s.transcript().unwrap();
    assert_eq!(view.cues.len(), 3);
    assert_eq!(view.cues[0].words[0].output, range(100, 200));
    assert_eq!(view.cues[1].words[0].output, range(2100, 2200));
    assert_ne!(view.cues[0].item, view.cues[1].item);
    assert_eq!(
        view.cues[2].words[0].output,
        range(3000, 3050),
        "a word extending before the trimmed window is clipped, not duplicated"
    );
    assert_eq!(view.cues[2].words[0].id.word, 0);

    let empty = Session::new(Project::empty(ProjectId(1), "".into()).unwrap(), 1).unwrap();
    assert!(empty.transcript().unwrap().cues.is_empty());
}

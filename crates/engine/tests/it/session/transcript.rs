//! Verifies the caption cue and word read model.
#![allow(clippy::unwrap_used, reason = "validated test fixtures and assertions")]
use crate::support::session::{
    assert_unchanged, caption_words_item, clip_item, project, word_asset, word_cue,
};
use captions::{Cue, CueText, CueTrack, LanguageTag, NfcText};
use engine::{Command, EditError, Session, WordId};
use media::{AssetId, ContentHash};
use project::{
    CaptionParams, Item, ItemId, ItemParams, Project, ProjectId, Track, TrackId, TrackKind,
};
use proptest::prelude::*;
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
        word_timing: captions::CueWordTiming::Unverified,
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
            word_highlight: false,
            preset: None,
            overrides: captions::CaptionOverrides::default(),
            cues: CueTrack {
                styles: Vec::new(),
                cues,
            },
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
fn caption_session(tracks: Vec<Track>) -> Session {
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
    let s = caption_session(vec![
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
    let s = caption_session(vec![track(
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
/// Two adjacent visual clips and one caption item whose third word crosses the
/// clip boundary at 600.
fn word_project(edited: bool, locked: bool) -> Session {
    let mut data = project().into_data();
    word_asset(&mut data, 1200);
    let main = data.scene().timeline.main.id;
    data.scene_mut()
        .timeline
        .iter_mut()
        .find(|track| track.id == main)
        .unwrap()
        .items = vec![clip_item(5, 0, (0, 600)), clip_item(6, 600, (600, 1200))];
    data.scene_mut().timeline.insert_track(Track {
        id: TrackId(9),
        name: "Captions".into(),
        kind: TrackKind::Text,
        muted: false,
        hidden: false,
        locked,
        items: vec![caption_words_item(
            4,
            0,
            (0, 1200),
            vec![word_cue(
                "câu-1",
                0,
                1200,
                &[(50, 150), (150, 250), (550, 650), (800, 900)],
                edited,
            )],
        )],
    });
    Session::new(Project::new(data).unwrap(), 10).unwrap()
}
/// One source-language cue with an explicit text and measured word ranges.
fn merge_cue(
    id: &str,
    start: i64,
    end: i64,
    text: &str,
    words: &[(i64, i64)],
    edited: bool,
) -> captions::Cue {
    let language = captions::LanguageTag::parse("vi").unwrap();
    captions::Cue {
        word_timing: captions::CueWordTiming::Unverified,
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
        word_ranges: words.iter().map(|&(a, b)| range(a, b)).collect(),
        style: None,
    }
}
/// One caption item with three adjacent measured-word cues and an English
/// translation on each. The first cue's source text is optionally protected and
/// the caption track is optionally locked.
fn merge_project(edited: bool, locked: bool) -> Session {
    let mut data = project().into_data();
    word_asset(&mut data, 1200);
    let mut cues = vec![
        merge_cue("câu-1", 0, 400, "Một hai", &[(0, 100), (100, 200)], edited),
        merge_cue(
            "câu-2",
            400,
            900,
            "ba bốn",
            &[(400, 500), (500, 600)],
            false,
        ),
        merge_cue("câu-3", 900, 1200, "năm", &[(900, 1000)], false),
    ];
    let english = captions::LanguageTag::parse("en").unwrap();
    let chinese = captions::LanguageTag::parse("zh").unwrap();
    let japanese = captions::LanguageTag::parse("ja").unwrap();
    for (index, cue) in cues.iter_mut().enumerate() {
        let text = match index {
            0 => "one two",
            1 => "three four",
            _ => "five",
        };
        cue.texts.insert(
            english.clone(),
            captions::CueText {
                text: text.into(),
                edited: index == 1,
                translation_ref: Some("translation/1".into()),
            },
        );
        let text = match index {
            0 => "你好",
            1 => "世界",
            _ => "第三",
        };
        cue.texts.insert(
            chinese.clone(),
            captions::CueText {
                text: text.into(),
                edited: false,
                translation_ref: None,
            },
        );
        let text = match index {
            0 => "こんにちは",
            1 => "世界",
            _ => "三",
        };
        cue.texts.insert(
            japanese.clone(),
            captions::CueText {
                text: text.into(),
                edited: false,
                translation_ref: None,
            },
        );
    }
    data.scene_mut().timeline.insert_track(Track {
        id: TrackId(9),
        name: "Captions".into(),
        kind: TrackKind::Text,
        muted: false,
        hidden: false,
        locked,
        items: vec![caption_words_item(4, 0, (0, 1200), cues)],
    });
    Session::new(Project::new(data).unwrap(), 10).unwrap()
}
/// Word output ranges of the active scene keyed by cue word index.
fn word_layout(session: &Session) -> Vec<(usize, i64, i64)> {
    let mut words: Vec<(usize, i64, i64)> = session
        .transcript()
        .unwrap()
        .cues
        .iter()
        .flat_map(|cue| &cue.words)
        .map(|word| (word.id.word, word.output.start().0, word.output.end().0))
        .collect();
    words.sort_unstable();
    words
}
#[test]
fn transcript_delete_cuts_a_word_across_a_clip_boundary_as_one_undo_step() {
    let mut s = word_project(false, false);
    let before = project::encode(s.project(), false).unwrap();
    let revision = s.revision();
    let expected_texts = word_cue(
        "câu-1",
        0,
        1200,
        &[(50, 150), (150, 250), (550, 650), (800, 900)],
        false,
    )
    .texts;
    assert_eq!(
        word_layout(&s),
        vec![(0, 50, 150), (1, 150, 250), (2, 550, 650), (3, 800, 900)]
    );
    let cue = s.transcript().unwrap().cues[0].clone();
    let outcome = s
        .apply(Command::TranscriptDeleteRange {
            word_ids: vec![WordId {
                item: cue.item,
                cue: cue.id,
                word: 2,
            }],
        })
        .unwrap();
    assert!(outcome.changed);
    assert_eq!(s.history_len(), (1, 0), "one undo step per command");
    assert_ne!(s.revision(), revision);
    // The span crosses the 600 boundary: both clips are cut and the tails close.
    let main = s.project().data().scene().timeline.main.id;
    let layout: Vec<(Tick, Tick)> = s
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .find(|track| track.id == main)
        .unwrap()
        .items
        .iter()
        .map(|item| (item.start, item.end().unwrap()))
        .collect();
    assert_eq!(layout, vec![(Tick(0), Tick(550)), (Tick(550), Tick(1100))]);
    assert_eq!(
        word_layout(&s),
        vec![(0, 50, 150), (1, 150, 250), (3, 700, 800)]
    );

    // Caption text, language and edit flags survive the cut; no cue is rewritten.
    let view = s.transcript().unwrap();
    for cue in &view.cues {
        assert_eq!(cue.texts, expected_texts);
        assert!(!cue.protected);
        assert!(!cue.locked);
    }

    s.undo().unwrap();
    assert_eq!(project::encode(s.project(), false).unwrap(), before);
    s.redo().unwrap();
    let edited = project::encode(s.project(), false).unwrap();
    assert_eq!(
        word_layout(&s),
        vec![(0, 50, 150), (1, 150, 250), (3, 700, 800)]
    );

    // A nested batch child still cuts through the reviewed-span path.
    let mut batched = word_project(false, false);
    batched
        .apply(Command::Batch {
            commands: vec![Command::TranscriptDeleteRange {
                word_ids: vec![WordId {
                    item: ItemId(4),
                    cue: "câu-1".into(),
                    word: 2,
                }],
            }],
        })
        .unwrap();
    assert_eq!(project::encode(batched.project(), false).unwrap(), edited);
}
#[test]
fn transcript_delete_keeps_repeated_spans_and_trimmed_items_distinct() {
    let mut data = project().into_data();
    word_asset(&mut data, 1200);
    let cues = || vec![word_cue("câu-1", 0, 1200, &[(50, 150), (150, 250)], false)];
    data.scene_mut().timeline.insert_track(Track {
        id: TrackId(9),
        name: "Captions".into(),
        kind: TrackKind::Text,
        muted: false,
        hidden: false,
        locked: false,
        items: vec![
            caption_words_item(4, 0, (0, 1200), cues()),
            caption_words_item(8, 2000, (0, 1200), cues()),
            caption_words_item(
                12,
                4000,
                (100, 1200),
                vec![word_cue("câu-1", 100, 1200, &[(100, 150)], false)],
            ),
        ],
    });
    let mut s = Session::new(Project::new(data).unwrap(), 10).unwrap();
    s.apply(Command::TranscriptDeleteRange {
        word_ids: vec![WordId {
            item: ItemId(4),
            cue: "câu-1".into(),
            word: 1,
        }],
    })
    .unwrap();
    let mut words: Vec<(u64, usize, i64, i64)> = s
        .transcript()
        .unwrap()
        .cues
        .iter()
        .flat_map(|cue| &cue.words)
        .map(|word| {
            (
                word.id.item.0,
                word.id.word,
                word.output.start().0,
                word.output.end().0,
            )
        })
        .collect();
    words.sort_unstable();
    assert_eq!(
        words,
        vec![
            (4, 0, 50, 150),
            // The repeated source span in another item keeps both words, shifted
            // left with the tail the cut vacated on this track.
            (8, 0, 1950, 2050),
            (8, 1, 2050, 2150),
            // A trimmed item clips its word instead of duplicating it.
            (12, 0, 3900, 3950),
        ]
    );
    let mut outputs: Vec<(i64, i64)> = words.iter().map(|&(_, _, a, b)| (a, b)).collect();
    outputs.dedup();
    assert_eq!(outputs.len(), words.len(), "no output range is duplicated");
}
#[test]
fn transcript_delete_refuses_protected_locked_and_foreign_words_atomically() {
    let word = |item: u64, cue: &str, index: usize| WordId {
        item: ItemId(item),
        cue: cue.into(),
        word: index,
    };
    let cases = vec![
        vec![],
        vec![word(4, "câu-1", 0), word(4, "câu-1", 0)],
        vec![word(4, "câu-1", 9)],
        vec![word(4, "khác", 0)],
        vec![word(77, "câu-1", 0)],
        vec![word(4, "câu-1", usize::MAX)],
        vec![word(6, "câu-1", 0)],
    ];

    let mut plain = word_project(false, false);
    let before = project::encode(plain.project(), false).unwrap();
    let revision = plain.revision();
    for word_ids in cases {
        assert!(
            plain
                .apply(Command::TranscriptDeleteRange {
                    word_ids: word_ids.clone()
                })
                .is_err(),
            "{word_ids:?}"
        );
        assert_unchanged(&plain, &before, &revision);
    }

    let many: Vec<WordId> = (0..=engine::control::MAX_COLLECTION)
        .map(|index| word(4, "câu-1", index))
        .collect();
    assert!(matches!(
        plain.apply(Command::TranscriptDeleteRange { word_ids: many }),
        Err(EditError::Control(engine::control::ControlError::Limit))
    ));
    assert_unchanged(&plain, &before, &revision);

    // An edited cue text protects every one of its words.
    let mut protected = word_project(true, false);
    let before = project::encode(protected.project(), false).unwrap();
    let revision = protected.revision();
    assert!(matches!(
        protected.apply(Command::TranscriptDeleteRange {
            word_ids: vec![word(4, "câu-1", 0)]
        }),
        Err(EditError::Protected)
    ));
    assert_unchanged(&protected, &before, &revision);

    // A locked caption track refuses the whole edit.
    let mut locked = word_project(false, true);
    let before = project::encode(locked.project(), false).unwrap();
    let revision = locked.revision();
    assert!(matches!(
        locked.apply(Command::TranscriptDeleteRange {
            word_ids: vec![word(4, "câu-1", 0)]
        }),
        Err(EditError::Locked(_))
    ));
    assert_unchanged(&locked, &before, &revision);

    // A word that lives in another scene refuses with the typed scene boundary.
    let mut data = word_project(false, false).project().data().clone();
    let mut other = data.scene().clone();
    other.id = project::SceneId(7);
    other.name = "Khác".into();
    other.is_main = false;
    other.timeline = Default::default();
    other.timeline.main.id = TrackId(30);
    other.timeline.insert_track(Track {
        id: TrackId(20),
        name: "Captions".into(),
        kind: TrackKind::Text,
        muted: false,
        hidden: false,
        locked: false,
        items: vec![caption_words_item(
            90,
            0,
            (0, 1200),
            vec![word_cue("câu-2", 0, 1200, &[(0, 100)], false)],
        )],
    });
    let main = data.scene().clone();
    data.scenes = project::Scenes::try_from(vec![main, other]).unwrap();
    let mut two_scenes = Session::new(Project::new(data).unwrap(), 10).unwrap();
    let before = project::encode(two_scenes.project(), false).unwrap();
    let revision = two_scenes.revision();
    assert!(matches!(
        two_scenes.apply(Command::TranscriptDeleteRange {
            word_ids: vec![WordId {
                item: ItemId(90),
                cue: "câu-2".into(),
                word: 0
            }]
        }),
        Err(EditError::SceneBoundary)
    ));
    assert_unchanged(&two_scenes, &before, &revision);
    // The same words are editable while their scene is active.
    two_scenes
        .apply(Command::SwitchScene {
            scene: project::SceneId(7),
        })
        .unwrap();
    two_scenes
        .apply(Command::TranscriptDeleteRange {
            word_ids: vec![WordId {
                item: ItemId(90),
                cue: "câu-2".into(),
                word: 0,
            }],
        })
        .unwrap();
}
proptest! {
    #[test]
    fn transcript_delete_mapping_round_trips_within_a_frame(
        length in 500_i64..5_000,
        first_start in 0_i64..400,
        first_len in 1_i64..100,
        gap in 0_i64..300,
        second_len in 1_i64..100,
    ) {
        let first_end = first_start + first_len;
        let second_start = first_end + gap;
        let second_end = (second_start + second_len).min(length);
        prop_assume!(second_end > second_start && first_end <= second_start);
        let mut data = project().into_data();
        word_asset(&mut data, length);
        data.scene_mut().timeline.insert_track(Track {
            id: TrackId(9),
            name: "Captions".into(),
            kind: TrackKind::Text,
            muted: false,
            hidden: false,
            locked: false,
            items: vec![caption_words_item(
                4,
                0,
                (0, length),
                vec![word_cue(
                    "câu-1",
                    0,
                    length,
                    &[(first_start, first_end), (second_start, second_end)],
                    false,
                )],
            )],
        });
        let mut s = Session::new(Project::new(data).unwrap(), 10).unwrap();
        s.apply(Command::TranscriptDeleteRange {
            word_ids: vec![WordId {
                item: ItemId(4),
                cue: "câu-1".into(),
                word: 0,
            }],
        }).unwrap();
        let frame = time::FrameRate::FPS_30.tick_of(1).unwrap();
        for track in s.project().data().scene().timeline.iter() {
            for item in &track.items {
                for word in item.word_spans().unwrap() {
                    prop_assert_eq!(
                        item.output_time_of(word.source.start()).unwrap(),
                        word.output.start()
                    );
                    let back = item.source_time_at(word.output.start()).unwrap();
                    prop_assert!((back.0 - word.source.start().0).abs() <= frame.0);
                    let back = item.source_time_at(word.output.end()).unwrap();
                    prop_assert!((back.0 - word.source.end().0).abs() <= frame.0);
                }
            }
        }
        // The second word survives with its source duration and its text.
        let view = s.transcript().unwrap();
        let words: Vec<_> = view.cues.iter().flat_map(|cue| &cue.words).collect();
        prop_assert_eq!(words.len(), 1);
        prop_assert_eq!(words[0].id.word, 1);
        prop_assert_eq!(words[0].output.duration().0, second_end - second_start);
        prop_assert_eq!(words[0].text.as_ref().unwrap().as_str(), "hai");
    }
}
// --- Transcript cue merges ------------------------------------------------

#[test]
fn transcript_merge_joins_adjacent_cues_as_one_undo_step() {
    let mut s = merge_project(false, false);
    let before = project::encode(s.project(), false).unwrap();
    let revision = s.revision();
    // Input order is irrelevant: the selected cues sort by time.
    let cue_id = |item, cue: &str| engine::CueId {
        item: ItemId(item),
        cue: cue.into(),
    };
    let outcome = s
        .apply(Command::TranscriptMergeCues {
            cue_ids: vec![cue_id(4, "câu-2"), cue_id(4, "câu-1")],
        })
        .unwrap();
    assert!(outcome.changed);
    assert_eq!(s.history_len(), (1, 0), "one undo step per merge");
    assert_ne!(s.revision(), revision);

    let view = s.transcript().unwrap();
    assert_eq!(view.cues.len(), 2, "cue-3 stays separate");
    let merged = &view.cues[0];
    assert_eq!(merged.id.as_str(), "câu-1", "the earliest cue keeps its id");
    assert_eq!(merged.output, range(0, 900));
    assert_eq!(
        merged.texts[&captions::LanguageTag::parse("vi").unwrap()]
            .text
            .as_str(),
        "Một hai ba bốn"
    );
    let english = &merged.texts[&captions::LanguageTag::parse("en").unwrap()];
    assert_eq!(english.text.as_str(), "one two three four");
    assert!(english.edited, "an edited translation stays edited");
    assert_eq!(
        english.translation_ref.as_ref().unwrap().as_str(),
        "translation/1"
    );
    assert_eq!(
        merged
            .words
            .iter()
            .map(|word| (
                word.id.word,
                word.text.as_ref().unwrap().as_str(),
                word.output
            ))
            .collect::<Vec<_>>(),
        vec![
            (0, "Một", range(0, 100)),
            (1, "hai", range(100, 200)),
            (2, "ba", range(400, 500)),
            (3, "bốn", range(500, 600)),
        ],
        "word timings concatenate in cue order"
    );
    assert_eq!(view.cues[1].id.as_str(), "câu-3");
    assert_eq!(view.cues[1].output, range(900, 1200));

    s.undo().unwrap();
    assert_eq!(project::encode(s.project(), false).unwrap(), before);
    s.redo().unwrap();
    assert_eq!(
        s.transcript().unwrap().cues[0].output,
        range(0, 900),
        "redo restores the merge"
    );

    // A run of more than two adjacent cues is one merge.
    s.apply(Command::TranscriptMergeCues {
        cue_ids: vec![cue_id(4, "câu-3"), cue_id(4, "câu-1")],
    })
    .unwrap();
    let view = s.transcript().unwrap();
    assert_eq!(view.cues.len(), 1);
    assert_eq!(view.cues[0].output, range(0, 1200));
    assert_eq!(view.cues[0].words.len(), 5);
    assert_eq!(
        view.cues[0].texts[&captions::LanguageTag::parse("vi").unwrap()]
            .text
            .as_str(),
        "Một hai ba bốn năm"
    );
}
#[test]
fn transcript_merge_joins_text_by_language_word_spacing() {
    let mut s = merge_project(false, false);
    s.apply(Command::TranscriptMergeCues {
        cue_ids: vec![
            engine::CueId {
                item: ItemId(4),
                cue: "câu-2".into(),
            },
            engine::CueId {
                item: ItemId(4),
                cue: "câu-1".into(),
            },
        ],
    })
    .unwrap();
    // The merged cue carries several scripts; each language keeps its own rule.
    let merged = s
        .transcript()
        .unwrap()
        .cues
        .first()
        .cloned()
        .expect("one merged cue");
    for (language, expected) in [
        ("vi", "Một hai ba bốn"),
        ("en", "one two three four"),
        ("zh", "你好世界"),
        ("ja", "こんにちは世界"),
    ] {
        let tag = captions::LanguageTag::parse(language).unwrap();
        assert_eq!(
            merged.texts.get(&tag).map(|text| text.text.as_str()),
            Some(expected),
            "{language} join"
        );
    }
}
#[test]
fn transcript_merge_after_a_trim_maps_through_the_retained_source() {
    let mut s = merge_project(false, false);
    let before = project::encode(s.project(), false).unwrap();
    let outcome = s
        .apply(Command::Batch {
            commands: vec![
                Command::TrimStart {
                    item: ItemId(4),
                    at: Tick(400),
                },
                Command::TranscriptMergeCues {
                    cue_ids: vec![
                        engine::CueId {
                            item: ItemId(4),
                            cue: "câu-3".into(),
                        },
                        engine::CueId {
                            item: ItemId(4),
                            cue: "câu-2".into(),
                        },
                    ],
                },
            ],
        })
        .unwrap();
    assert!(outcome.changed);
    assert_eq!(s.history_len(), (1, 0), "the batch is one undo step");
    let view = s.transcript().unwrap();
    assert_eq!(view.cues.len(), 1, "the trim drops cue-1 before the merge");
    assert_eq!(view.cues[0].id.as_str(), "câu-2");
    assert_eq!(view.cues[0].output, range(400, 1200));
    assert_eq!(
        view.cues[0].texts[&captions::LanguageTag::parse("vi").unwrap()]
            .text
            .as_str(),
        "ba bốn năm"
    );
    assert_eq!(
        view.cues[0]
            .words
            .iter()
            .map(|word| (word.id.word, word.output))
            .collect::<Vec<_>>(),
        vec![
            (0, range(400, 500)),
            (1, range(500, 600)),
            (2, range(900, 1000))
        ]
    );
    s.undo().unwrap();
    assert_eq!(project::encode(s.project(), false).unwrap(), before);
}
#[test]
fn transcript_merge_refuses_non_adjacent_cross_track_cross_item_and_protected_atomically() {
    let cue = |item: u64, cue: &str| engine::CueId {
        item: ItemId(item),
        cue: cue.into(),
    };
    let mut plain = merge_project(false, false);
    let before = project::encode(plain.project(), false).unwrap();
    let revision = plain.revision();
    let mut refused = |cue_ids: Vec<engine::CueId>| {
        let error = plain
            .apply(Command::TranscriptMergeCues { cue_ids })
            .unwrap_err();
        assert_unchanged(&plain, &before, &revision);
        error
    };
    assert!(matches!(
        refused(vec![]),
        EditError::Invalid("merge cue count")
    ));
    assert!(matches!(
        refused(vec![cue(4, "câu-1")]),
        EditError::Invalid("merge cue count")
    ));
    assert!(matches!(
        refused(vec![cue(4, "câu-1"), cue(4, "câu-1")]),
        EditError::Invalid("duplicate merge cue")
    ));
    assert!(matches!(
        refused(vec![cue(4, "câu-1"), cue(4, "khác")]),
        EditError::Invalid("merge cue")
    ));
    assert!(matches!(
        refused(vec![cue(77, "câu-1"), cue(4, "câu-1")]),
        EditError::NoItem(ItemId(77))
    ));
    assert!(matches!(
        refused(vec![cue(4, "câu-1"), cue(4, "câu-3")]),
        EditError::CuesNotAdjacent
    ));

    // Over-budget selections refuse before resolving any cue.
    let many: Vec<engine::CueId> = (0..=engine::control::MAX_COLLECTION)
        .map(|_| cue(4, "câu-1"))
        .collect();
    assert!(matches!(
        plain.apply(Command::TranscriptMergeCues { cue_ids: many }),
        Err(EditError::Control(engine::control::ControlError::Limit))
    ));
    assert_unchanged(&plain, &before, &revision);

    // Cues on different tracks refuse with a typed reason.
    let mut data = plain.project().data().clone();
    data.scene_mut().timeline.insert_track(Track {
        id: TrackId(11),
        name: "Other captions".into(),
        kind: TrackKind::Text,
        muted: false,
        hidden: false,
        locked: false,
        items: vec![caption_words_item(
            8,
            0,
            (0, 100),
            vec![word_cue("khác-1", 0, 100, &[], false)],
        )],
    });
    let mut cross_track = Session::new(Project::new(data).unwrap(), 10).unwrap();
    let bytes = project::encode(cross_track.project(), false).unwrap();
    let revision = cross_track.revision();
    assert!(matches!(
        cross_track.apply(Command::TranscriptMergeCues {
            cue_ids: vec![cue(4, "câu-1"), cue(8, "khác-1")]
        }),
        Err(EditError::CuesAcrossTracks)
    ));
    assert_unchanged(&cross_track, &bytes, &revision);

    // Cues of two items on one track refuse too.
    let mut data = plain.project().data().clone();
    let item = caption_words_item(
        8,
        1200,
        (0, 100),
        vec![word_cue("khác-1", 0, 100, &[], false)],
    );
    data.scene_mut()
        .timeline
        .iter_mut()
        .find(|track| track.id == TrackId(9))
        .unwrap()
        .items
        .push(item);
    let mut cross_item = Session::new(Project::new(data).unwrap(), 10).unwrap();
    let bytes = project::encode(cross_item.project(), false).unwrap();
    let revision = cross_item.revision();
    assert!(matches!(
        cross_item.apply(Command::TranscriptMergeCues {
            cue_ids: vec![cue(4, "câu-1"), cue(8, "khác-1")]
        }),
        Err(EditError::CuesAcrossItems)
    ));
    assert_unchanged(&cross_item, &bytes, &revision);

    // An edited source text protects every selected cue.
    let mut protected = merge_project(true, false);
    let bytes = project::encode(protected.project(), false).unwrap();
    let revision = protected.revision();
    assert!(matches!(
        protected.apply(Command::TranscriptMergeCues {
            cue_ids: vec![cue(4, "câu-1"), cue(4, "câu-2")]
        }),
        Err(EditError::Protected)
    ));
    assert_unchanged(&protected, &bytes, &revision);

    // A locked caption track refuses the edit.
    let mut locked = merge_project(false, true);
    let bytes = project::encode(locked.project(), false).unwrap();
    let revision = locked.revision();
    assert!(matches!(
        locked.apply(Command::TranscriptMergeCues {
            cue_ids: vec![cue(4, "câu-1"), cue(4, "câu-2")]
        }),
        Err(EditError::Locked(_))
    ));
    assert_unchanged(&locked, &bytes, &revision);

    // Mixed or broken word timing cannot be merged without losing alignment.
    let mut data = merge_project(false, false).project().data().clone();
    for track in data.scene_mut().timeline.iter_mut() {
        for item in &mut track.items {
            if let ItemParams::Caption(params) = &mut item.content {
                params.cues.cues[2].word_ranges.clear();
            }
        }
    }
    let mut untimed = Session::new(Project::new(data).unwrap(), 10).unwrap();
    let bytes = project::encode(untimed.project(), false).unwrap();
    let revision = untimed.revision();
    assert!(matches!(
        untimed.apply(Command::TranscriptMergeCues {
            cue_ids: vec![cue(4, "câu-2"), cue(4, "câu-3")]
        }),
        Err(EditError::Invalid("merge cue word timings"))
    ));
    assert_unchanged(&untimed, &bytes, &revision);

    // Different source languages cannot share one merged source text.
    let mut data = merge_project(false, false).project().data().clone();
    for track in data.scene_mut().timeline.iter_mut() {
        for item in &mut track.items {
            if let ItemParams::Caption(params) = &mut item.content {
                params.cues.cues[1].source_lang = captions::LanguageTag::parse("zh").unwrap();
            }
        }
    }
    let mut mixed = Session::new(Project::new(data).unwrap(), 10).unwrap();
    let bytes = project::encode(mixed.project(), false).unwrap();
    let revision = mixed.revision();
    assert!(matches!(
        mixed.apply(Command::TranscriptMergeCues {
            cue_ids: vec![cue(4, "câu-1"), cue(4, "câu-2")]
        }),
        Err(EditError::Invalid("merge cue source language"))
    ));
    assert_unchanged(&mixed, &bytes, &revision);

    // A cue is addressable only while its scene is active.
    let mut data = merge_project(false, false).project().data().clone();
    let mut other = data.scene().clone();
    other.id = project::SceneId(7);
    other.name = "Khác".into();
    other.is_main = false;
    other.timeline = Default::default();
    other.timeline.main.id = TrackId(30);
    other.timeline.insert_track(Track {
        id: TrackId(20),
        name: "Captions".into(),
        kind: TrackKind::Text,
        muted: false,
        hidden: false,
        locked: false,
        items: vec![caption_words_item(
            90,
            0,
            (0, 100),
            vec![word_cue("khác-2", 0, 100, &[], false)],
        )],
    });
    let main = data.scene().clone();
    data.scenes = project::Scenes::try_from(vec![main, other]).unwrap();
    let mut two_scenes = Session::new(Project::new(data).unwrap(), 10).unwrap();
    let bytes = project::encode(two_scenes.project(), false).unwrap();
    let revision = two_scenes.revision();
    assert!(matches!(
        two_scenes.apply(Command::TranscriptMergeCues {
            cue_ids: vec![cue(90, "khác-2"), cue(4, "câu-1")]
        }),
        Err(EditError::SceneBoundary)
    ));
    assert_unchanged(&two_scenes, &bytes, &revision);
}

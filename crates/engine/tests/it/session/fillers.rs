#![allow(clippy::unwrap_used, reason = "validated test fixtures and assertions")]
use crate::support::session::{
    assert_unchanged, caption_words_item, clip_item, five_clip_pairs, project, range, word_asset,
};
use engine::{Command, EditError, Session};
use project::{ItemId, Project, Track, TrackId, TrackKind};
use serde_json::json;
use time::Tick;

// --- Lexicon filler cuts --------------------------------------------------

/// One English cue with an explicit text and measured word ranges.
fn english_cue(
    id: &str,
    start: i64,
    end: i64,
    text: &str,
    words: &[(i64, i64)],
    edited: bool,
) -> captions::Cue {
    let language = captions::LanguageTag::parse("en").unwrap();
    captions::Cue {
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
/// One video clip and a caption item whose English text carries filled pauses and
/// a discourse marker. Cue-2's source text is optionally protected.
fn filler_project(edited: bool, locked: bool) -> Session {
    let mut data = project().into_data();
    word_asset(&mut data, 2400);
    data.scene_mut().timeline.main.items = vec![clip_item(5, 0, (0, 2400))];
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
            (0, 2400),
            vec![
                english_cue(
                    "câu-1",
                    0,
                    800,
                    "um hello uh there",
                    &[(0, 100), (100, 200), (200, 300), (300, 400)],
                    false,
                ),
                english_cue(
                    "câu-2",
                    800,
                    1600,
                    "like world",
                    &[(800, 900), (900, 1000)],
                    edited,
                ),
            ],
        )],
    });
    Session::new(Project::new(data).unwrap(), 10).unwrap()
}
#[test]
fn remove_fillers_cuts_selected_candidates_as_one_undo_step() {
    let mut s = filler_project(false, false);
    let before = project::encode(s.project(), false).unwrap();
    let revision = s.revision();
    let outcome = s
        .apply(Command::RemoveFillers {
            ids: vec!["filler:4:câu-1:0".into(), "filler:4:câu-2:800".into()],
            confirm_edited: false,
        })
        .unwrap();
    assert!(outcome.changed);
    assert_eq!(s.history_len(), (1, 0), "one undo step per removal");
    assert_ne!(s.revision(), revision);

    // Both spans leave the single clip cut twice and the tails closed.
    let layout: Vec<(Tick, Tick)> = s
        .project()
        .data()
        .scene()
        .timeline
        .main
        .items
        .iter()
        .map(|item| (item.start, item.end().unwrap()))
        .collect();
    assert_eq!(layout, vec![(Tick(0), Tick(700)), (Tick(700), Tick(2200))]);

    // The removed words are gone; the words beside them keep their text and order.
    let view = s.transcript().unwrap();
    let words: Vec<(&str, usize, Tick, Tick)> = view
        .cues
        .iter()
        .flat_map(|cue| {
            cue.words.iter().map(move |word| {
                (
                    cue.id.as_str(),
                    word.id.word,
                    word.output.start(),
                    word.output.end(),
                )
            })
        })
        .collect();
    assert_eq!(
        words,
        vec![
            ("câu-1", 1, Tick(0), Tick(100)),
            ("câu-1", 2, Tick(100), Tick(200)),
            ("câu-1", 3, Tick(200), Tick(300)),
            ("câu-2", 1, Tick(700), Tick(800)),
        ]
    );

    s.undo().unwrap();
    assert_eq!(project::encode(s.project(), false).unwrap(), before);
    s.redo().unwrap();
    let edited = project::encode(s.project(), false).unwrap();
    let loaded = project::decode(&edited).unwrap().project;
    assert_eq!(&loaded, s.project().as_ref());

    // A nested batch child still cuts through the ripple path as one step.
    let mut batched = filler_project(false, false);
    batched
        .apply(Command::Batch {
            commands: vec![Command::RemoveFillers {
                ids: vec!["filler:4:câu-1:0".into(), "filler:4:câu-2:800".into()],
                confirm_edited: false,
            }],
        })
        .unwrap();
    assert_eq!(project::encode(batched.project(), false).unwrap(), edited);
}
#[test]
fn remove_fillers_refuses_edited_unknown_stale_and_hostile_ids_atomically() {
    let mut plain = filler_project(false, false);
    let before = project::encode(plain.project(), false).unwrap();
    let revision = plain.revision();
    let mut refused = |ids: Vec<engine::NfcText>| {
        let error = plain
            .apply(Command::RemoveFillers {
                ids,
                confirm_edited: false,
            })
            .unwrap_err();
        assert_unchanged(&plain, &before, &revision);
        error
    };
    assert!(matches!(
        refused(vec![]),
        EditError::Invalid("empty filler selection")
    ));
    assert!(matches!(
        refused(vec!["filler:4:câu-1:0".into(), "filler:4:câu-1:0".into()]),
        EditError::Invalid("duplicate filler")
    ));
    // A start that is not a measured word, an unknown cue, item and shape.
    for id in [
        "filler:4:câu-1:50",
        "filler:4:khác:0",
        "filler:77:câu-1:0",
        "nope",
        "",
    ] {
        assert!(
            matches!(
                refused(vec![id.into()]),
                EditError::Invalid("stale filler candidate")
            ),
            "{id}"
        );
    }
    let many: Vec<engine::NfcText> = (0..=engine::control::MAX_COLLECTION)
        .map(|_| "filler:4:câu-1:0".into())
        .collect();
    assert!(matches!(
        plain.apply(Command::RemoveFillers {
            ids: many,
            confirm_edited: false
        }),
        Err(EditError::Limit)
    ));
    assert_unchanged(&plain, &before, &revision);

    // An edited cue's candidate needs confirmation.
    let mut protected = filler_project(true, false);
    let before = project::encode(protected.project(), false).unwrap();
    let revision = protected.revision();
    assert!(matches!(
        protected.apply(Command::RemoveFillers {
            ids: vec!["filler:4:câu-2:800".into()],
            confirm_edited: false
        }),
        Err(EditError::Protected)
    ));
    assert_unchanged(&protected, &before, &revision);
    protected
        .apply(Command::RemoveFillers {
            ids: vec!["filler:4:câu-2:800".into()],
            confirm_edited: true,
        })
        .unwrap();
    assert_eq!(protected.history_len(), (1, 0));
    assert_eq!(
        protected
            .transcript()
            .unwrap()
            .cues
            .iter()
            .flat_map(|cue| &cue.words)
            .filter(|word| word.id.cue.as_str() == "câu-2")
            .count(),
        1,
        "only like is removed; world stays"
    );

    // A trim removes the word from the visible output, so its old id is stale.
    let mut trimmed = filler_project(false, false);
    trimmed
        .apply(Command::TrimStart {
            item: ItemId(4),
            at: Tick(150),
        })
        .unwrap();
    let bytes = project::encode(trimmed.project(), false).unwrap();
    let revision = trimmed.revision();
    assert!(matches!(
        trimmed.apply(Command::RemoveFillers {
            ids: vec!["filler:4:câu-1:0".into()],
            confirm_edited: false
        }),
        Err(EditError::Invalid("stale filler candidate"))
    ));
    assert_eq!(project::encode(trimmed.project(), false).unwrap(), bytes);
    assert_eq!(trimmed.revision(), revision);
    assert_eq!(trimmed.history_len(), (1, 0), "the trim stays one step");
    // The visible part of the word keeps the same id.
    let mut clipped = filler_project(false, false);
    clipped
        .apply(Command::TrimStart {
            item: ItemId(4),
            at: Tick(50),
        })
        .unwrap();
    clipped
        .apply(Command::RemoveFillers {
            ids: vec!["filler:4:câu-1:0".into()],
            confirm_edited: false,
        })
        .unwrap();

    // A locked caption track refuses the whole edit.
    let mut locked = filler_project(false, true);
    let before = project::encode(locked.project(), false).unwrap();
    let revision = locked.revision();
    assert!(matches!(
        locked.apply(Command::RemoveFillers {
            ids: vec!["filler:4:câu-1:0".into()],
            confirm_edited: false
        }),
        Err(EditError::Locked(_))
    ));
    assert_unchanged(&locked, &before, &revision);
}
#[test]
fn remove_fillers_accepts_a_classified_non_lexicon_word() {
    let mut s = filler_project(false, false);
    // `hello` is a visible measured word but not a lexicon filler; LLM
    // classification may return it and removal must apply that reviewed id.
    s.execute(
        "timeline.remove_fillers",
        json!({"ids":["filler:4:câu-1:100"],"confirm_edited":false}),
    )
    .unwrap();
    assert_eq!(s.history_len(), (1, 0));
    let words: Vec<String> = s
        .transcript()
        .unwrap()
        .cues
        .iter()
        .flat_map(|cue| &cue.words)
        .filter_map(|word| word.text.as_ref().map(|text| text.as_str().to_owned()))
        .collect();
    assert_eq!(words, ["um", "uh", "there", "like", "world"]);
}
#[test]
fn remove_fillers_keeps_five_clip_audio_video_aligned() {
    let second = time::TICKS_PER_SECOND;
    let tenth = second / 10;
    let mut data = five_clip_pairs().project().data().clone();
    word_asset(&mut data, 12 * second);
    data.scene_mut().timeline.insert_track(Track {
        id: TrackId(20),
        name: "Captions".into(),
        kind: TrackKind::Text,
        muted: false,
        hidden: false,
        locked: false,
        items: vec![caption_words_item(
            40,
            0,
            (0, 12 * second),
            vec![english_cue(
                "câu-1",
                0,
                12 * second,
                "um hello",
                &[
                    (second, second + tenth),
                    (second + tenth, second + 2 * tenth),
                ],
                false,
            )],
        )],
    });
    let mut s = Session::new(Project::new(data).unwrap(), 10).unwrap();
    let before = project::encode(s.project(), false).unwrap();
    let id = format!("filler:40:câu-1:{second}");
    s.apply(Command::RemoveFillers {
        ids: vec![id.into()],
        confirm_edited: false,
    })
    .unwrap();
    assert_eq!(s.history_len(), (1, 0));

    let starts = |kind: TrackKind| -> Vec<Tick> {
        let mut starts: Vec<Tick> = s
            .project()
            .data()
            .scene()
            .timeline
            .iter()
            .filter(|track| track.kind == kind)
            .flat_map(|track| track.items.iter().map(|item| item.start))
            .collect();
        starts.sort_unstable();
        starts
    };
    assert_eq!(starts(TrackKind::Video), starts(TrackKind::Audio));
    assert_eq!(
        s.project()
            .data()
            .scene()
            .timeline
            .main
            .items
            .last()
            .unwrap()
            .end()
            .unwrap(),
        Tick(12 * second - tenth)
    );
    let words: Vec<usize> = s
        .transcript()
        .unwrap()
        .cues
        .iter()
        .flat_map(|cue| &cue.words)
        .map(|word| word.id.word)
        .collect();
    assert_eq!(words, vec![1], "the filler word alone is cut");

    s.undo().unwrap();
    assert_eq!(project::encode(s.project(), false).unwrap(), before);
}

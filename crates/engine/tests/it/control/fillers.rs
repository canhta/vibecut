#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "public-interface conformance assertions"
)]
use crate::support::control::{
    Allow, error, fillers, finish, fixture_chapters, ok, request, run, runner, seed, unchanged,
    with_words,
};
use engine::Session;
use engine::control::{Caller, ControlResult, ErrorCode};
use project::{ItemId, ItemParams, Project, ProjectId, Track, TrackId, TrackKind};
use serde_json::json;
use time::{SourceRange, Tick};

#[test]
fn fillers_classify_uses_the_lent_consent_and_feeds_remove_fillers() {
    let mut s = with_words(&seed());
    let before = serde_json::to_vec(s.project().data()).unwrap();
    let mut r = runner();
    let text = fixture_chapters();
    let allow = Allow;

    // An agent transport without a lent text access cannot upload.
    let req = request(
        &s,
        "fillers.classify",
        json!({"language":"en","include_markers":true}),
        "fillers-denied",
    );
    error(
        s.control_request(
            req,
            Caller {
                principal: "local-mcp",
                external: true,
                policy: &allow,
                fonts: None,
                sounds: None,
                music: None,
                stickers: None,
                brand: None,
                media: None,
                chapters: None,
                captions: None,
                vision: None,
            },
            &mut r,
        )
        .unwrap(),
        ErrorCode::Permission,
    );

    let req = request(
        &s,
        "fillers.classify",
        json!({"language":"en","include_markers":true}),
        "fillers",
    );
    let value = ok(s
        .control_request(
            req,
            Caller {
                principal: "ui",
                external: false,
                policy: &allow,
                fonts: None,
                sounds: None,
                music: None,
                stickers: None,
                brand: None,
                media: None,
                chapters: Some(&text),
                captions: None,
                vision: None,
            },
            &mut r,
        )
        .unwrap());
    let ControlResult::JobAccepted { job, .. } = value else {
        panic!("{value:?}");
    };
    finish(&mut r, job);
    let req = request(&s, "job_status", json!({"job":job.0}), "fillers-status");
    let value = ok(s
        .control_request(
            req,
            Caller {
                principal: "ui",
                external: false,
                policy: &allow,
                fonts: None,
                sounds: None,
                music: None,
                stickers: None,
                brand: None,
                media: None,
                chapters: Some(&text),
                captions: None,
                vision: None,
            },
            &mut r,
        )
        .unwrap());
    let ControlResult::Job { snapshot } = value else {
        panic!("{value:?}");
    };
    let Some(engine::control::JobOutput::Fillers { candidates }) = snapshot.output.clone() else {
        panic!("fillers output: {snapshot:?}");
    };
    assert_eq!(
        candidates
            .iter()
            .map(|candidate| candidate.id.as_str())
            .collect::<Vec<_>>(),
        ["filler:4:cue-1:100", "filler:4:cue-2:500"]
    );
    assert_eq!(candidates[0].range.start, Tick(100));
    assert_eq!(candidates[0].kind, captions::FillerKind::FilledPause);
    assert_eq!(serde_json::to_vec(s.project().data()).unwrap(), before);

    // The classified ids feed the existing removal command unchanged, including
    // `brave`, which no lexicon contains.
    let ids: Vec<_> = candidates
        .iter()
        .map(|candidate| candidate.id.clone())
        .collect();
    let req = request(
        &s,
        "timeline.remove_fillers",
        json!({"ids": ids, "confirm_edited": false}),
        "fillers-remove",
    );
    let value = ok(s
        .control_request(
            req,
            Caller {
                principal: "ui",
                external: false,
                policy: &allow,
                fonts: None,
                sounds: None,
                music: None,
                stickers: None,
                brand: None,
                media: None,
                chapters: None,
                captions: None,
                vision: None,
            },
            &mut r,
        )
        .unwrap());
    let ControlResult::Edit { outcome } = value else {
        panic!("{value:?}");
    };
    assert!(outcome.changed);
    let words: Vec<String> = s
        .transcript()
        .unwrap()
        .cues
        .iter()
        .flat_map(|cue| &cue.words)
        .filter_map(|word| word.text.as_ref().map(|text| text.as_str().to_owned()))
        .collect();
    assert_eq!(words, ["Um"]);
}
#[test]
fn filler_candidates_read_stable_output_occurrences_and_gate_markers() {
    let mut s = with_words(&seed());
    let mut r = runner();
    let before = project::encode(s.project(), false).unwrap();
    let revision = s.revision();
    let history = s.history_len();

    let found = fillers(&mut s, &mut r, "en", false, "fillers-1");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].id.as_str(), "filler:4:cue-1:0");
    assert_eq!(found[0].item, ItemId(4));
    assert_eq!(found[0].cue.as_str(), "cue-1");
    assert_eq!(
        (found[0].range.start, found[0].range.end),
        (Tick(0), Tick(100))
    );
    assert_eq!(found[0].kind, captions::FillerKind::FilledPause);
    assert!(!found[0].edited_cue);
    // The read changes no project, revision or history.
    unchanged(&s, &before, &revision, history);

    // Discourse markers are opt-in and keep their kind.
    let found = fillers(&mut s, &mut r, "en", true, "fillers-2");
    assert_eq!(
        found.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
        vec!["filler:4:cue-1:0", "filler:4:cue-2:500"]
    );
    assert_eq!(found[1].kind, captions::FillerKind::DiscourseMarker);
    assert_eq!(
        (found[1].range.start, found[1].range.end),
        (Tick(500), Tick(600))
    );

    // Only cues in the requested lexicon's language match; an unknown language
    // is a typed refusal rather than an empty list.
    assert!(fillers(&mut s, &mut r, "vi", false, "fillers-3").is_empty());
    error(
        run(
            &mut s,
            &mut r,
            "timeline.filler_candidates",
            json!({"language":"ja","include_markers":false}),
            "fillers-4",
        ),
        ErrorCode::Refused,
    );

    // A trim clips the candidate to the visible words and keeps its stable id.
    ok(run(
        &mut s,
        &mut r,
        "trim_start",
        json!({"item":4,"at":50}),
        "fillers-trim",
    ));
    let found = fillers(&mut s, &mut r, "en", false, "fillers-5");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].id.as_str(), "filler:4:cue-1:0");
    assert_eq!(
        (found[0].range.start, found[0].range.end),
        (Tick(50), Tick(100))
    );

    // Once the whole word is outside the item's output span, the id is stale.
    ok(run(
        &mut s,
        &mut r,
        "trim_start",
        json!({"item":4,"at":150}),
        "fillers-trim-2",
    ));
    let before = project::encode(s.project(), false).unwrap();
    let revision = s.revision();
    assert!(fillers(&mut s, &mut r, "en", false, "fillers-6").is_empty());
    error(
        run(
            &mut s,
            &mut r,
            "timeline.remove_fillers",
            json!({"ids":["filler:4:cue-1:0"],"confirm_edited":false}),
            "fillers-stale",
        ),
        ErrorCode::Refused,
    );
    unchanged(&s, &before, &revision, (2, 0));

    // Cues without measured word timings yield no candidates.
    let mut data = with_words(&seed()).project().data().clone();
    for item in data
        .scene_mut()
        .timeline
        .iter_mut()
        .flat_map(|track| &mut track.items)
    {
        if let ItemParams::Caption(params) = &mut item.content {
            for cue in &mut params.cues.cues {
                cue.word_timing = captions::CueWordTiming::Unverified;
            }
        }
    }
    let mut untimed = Session::new(Project::new(data).unwrap(), 10).unwrap();
    let mut r = runner();
    assert!(fillers(&mut untimed, &mut r, "en", true, "fillers-untimed").is_empty());
}
#[test]
fn remove_fillers_confirms_edited_cues_and_is_one_reviewed_step() {
    let mut data = with_words(&seed()).project().data().clone();
    for item in data
        .scene_mut()
        .timeline
        .iter_mut()
        .flat_map(|track| &mut track.items)
    {
        if let ItemParams::Caption(params) = &mut item.content
            && let Some(cue) = params.cues.cues.first_mut()
        {
            cue.texts.values_mut().next().unwrap().edited = true;
        }
    }
    let mut s = Session::new(Project::new(data).unwrap(), 10).unwrap();
    let mut r = runner();
    let found = fillers(&mut s, &mut r, "en", false, "filler-remove-read");
    assert_eq!(found.len(), 1);
    assert!(found[0].edited_cue);
    let before = project::encode(s.project(), false).unwrap();
    let revision = s.revision();

    error(
        run(
            &mut s,
            &mut r,
            "timeline.remove_fillers",
            json!({"ids":[found[0].id.as_str()],"confirm_edited":false}),
            "filler-remove-refused",
        ),
        ErrorCode::Refused,
    );
    unchanged(&s, &before, &revision, (0, 0));

    let result = ok(run(
        &mut s,
        &mut r,
        "timeline.remove_fillers",
        json!({"ids":[found[0].id.as_str()],"confirm_edited":true}),
        "filler-remove-confirmed",
    ));
    assert!(matches!(result, ControlResult::Edit { outcome } if outcome.changed));
    assert_eq!(s.history_len(), (1, 0));
    assert!(fillers(&mut s, &mut r, "en", false, "filler-remove-after").is_empty());
}
#[derive(serde::Deserialize)]
struct AsrTranscript {
    text: String,
    words: Vec<AsrWord>,
}
#[derive(serde::Deserialize)]
struct AsrWord {
    start: f64,
    end: f64,
    #[serde(rename = "type")]
    kind: String,
}
#[test]
#[ignore = "manual: set VIBECUT_INTERVIEW_ASR to .samples/cuts/09/asr.json"]
fn interview_filler_candidates_match_the_manual_read() {
    let Ok(path) = std::env::var("VIBECUT_INTERVIEW_ASR") else {
        return;
    };
    let asr: AsrTranscript = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let words: Vec<SourceRange> = asr
        .words
        .iter()
        .filter(|word| word.kind == "word")
        .map(|word| {
            SourceRange::new(
                Tick::from_seconds_f64(word.start, time::Rounding::Nearest).unwrap(),
                Tick::from_seconds_f64(word.end, time::Rounding::Nearest).unwrap(),
            )
            .unwrap()
        })
        .collect();
    let language = captions::LanguageTag::parse("vi").unwrap();
    let range =
        SourceRange::new(words.first().unwrap().start(), words.last().unwrap().end()).unwrap();
    let mut data = Project::empty(ProjectId(0), "Interview".into())
        .unwrap()
        .into_data();
    data.assets.push(
        testkit::source::asset(7, std::env::temp_dir().join("interview-09.mp4"))
            .format("mp4")
            .duration(range.end())
            .content_hash(media::ContentHash::from_bytes(b"interview"))
            .build(),
    );
    data.scene_mut().timeline.insert_track(Track {
        id: TrackId(40),
        name: "Transcript".into(),
        kind: TrackKind::Text,
        muted: false,
        hidden: false,
        locked: false,
        items: vec![
            testkit::item::item(
                ItemId(40),
                ItemParams::Caption(project::CaptionParams {
                    emphasis: Default::default(),
                    word_highlight: false,
                    preset: None,
                    overrides: captions::CaptionOverrides::default(),
                    cues: captions::CueTrack {
                        styles: Vec::new(),
                        cues: vec![captions::Cue {
                            word_timing: crate::support::session::measured(
                                &language, &asr.text, &words,
                            ),
                            id: "interview-09".into(),
                            range,
                            source_lang: language.clone(),
                            texts: std::collections::BTreeMap::from([(
                                language,
                                captions::CueText {
                                    text: asr.text.clone().into(),
                                    edited: false,
                                    translation_ref: None,
                                },
                            )]),
                            style: None,
                        }],
                    },
                }),
            )
            .asset(media::AssetId(7))
            .source(range)
            .build(),
        ],
    });
    let mut s = Session::new(Project::new(data).unwrap(), 10).unwrap();
    let mut r = runner();
    let found = fillers(&mut s, &mut r, "vi", false, "interview-fillers");
    println!(
        "{:?}",
        found
            .iter()
            .map(|candidate| (
                candidate.id.as_str(),
                candidate.range.start.0,
                candidate.range.end.0
            ))
            .collect::<Vec<_>>()
    );
    assert!(
        found.is_empty(),
        "the excerpt has no ừm/ờ/ờm token; only the backchannel Dạ survives"
    );
    r.shutdown();
}

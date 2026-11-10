#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "public-interface conformance assertions"
)]
use crate::support::control::{
    Allow, finish, fixture_chapters, font_access, media_access, ok, request, runner, seed, submit,
    with_audio, with_loudness, with_source_audio, with_thumbnails, with_video, with_waveform,
    with_words, write_png, write_video, write_wav,
};
use engine::control::{self, Caller, ControlResult};
use engine::jobs::{JobState, SystemClock};
use engine::{ByteBudget, Command, Effect, Fixture, Session, registry};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

fn replace_paths(v: &mut Value, root: &std::path::Path) {
    match v {
        Value::String(s) if s == "$output" => {
            *s = root.to_str().unwrap().to_owned();
        }
        Value::String(s) if s.starts_with("$output/") => {
            *s = root
                .join(s.trim_start_matches("$output/"))
                .to_str()
                .unwrap()
                .to_owned()
        }
        Value::Object(o) => {
            for v in o.values_mut() {
                replace_paths(v, root)
            }
        }
        Value::Array(a) => {
            for v in a {
                replace_paths(v, root)
            }
        }
        _ => {}
    }
}
#[test]
fn registry_examples_validate_and_dispatch_without_an_id_table() {
    let root = tempfile::tempdir().unwrap();
    let wav = root.path().join("registry-waveform.wav");
    let png = root.path().join("registry-thumbnails.png");
    let video = root.path().join("registry-scenes.mkv");
    write_wav(&wav, 8_000);
    write_png(&png);
    write_video(&video);
    let media = Mutex::new(media_access(root.path()));
    let chapters = fixture_chapters();
    let mut exercised = std::collections::BTreeSet::new();
    for spec in registry() {
        let mut s = seed();
        let mut r = runner();
        let mut params = spec.example.params.clone();
        replace_paths(&mut params, root.path());
        let validator = jsonschema::options()
            .offline()
            .build(&spec.params_schema)
            .unwrap();
        assert!(
            validator.is_valid(&params),
            "{} schema rejects {}: {:?}",
            spec.id,
            params,
            validator.iter_errors(&params).collect::<Vec<_>>()
        );
        let typed: Command =
            serde_json::from_value(json!({"command":spec.id,"params":params})).unwrap();
        assert_eq!(typed.id(), spec.id);
        match spec.example.fixture {
            Fixture::TransitionCut => s = crate::session::transition_session(false),
            Fixture::Transition => s = crate::session::transition_session(true),
            Fixture::Audio => s = with_audio(&s),
            Fixture::SourceAudio => s = with_source_audio(&s),
            Fixture::Waveform => s = with_waveform(&s, &wav),
            Fixture::Thumbnails => s = with_thumbnails(&s, &png),
            Fixture::Video => s = with_video(&s, &video),
            Fixture::Loudness => s = with_loudness(&s, &wav),
            Fixture::Words => s = with_words(&s),
            Fixture::History => {
                s.apply(Command::RenameProject {
                    name: "History".into(),
                })
                .unwrap();
            }
            Fixture::Redo => {
                s.apply(Command::RenameProject {
                    name: "History".into(),
                })
                .unwrap();
                s.undo().unwrap();
            }
            Fixture::Gesture => {
                s.begin_gesture().unwrap();
            }
            Fixture::Job => {
                submit(&mut r);
            }
            Fixture::Library => {
                let mut library = engine::library::ProjectLibrary::open(
                    &root.path().join(format!("library-{}", spec.id)),
                    ByteBudget::new(64 << 20).unwrap(),
                )
                .unwrap();
                library
                    .execute(
                        project::ProjectId(1),
                        Command::CreateProject {
                            id: project::ProjectId(1),
                            name: "Example".into(),
                        },
                    )
                    .unwrap();
                library.execute(project::ProjectId(1), typed).unwrap();
                exercised.insert(spec.id);
                continue;
            }
            Fixture::Create => {
                assert!(Session::create(typed, 10).is_ok());
                exercised.insert(spec.id);
                continue;
            }
            Fixture::Save => {
                let mut autosave = engine::recovery::Autosave::new(
                    &root.path().join("recovery"),
                    s,
                    ByteBudget::new(512 << 20).unwrap(),
                    Arc::new(SystemClock::default()),
                    engine::recovery::Schedule::default(),
                )
                .unwrap();
                autosave.save_as(root.path().join("saved.vibecut")).unwrap();
                autosave.flush().unwrap();
                autosave.apply(typed).unwrap();
                autosave.flush().unwrap();
                exercised.insert(spec.id);
                continue;
            }
            _ => {}
        }
        let req = request(&s, spec.id, params, "example");
        let fonts = font_access();
        let result = s
            .control_request(
                req,
                Caller {
                    principal: "ui",
                    external: false,
                    policy: &Allow,
                    fonts: Some(&fonts),
                    media: Some(&media),
                    chapters: Some(&chapters),
                },
                &mut r,
            )
            .unwrap();
        let value = ok(result);
        if let ControlResult::JobAccepted { job, .. } = value {
            finish(&mut r, job);
            let snap = r.snapshot(job).unwrap();
            assert_eq!(
                snap.state,
                JobState::Completed,
                "{}: {:?}",
                spec.id,
                snap.error
            );
        }
        exercised.insert(spec.id);
    }
    assert_eq!(exercised, registry().iter().map(|s| s.id).collect());
}
#[test]
fn all_schema_shapes_agree_on_required_fields_wrong_types_and_closed_objects() {
    for spec in registry() {
        let schema = jsonschema::options()
            .offline()
            .build(&spec.params_schema)
            .unwrap();
        let params = &spec.example.params;
        assert!(schema.is_valid(params), "{}", spec.id);
        let mut extra = params.clone();
        extra
            .as_object_mut()
            .unwrap()
            .insert("untrusted-secret".into(), json!(true));
        assert!(!schema.is_valid(&extra), "{}", spec.id);
        assert!(
            serde_json::from_value::<Command>(json!({"command":spec.id,"params":extra})).is_err()
        );
        for field in spec.params_schema["required"]
            .as_array()
            .into_iter()
            .flatten()
        {
            let mut missing = params.clone();
            missing
                .as_object_mut()
                .unwrap()
                .remove(field.as_str().unwrap());
            assert!(!schema.is_valid(&missing), "{} missing {field}", spec.id);
            assert!(
                serde_json::from_value::<Command>(json!({"command":spec.id,"params":missing}))
                    .is_err(),
                "{} missing {field}",
                spec.id
            );
        }
    }
}
#[test]
fn deterministic_catalog_and_schema_document_receipts() {
    let first = serde_json::to_vec(registry()).unwrap();
    let second = serde_json::to_vec(registry()).unwrap();
    assert_eq!(first, second);
    assert_eq!(control::registry_hash().len(), 64);
    for spec in registry() {
        assert_eq!(
            spec.params_schema["$schema"],
            "https://json-schema.org/draft/2020-12/schema"
        );
        assert!(
            jsonschema::options()
                .offline()
                .build(&spec.output_schema)
                .is_ok()
        );
    }
    println!(
        "catalog_bytes={} registry_hash={}",
        first.len(),
        control::registry_hash()
    );
    println!(
        "representative_project_bytes={}",
        control::bounded_json(seed().project().data(), control::MAX_RESPONSE_BYTES)
            .unwrap()
            .len()
    );
    assert!(first.len() < control::MAX_RESPONSE_BYTES);
    let mut job_control: Vec<_> = registry()
        .iter()
        .filter(|s| s.effect == Effect::JobControl)
        .map(|s| s.id)
        .collect();
    job_control.sort_unstable();
    assert_eq!(
        job_control,
        [
            "audio.measure_loudness",
            "chapters.generate",
            "fillers.classify",
            "fonts.download",
            "job_cancel",
            "media.detect_scenes",
            "media.generate_clip_thumbnails",
            "media.generate_waveform",
            "preview",
            "timeline.pause_candidates"
        ]
    );
    // Each fact rides on its declaration line: a renamed id keeps its rule, and a
    // command without a grant class (or a publisher without a path) fails here.
    use engine::{Availability, GrantClass, Lifecycle};
    let ids = |pick: &dyn Fn(&engine::CommandSpec) -> bool| {
        let mut ids: Vec<_> = registry()
            .iter()
            .filter(|s| pick(s))
            .map(|s| s.id)
            .collect();
        ids.sort_unstable();
        ids
    };
    for (rule, expected) in [
        (
            Availability::CaptionItem,
            &[
                "burn_subtitles",
                "export_subtitles",
                "fillers.classify",
                "timeline.filler_candidates",
                "timeline.remove_fillers",
                "transcript.delete_range",
                "transcript.merge_cues",
            ][..],
        ),
        (
            Availability::NoTransaction,
            &["begin_gesture", "timeline.set_ripple"],
        ),
        (
            Availability::GestureActive,
            &[
                "cancel_gesture",
                "commit_gesture",
                "timeline.commit_preview",
                "timeline.discard_preview",
            ],
        ),
        (Availability::PlayableTimeline, &["preview.snapshot"]),
        (Availability::CanUndo, &["undo"]),
        (Availability::CanRedo, &["redo"]),
        (Availability::Assets, &["media.remove"]),
        (Availability::RetimableItem, &["timeline.set_speed"]),
        (Availability::RemovableTrack, &["track.remove"]),
        (
            Availability::VisualItem,
            &[
                "effects.add",
                "effects.remove",
                "effects.reorder",
                "effects.toggle",
                "effects.update_param",
                "masks.add",
                "masks.remove",
                "masks.reorder",
                "masks.update",
            ],
        ),
        (
            Availability::UnlockedItem,
            &[
                "audio.set_fade",
                "delete",
                "keyframes.add",
                "keyframes.delete_selection",
                "keyframes.remove",
                "keyframes.retime",
                "keyframes.scale_time",
                "keyframes.set_extrapolation",
                "keyframes.set_handle",
                "keyframes.set_param",
                "keyframes.toggle_at",
                "move_item",
                "move_items",
                "retranslate",
                "ripple_delete",
                "set_item_param",
                "split_at",
                "timeline.duplicate",
                "timeline.move_items",
                "timeline.set_hidden",
                "timeline.set_item_param",
                "timeline.set_muted",
                "timeline.split_left",
                "timeline.split_right",
                "timeline.trim",
                "trim_end",
                "trim_start",
            ],
        ),
        (
            Availability::VideoItem,
            &["timeline.extract_audio", "timeline.recover_audio"],
        ),
    ] {
        assert_eq!(ids(&|s| s.availability == rule), expected, "{rule:?}");
    }
    assert_eq!(
        ids(&|s| s.grant == Some(GrantClass::Human)),
        [
            "begin_gesture",
            "cancel_gesture",
            "captions.import",
            "commit_gesture",
            "fonts.download",
            "import.add_asset",
            "project.create",
            "project.delete",
            "project.duplicate",
            "project.thumbnail",
            "sounds.add_to_timeline",
            "timeline.commit_preview",
            "timeline.discard_preview",
            "timeline.preview",
        ]
    );
    assert_eq!(
        ids(&|s| s.grant == Some(GrantClass::Output)),
        [
            "burn_subtitles",
            "chapters.export_sidecars",
            "export",
            "export_subtitles",
            "preview.snapshot"
        ]
    );
    assert_eq!(ids(&|s| s.grant.is_none()), Vec::<&str>::new());
    assert_eq!(
        ids(&|s| s.curated),
        [
            "batch",
            "export",
            "inspect",
            "job_cancel",
            "job_status",
            "preview",
            "redo",
            "undo"
        ]
    );
    assert_eq!(
        ids(&|s| s.lifecycle.is_some()),
        [
            "project.create",
            "project.delete",
            "project.duplicate",
            "project.save",
            "project.thumbnail",
        ]
    );
    assert_eq!(engine::lifecycle("project.save"), Some(Lifecycle::Save));
    for spec in registry()
        .iter()
        .filter(|s| s.effect == engine::Effect::FilePublish)
    {
        let command: Command =
            serde_json::from_value(json!({"command":spec.id,"params":spec.example.params.clone()}))
                .unwrap();
        assert!(
            command.published_path().is_some(),
            "{} has no output path",
            spec.id
        );
    }
}
#[test]
fn checked_in_registry_digest_and_command_reference_match_the_registry() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    assert_eq!(
        std::fs::read_to_string(root.join("catalog.digest"))
            .unwrap()
            .replace("\r\n", "\n"),
        format!("{}\n", control::registry_hash()),
        "regenerate with cargo xtask catalog"
    );
    assert_eq!(
        std::fs::read_to_string(root.join("../../docs/product/agent-control/commands.md"))
            .unwrap()
            .replace("\r\n", "\n"),
        control::reference_markdown().unwrap(),
        "regenerate with cargo xtask catalog"
    );
}

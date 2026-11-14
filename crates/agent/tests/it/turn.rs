//! The turn loop over a scripted model: edits on a fork, refusals, jobs, limits, cancel.
use crate::support::{
    all_kinds, call, calls, conversation, conversation_with, drive, job_key, results, run_command,
    runner, say, then,
};
use agent::{AgentEvent, CancelFlag, EndReason, TurnEnd, TurnLimits, fixture, tool_specs};
use ai::text::conversation::{
    DataKind, ImageType, Message, ScriptedReply, ScriptedStep, ToolCall, ToolInput,
};
use engine::jobs::{JobBytes, JobKind, JobSpec, JobState, ResourceClass};
use engine::{Command, Session};
use serde_json::json;
use std::collections::BTreeSet;
use std::time::Duration;

fn has_track(session: &Session, name: &str) -> bool {
    session
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .any(|track| track.name.as_str() == name)
}

fn tool_names(events: &[AgentEvent]) -> Vec<&str> {
    events
        .iter()
        .filter_map(|event| match event {
            AgentEvent::ToolStarted { name, .. } => Some(name.as_str()),
            _ => None,
        })
        .collect()
}

#[test]
fn a_multi_step_edit_reads_its_own_result_and_the_live_project_waits_for_apply() {
    let mut session = fixture().unwrap();
    let before = session.project().clone();
    let mut conversation = conversation(vec![
        calls(vec![run_command(
            "c1",
            "timeline.add_track",
            json!({"name": "B-roll", "kind": "video"}),
        )]),
        calls(vec![call("c2", "project_summary", json!({}))]),
        // The model reads the track id the first edit created from its own summary.
        then(|read| {
            let tracks = read[0]["tracks"].as_array().unwrap();
            let track = tracks.iter().find(|t| t["name"] == "B-roll").unwrap();
            vec![run_command(
                "c3",
                "track.rename",
                json!({"track": track["id"], "name": "Cutaways"}),
            )]
        }),
        say("Added a track and named it Cutaways."),
    ]);
    let mut runner = runner();
    let (turn, events) = drive(
        &session,
        &mut conversation,
        &mut runner,
        TurnLimits::default(),
        &CancelFlag::new(),
        |_| {},
    );

    assert_eq!(turn.end, TurnEnd::Done);
    assert_eq!(
        tool_names(&events),
        ["command_run", "project_summary", "command_run"]
    );
    assert!(events.contains(&AgentEvent::TextDelta {
        text: "Added a track and named it Cutaways.".into()
    }));
    assert!(matches!(events.last(), Some(AgentEvent::TurnDone { .. })));
    assert!(
        events
            .iter()
            .filter(|event| matches!(event, AgentEvent::ToolFinished { .. }))
            .all(|event| matches!(event, AgentEvent::ToolFinished { ok: true, .. }))
    );
    let logged: Vec<&str> = turn.fork.commands().map(|command| command.id()).collect();
    assert_eq!(logged, ["timeline.add_track", "track.rename"]);
    assert!(has_track(turn.fork.session(), "Cutaways"));
    assert!(
        !has_track(&session, "Cutaways"),
        "the fork is not the live project"
    );
    assert_eq!(**session.project(), *before);

    session.apply_fork(turn.fork).unwrap();
    assert!(has_track(&session, "Cutaways"));
}

#[test]
fn refused_commands_change_nothing_and_say_why() {
    let session = fixture().unwrap();
    let mut conversation = conversation(vec![
        calls(vec![
            run_command("e1", "export", json!({})),
            run_command(
                "b1",
                "batch",
                json!({"commands": [
                    {"command": "project.rename", "params": {"name": "Sneaky"}},
                    {"command": "project.save", "params": {}},
                ]}),
            ),
            run_command("u1", "undo", json!({})),
        ]),
        say("I could not do that."),
    ]);
    let mut runner = runner();
    let (turn, _) = drive(
        &session,
        &mut conversation,
        &mut runner,
        TurnLimits::default(),
        &CancelFlag::new(),
        |_| {},
    );

    assert_eq!(turn.end, TurnEnd::Done);
    assert_eq!(turn.refusals, 3);
    assert!(turn.fork.is_empty());
    assert_eq!(
        turn.fork.session().project().data().name.as_str(),
        "Evaluation"
    );
    let held = results(&conversation);
    let reasons: Vec<(&str, &str)> = held
        .iter()
        .map(|(content, is_error)| {
            assert!(is_error);
            (
                content["error"].as_str().unwrap(),
                content["reason"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        reasons,
        [
            ("refused", "export"),
            ("refused", "save"),
            ("refused", "history")
        ]
    );
}

#[test]
fn malformed_unknown_and_extra_field_calls_come_back_as_errors_before_any_effect() {
    let session = fixture().unwrap();
    let mut conversation = conversation(vec![
        ScriptedStep::Reply(ScriptedReply {
            text: String::new(),
            calls: vec![
                ToolCall {
                    id: "m1".into(),
                    name: "command_run".into(),
                    input: ToolInput::Malformed,
                },
                call("n1", "no_such_tool", json!({})),
                run_command("x1", "project.rename", json!({"name": "A", "extra": 1})),
                call(
                    "x2",
                    "command_run",
                    json!({"command": "project.rename", "params": {"name": "A"}, "base_rev": "x"}),
                ),
                run_command("x3", "nope.nothing", json!({})),
            ],
        }),
        say("Understood."),
    ]);
    let mut runner = runner();
    let (turn, _) = drive(
        &session,
        &mut conversation,
        &mut runner,
        TurnLimits::default(),
        &CancelFlag::new(),
        |_| {},
    );

    assert_eq!(turn.end, TurnEnd::Done);
    assert!(turn.fork.is_empty());
    let held = results(&conversation);
    assert_eq!(held.len(), 5);
    assert!(held.iter().all(|(_, is_error)| *is_error));
    assert_eq!(held[2].0["error"], "invalid_params");
    assert_eq!(held[3].0["error"], "invalid_input");
    assert_eq!(held[4].0["reason"], "unlisted");
}

#[test]
fn a_job_on_the_fork_reports_progress_until_it_completes() {
    let session = fixture().unwrap();
    let mut runner = runner();
    let (job, _) = runner
        .submit::<engine::control::Error>(
            JobSpec {
                kind: JobKind::GenerateProxy,
                class: ResourceClass::Cpu,
                input: job_key(),
                bytes: JobBytes::default(),
            },
            |context| {
                for _ in 0..10 {
                    context.check_cancelled()?;
                    std::thread::sleep(Duration::from_millis(5));
                }
                Ok(Vec::new())
            },
        )
        .unwrap();
    let mut conversation = conversation(vec![
        calls(vec![call(
            "j1",
            "job_status",
            json!({"job": job.0, "wait_ms": 10_000}),
        )]),
        say("The job finished."),
    ]);
    let (turn, events) = drive(
        &session,
        &mut conversation,
        &mut runner,
        TurnLimits::default(),
        &CancelFlag::new(),
        |_| {},
    );

    assert_eq!(turn.end, TurnEnd::Done);
    assert!(events.iter().any(|event| matches!(
        event,
        AgentEvent::JobProgress { state, .. } if state == "completed"
    )));
    let held = results(&conversation);
    assert_eq!(held[0].0["state"], "completed");
    assert!(!held[0].1);
    runner.shutdown();
}

#[test]
fn the_step_limit_ends_the_turn_and_frees_the_conversation() {
    let session = fixture().unwrap();
    let step = || calls(vec![call("s", "project_summary", json!({}))]);
    let mut conversation = conversation(vec![step(), step(), step(), step()]);
    let mut runner = runner();
    let (turn, events) = drive(
        &session,
        &mut conversation,
        &mut runner,
        TurnLimits { max_steps: 2 },
        &CancelFlag::new(),
        |_| {},
    );

    assert_eq!(turn.end, TurnEnd::Stopped(EndReason::StepLimit));
    assert_eq!(
        events.last(),
        Some(&AgentEvent::TurnError {
            reason: EndReason::StepLimit
        })
    );
    assert_eq!(tool_names(&events).len(), 2);
    assert!(conversation.history().is_empty(), "the next turn can start");
    assert!(turn.fork.is_empty());
}

#[test]
fn cancel_during_a_tool_finishes_that_edit_and_stops_before_the_next_request() {
    let session = fixture().unwrap();
    let mut conversation = conversation(vec![
        calls(vec![run_command(
            "c1",
            "timeline.add_track",
            json!({"name": "Late", "kind": "video"}),
        )]),
        say("This reply is never requested."),
    ]);
    let mut runner = runner();
    let cancel = CancelFlag::new();
    let (turn, events) = drive(
        &session,
        &mut conversation,
        &mut runner,
        TurnLimits::default(),
        &cancel,
        |event| {
            if matches!(event, AgentEvent::ToolStarted { .. }) {
                cancel.cancel();
            }
        },
    );

    assert_eq!(turn.end, TurnEnd::Stopped(EndReason::Cancelled));
    assert!(
        has_track(turn.fork.session(), "Late"),
        "the running tool completed"
    );
    assert!(
        !turn.fork.is_empty(),
        "the user may still inspect or apply the fork"
    );
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, AgentEvent::TextDelta { .. }))
    );
    assert!(matches!(
        events.last(),
        Some(AgentEvent::TurnError {
            reason: EndReason::Cancelled
        })
    ));
    assert!(conversation.history().is_empty());
}

#[test]
fn cancel_during_a_job_stops_the_job() {
    let session = fixture().unwrap();
    let mut runner = runner();
    let (job, _) = runner
        .submit::<engine::control::Error>(
            JobSpec {
                kind: JobKind::GenerateProxy,
                class: ResourceClass::Cpu,
                input: job_key(),
                bytes: JobBytes::default(),
            },
            |context| loop {
                context.check_cancelled()?;
                std::thread::sleep(Duration::from_millis(2));
            },
        )
        .unwrap();
    let mut conversation = conversation(vec![
        calls(vec![call(
            "j1",
            "job_status",
            json!({"job": job.0, "wait_ms": 30_000}),
        )]),
        say("This reply is never requested."),
    ]);
    let cancel = CancelFlag::new();
    let (turn, _) = drive(
        &session,
        &mut conversation,
        &mut runner,
        TurnLimits::default(),
        &cancel,
        |event| {
            if matches!(event, AgentEvent::JobProgress { .. }) {
                cancel.cancel();
            }
        },
    );

    assert_eq!(turn.end, TurnEnd::Stopped(EndReason::Cancelled));
    assert_eq!(runner.snapshot(job).unwrap().state, JobState::Cancelled);
    runner.shutdown();
}

#[test]
fn a_cancel_before_the_first_request_sends_nothing() {
    let session = fixture().unwrap();
    let mut conversation = conversation(vec![say("never")]);
    let mut runner = runner();
    let cancel = CancelFlag::new();
    cancel.cancel();
    let (turn, events) = drive(
        &session,
        &mut conversation,
        &mut runner,
        TurnLimits::default(),
        &cancel,
        |_| {},
    );

    assert_eq!(turn.end, TurnEnd::Stopped(EndReason::Cancelled));
    assert_eq!(events.len(), 1);
}

#[test]
fn a_request_that_could_pass_the_spend_limit_is_not_sent() {
    let session = fixture().unwrap();
    let mut conversation = conversation_with(vec![say("never")], all_kinds(), 1);
    let mut runner = runner();
    let (turn, events) = drive(
        &session,
        &mut conversation,
        &mut runner,
        TurnLimits::default(),
        &CancelFlag::new(),
        |_| {},
    );

    assert_eq!(turn.end, TurnEnd::Stopped(EndReason::SpendLimit));
    assert_eq!(conversation.ledger().spent_micros(), 0);
    assert_eq!(events.len(), 1);
}

#[test]
fn applying_after_a_live_edit_refuses_and_changes_nothing() {
    let mut session = fixture().unwrap();
    let mut conversation = conversation(vec![
        calls(vec![run_command(
            "c1",
            "timeline.add_track",
            json!({"name": "Stale", "kind": "video"}),
        )]),
        say("Done."),
    ]);
    let mut runner = runner();
    let (turn, _) = drive(
        &session,
        &mut conversation,
        &mut runner,
        TurnLimits::default(),
        &CancelFlag::new(),
        |_| {},
    );
    assert_eq!(turn.end, TurnEnd::Done);

    session
        .apply(Command::RenameProject {
            name: "Edited".into(),
        })
        .unwrap();
    let live = session.project().clone();
    assert!(session.apply_fork(turn.fork).is_err());
    assert_eq!(**session.project(), *live);
    assert!(!has_track(&session, "Stale"));
}

#[test]
fn data_the_conversation_does_not_share_is_withheld_from_the_model() {
    let session = fixture().unwrap();
    let kinds: BTreeSet<DataKind> = [DataKind::ProjectSummary].into_iter().collect();
    let mut conversation = conversation_with(
        vec![
            calls(vec![call("t1", "transcript_read", json!({}))]),
            say("Understood."),
        ],
        kinds,
        5_000_000,
    );
    let mut runner = runner();
    let (turn, _) = drive(
        &session,
        &mut conversation,
        &mut runner,
        TurnLimits::default(),
        &CancelFlag::new(),
        |_| {},
    );

    assert_eq!(turn.end, TurnEnd::Done);
    let held = results(&conversation);
    assert!(held[0].1);
    assert_eq!(held[0].0["error"], "not_shared");
}

fn tool_results(
    conversation: &ai::text::conversation::Conversation,
) -> Vec<ai::text::conversation::ToolResult> {
    conversation
        .history()
        .iter()
        .filter_map(|message| match message {
            Message::ToolResults(results) => Some(results.clone()),
            _ => None,
        })
        .flatten()
        .collect()
}

#[test]
fn preview_frame_shows_the_model_one_frame_of_the_fork_and_refuses_bad_input() {
    let session = fixture().unwrap();
    let mut kinds = all_kinds();
    kinds.insert(DataKind::Frames);
    let mut conversation = conversation_with(
        vec![
            calls(vec![
                call("f1", "preview_frame", json!({"frame": 0})),
                call("f2", "preview_frame", json!({})),
                call("f3", "preview_frame", json!({"frame": 0, "extra": 1})),
                call("f4", "preview_frame", json!({"frame": 1_000_000})),
                run_command("f5", "preview", json!({"frames": [0]})),
            ]),
            say("I saw the frame."),
        ],
        kinds,
        5_000_000,
    );
    let mut runner = runner();
    let (turn, events) = drive(
        &session,
        &mut conversation,
        &mut runner,
        TurnLimits::default(),
        &CancelFlag::new(),
        |_| {},
    );

    assert_eq!(turn.end, TurnEnd::Done);
    assert_eq!(turn.refusals, 1, "preview through command_run is refused");
    assert!(turn.fork.is_empty(), "a frame is not an edit");
    assert!(events.iter().any(|event| matches!(
        event,
        AgentEvent::ToolFinished { id, ok: true, .. } if id == "f1"
    )));
    let held = tool_results(&conversation);
    assert_eq!(held.len(), 5);
    let shown = &held[0];
    assert!(!shown.is_error, "{}", shown.content);
    assert!(shown.kinds.contains(&DataKind::Frames));
    assert_eq!(shown.images.len(), 1);
    assert_eq!(shown.images[0].image_type(), ImageType::Jpeg);
    let described: serde_json::Value = serde_json::from_str(&shown.content).unwrap();
    assert_eq!(described["frame"], 0);
    assert!(described["width"].as_u64().unwrap() <= 640);
    assert!(!described["clips"].as_array().unwrap().is_empty());
    for refused in &held[1..] {
        assert!(refused.is_error);
        assert!(refused.images.is_empty());
        assert!(refused.kinds.is_empty());
    }
    let codes: Vec<String> = held[1..4]
        .iter()
        .map(|r| {
            let value: serde_json::Value = serde_json::from_str(&r.content).unwrap();
            value["error"].as_str().unwrap().to_owned()
        })
        .collect();
    assert_eq!(codes[0], "invalid_input");
    assert_eq!(codes[1], "invalid_input");
    assert!(
        codes[2] == "invalid_params" || codes[2] == "limit",
        "{codes:?}"
    );
    let reason: serde_json::Value = serde_json::from_str(&held[4].content).unwrap();
    assert_eq!(reason["reason"], "use_tool");
    runner.shutdown();
}

#[test]
fn preview_frame_is_offered_only_when_frames_are_shared() {
    let names = |frames: bool| -> Vec<String> {
        tool_specs(frames)
            .into_iter()
            .map(|spec| spec.name)
            .collect()
    };
    assert!(!names(false).contains(&"preview_frame".to_owned()));
    assert!(names(true).contains(&"preview_frame".to_owned()));
    for frames in [false, true] {
        let specs = tool_specs(frames);
        let run = specs
            .iter()
            .find(|spec| spec.name == "command_run")
            .unwrap();
        let offered = run.input_schema["properties"]["command"]["enum"]
            .as_array()
            .unwrap();
        assert!(
            !offered.iter().any(|id| id == "preview"),
            "command_run never runs preview; preview_frame does"
        );
    }

    // A conversation that does not share frames never offers the tool, so the call is
    // answered as an unknown tool and nothing is rendered.
    let session = fixture().unwrap();
    let mut conversation = conversation(vec![
        calls(vec![call("f1", "preview_frame", json!({"frame": 0}))]),
        say("No frame."),
    ]);
    let mut runner = runner();
    let (turn, _) = drive(
        &session,
        &mut conversation,
        &mut runner,
        TurnLimits::default(),
        &CancelFlag::new(),
        |_| {},
    );
    assert_eq!(turn.end, TurnEnd::Done);
    let held = tool_results(&conversation);
    assert!(held[0].is_error && held[0].images.is_empty());
    runner.shutdown();
}

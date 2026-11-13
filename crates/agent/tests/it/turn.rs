//! The turn loop over a scripted model: edits on a fork, refusals, jobs, limits, cancel.
use crate::support::{
    all_kinds, call, calls, conversation, conversation_with, drive, job_key, results, run_command,
    runner, say, then,
};
use agent::{AgentEvent, CancelFlag, EndReason, TurnEnd, TurnLimits, fixture};
use ai::text::conversation::{DataKind, ScriptedReply, ScriptedStep, ToolCall, ToolInput};
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

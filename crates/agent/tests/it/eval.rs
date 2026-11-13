//! The evaluation set scored with a scripted model per case.
use crate::support::{call, calls, conversation, run_command, runner, say, then};
use agent::{CASES, Case, Setup, TurnEnd, evaluate, fixture};
use ai::text::conversation::ScriptedStep;
use serde_json::{Value, json};

/// Item ids of a `timeline_read` result with their start ticks, earliest start first.
fn clips(read: &Value) -> Vec<(u64, i64)> {
    let mut clips: Vec<(u64, i64)> = read["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| {
            (
                item["id"].as_u64().unwrap(),
                item["start"].as_i64().unwrap(),
            )
        })
        .collect();
    clips.sort_by_key(|(_, start)| *start);
    clips
}

/// What a model that does each case correctly would say and call.
fn script(name: &str) -> Vec<ScriptedStep> {
    let read = || calls(vec![call("r1", "timeline_read", json!({}))]);
    match name {
        "rename" => vec![
            calls(vec![run_command(
                "c1",
                "project.rename",
                json!({"name": "Launch Day"}),
            )]),
            say("Renamed."),
        ],
        "add_track" => vec![
            calls(vec![run_command(
                "c1",
                "timeline.add_track",
                json!({"name": "B-roll", "kind": "video"}),
            )]),
            say("Added."),
        ],
        "delete_last" => vec![
            read(),
            then(|results| {
                let (last, _) = *clips(&results[0]).last().unwrap();
                vec![run_command("c1", "delete", json!({"items": [last]}))]
            }),
            say("Deleted."),
        ],
        "hide_first" => vec![
            read(),
            then(|results| {
                let (first, _) = clips(&results[0])[0];
                vec![run_command(
                    "c1",
                    "timeline.set_hidden",
                    json!({"items": [first], "hidden": true}),
                )]
            }),
            say("Hidden."),
        ],
        "split_first" => vec![
            read(),
            then(|results| {
                let (first, start) = clips(&results[0])[0];
                vec![run_command(
                    "c1",
                    "split_at",
                    json!({"item": first, "at": start + time::TICKS_PER_SECOND}),
                )]
            }),
            say("Split."),
        ],
        "export_refused" => vec![
            calls(vec![run_command("c1", "export", json!({}))]),
            say("I cannot export; please export from the Export button."),
        ],
        _ => vec![read(), say("Two clips.")],
    }
}

#[test]
fn the_fixed_set_scores_every_case_with_a_correct_scripted_model() {
    let mut setup = |case: &Case| -> Result<Setup, agent::Error> {
        Ok(Setup {
            session: fixture()?,
            conversation: conversation(script(case.name)),
            runner: runner(),
        })
    };
    let report = evaluate(CASES, &mut setup).unwrap();

    for case in &report.cases {
        assert_eq!(case.end, TurnEnd::Done, "{}", case.name);
        assert!(case.commands_correct, "{}: {:?}", case.name, case.commands);
        assert!(case.summary_correct, "{}", case.name);
    }
    assert_eq!(report.cases.len(), CASES.len());
    assert_eq!(report.commands_correct(), CASES.len());
    assert_eq!(report.summaries_correct(), CASES.len());
    assert_eq!(report.no_ops(), 0);
    assert_eq!(report.refusals(), 1, "the export prompt is the one refusal");
}

#[test]
fn a_model_that_does_nothing_is_counted_as_no_ops_and_wrong_commands_are_flagged() {
    let mut setup = |case: &Case| -> Result<Setup, agent::Error> {
        let steps = if case.name == "rename" {
            // Runs a command the case does not accept.
            vec![
                calls(vec![run_command(
                    "c1",
                    "timeline.add_track",
                    json!({"name": "Wrong", "kind": "video"}),
                )]),
                say("Done."),
            ]
        } else {
            vec![say("I did nothing.")]
        };
        Ok(Setup {
            session: fixture()?,
            conversation: conversation(steps),
            runner: runner(),
        })
    };
    let report = evaluate(CASES, &mut setup).unwrap();

    let rename = &report.cases[0];
    assert!(!rename.commands_correct);
    assert!(!rename.no_op);
    // Four of the five cases that expect a change got none (the rename got a wrong one);
    // the two cases that expect none are right.
    assert_eq!(report.no_ops(), 4);
    assert_eq!(report.commands_correct(), 2);
    assert_eq!(report.refusals(), 0);
}

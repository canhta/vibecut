//! Scores the agent against a fixed list of prompts with the changes each should make.
use crate::cancel::CancelFlag;
use crate::error::Error;
use crate::turn::{TurnEnd, TurnLimits, TurnRequest, run_turn};
use ai::text::conversation::Conversation;
use engine::Session;
use engine::control::run_wire;
use engine::jobs::JobRunner;
use project::{ItemId, Project, ProjectId};
use serde_json::json;
use std::collections::BTreeSet;

/// What a prompt should make the agent do. Counts are items of the active scene, taken from
/// the fork's change summary; the commands are the ids the fork logged.
#[derive(Clone, Copy, Debug)]
pub struct Expect {
    /// Command ids the turn may log; each logged command must be one of them. Empty means
    /// the prompt must change nothing.
    pub accepted: &'static [&'static str],
    pub added: usize,
    pub removed: usize,
    /// Items that kept their identity and were moved, trimmed, retimed or changed.
    pub touched: usize,
}

#[derive(Clone, Copy, Debug)]
pub struct Case {
    pub name: &'static str,
    pub prompt: &'static str,
    pub expect: Expect,
}

/// Two seconds of ticks, the length of each fixture clip.
const CLIP_TICKS: i64 = 2 * time::TICKS_PER_SECOND;

/// The fixed set, written for [`fixture`].
pub const CASES: &[Case] = &[
    Case {
        name: "rename",
        prompt: "Rename the project to Launch Day.",
        expect: Expect {
            accepted: &["project.rename"],
            added: 0,
            removed: 0,
            touched: 0,
        },
    },
    Case {
        name: "add_track",
        prompt: "Add a new video track called B-roll.",
        expect: Expect {
            accepted: &["timeline.add_track"],
            added: 0,
            removed: 0,
            touched: 0,
        },
    },
    Case {
        name: "delete_last",
        prompt: "Delete the last clip on the timeline.",
        expect: Expect {
            accepted: &["delete", "ripple_delete"],
            added: 0,
            removed: 1,
            touched: 0,
        },
    },
    Case {
        name: "hide_first",
        prompt: "Hide the first clip.",
        expect: Expect {
            accepted: &[
                "timeline.set_hidden",
                "timeline.set_item_param",
                "timeline.set_item_param_many",
                "set_item_param",
            ],
            added: 0,
            removed: 0,
            touched: 1,
        },
    },
    Case {
        name: "split_first",
        prompt: "Split the first clip in the middle.",
        expect: Expect {
            accepted: &["split_at", "timeline.split_all"],
            added: 1,
            removed: 0,
            touched: 1,
        },
    },
    Case {
        name: "export_refused",
        prompt: "Export the finished video to my desktop.",
        expect: Expect {
            accepted: &[],
            added: 0,
            removed: 0,
            touched: 0,
        },
    },
    Case {
        name: "question",
        prompt: "How many clips are on the timeline?",
        expect: Expect {
            accepted: &[],
            added: 0,
            removed: 0,
            touched: 0,
        },
    },
];

/// The project every case starts from: two red overlay clips of two seconds, the second
/// starting where the first ends.
pub fn fixture() -> Result<Session, Error> {
    let mut session = Session::new(Project::empty(ProjectId(1), "Evaluation".into())?, 10)?;
    for start in [0_i64, CLIP_TICKS] {
        run_wire(
            &mut session,
            "timeline.insert",
            json!({
                "placement": {"mode": "auto"},
                "item": {
                    "animations": {},
                    "source_asset": null,
                    "source_range": {"start": 0, "end": CLIP_TICKS},
                    "start": start,
                    "content": {"kind": "overlay", "params": {"color": [255, 0, 0, 255]}},
                },
            }),
        )?;
    }
    Ok(session)
}

/// What one case runs on: its session, a conversation over the model under test and a runner.
pub struct Setup {
    pub session: Session,
    pub conversation: Conversation,
    pub runner: JobRunner,
}

/// The outcome of one case.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaseReport {
    pub name: &'static str,
    pub commands: Vec<String>,
    /// Every logged command is accepted, and none was logged when none was expected.
    pub commands_correct: bool,
    /// Added, removed and touched counts equal the expected ones.
    pub summary_correct: bool,
    /// Changes were expected and the fork is empty.
    pub no_op: bool,
    pub refusals: usize,
    pub end: TurnEnd,
}

/// The score of a run. It reports; no threshold is applied to a provider.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    pub cases: Vec<CaseReport>,
}
impl Report {
    pub fn commands_correct(&self) -> usize {
        self.cases
            .iter()
            .filter(|case| case.commands_correct)
            .count()
    }
    pub fn summaries_correct(&self) -> usize {
        self.cases
            .iter()
            .filter(|case| case.summary_correct)
            .count()
    }
    pub fn no_ops(&self) -> usize {
        self.cases.iter().filter(|case| case.no_op).count()
    }
    pub fn refusals(&self) -> usize {
        self.cases.iter().map(|case| case.refusals).sum()
    }
}

/// Run every case on a fresh [`Setup`] and score the fork it leaves. A real provider is
/// scored by passing a `setup` that builds its conversation; nothing here gates on accuracy.
pub fn evaluate(
    cases: &[Case],
    setup: &mut dyn FnMut(&Case) -> Result<Setup, Error>,
) -> Result<Report, Error> {
    let mut reports = Vec::with_capacity(cases.len());
    for case in cases {
        let Setup {
            session,
            mut conversation,
            mut runner,
        } = setup(case)?;
        let turn = run_turn(
            TurnRequest {
                session: &session,
                prompt: case.prompt,
                conversation: &mut conversation,
                runner: &mut runner,
                limits: TurnLimits::default(),
                cancel: CancelFlag::new(),
            },
            &mut |_| {},
        )?;
        runner.shutdown();
        let commands: Vec<String> = turn
            .fork
            .commands()
            .map(|command| command.id().to_owned())
            .collect();
        let summary = turn.fork.summary();
        let touched: BTreeSet<ItemId> = summary
            .moved
            .iter()
            .chain(&summary.trimmed)
            .chain(&summary.retimed)
            .chain(&summary.params_changed)
            .chain(&summary.captions_changed)
            .copied()
            .collect();
        let expect = case.expect;
        let commands_correct = if expect.accepted.is_empty() {
            commands.is_empty()
        } else {
            !commands.is_empty()
                && commands
                    .iter()
                    .all(|id| expect.accepted.contains(&id.as_str()))
        };
        reports.push(CaseReport {
            name: case.name,
            commands_correct,
            summary_correct: summary.added.len() == expect.added
                && summary.removed.len() == expect.removed
                && touched.len() == expect.touched,
            no_op: !expect.accepted.is_empty() && turn.fork.is_empty(),
            commands,
            refusals: turn.refusals,
            end: turn.end,
        });
    }
    Ok(Report { cases: reports })
}

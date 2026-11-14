//! Folds the agent's event stream into the chat transcript the panel shows and keeps.
use agent::{AgentEvent, EndReason};
use engine::ChangeSummary;
use serde::{Deserialize, Serialize};

/// What the creator decided about a finished turn's changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Decision {
    Applied,
    Discarded,
}

/// A change summary reduced to counts, which is all the history keeps of a card.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Counts {
    pub(super) added: u32,
    pub(super) removed: u32,
    pub(super) moved: u32,
    pub(super) trimmed: u32,
    pub(super) retimed: u32,
    pub(super) edited: u32,
    pub(super) captions: u32,
    /// Timeline length before and after, in ticks.
    pub(super) before: i64,
    pub(super) after: i64,
}

fn count(items: &[project::ItemId]) -> u32 {
    u32::try_from(items.len()).unwrap_or(u32::MAX)
}

impl Counts {
    pub(super) fn of(summary: &ChangeSummary) -> Self {
        Self {
            added: count(&summary.added),
            removed: count(&summary.removed),
            moved: count(&summary.moved),
            trimmed: count(&summary.trimmed),
            retimed: count(&summary.retimed),
            edited: count(&summary.params_changed),
            captions: count(&summary.captions_changed),
            before: summary.duration_before.0,
            after: summary.duration_after.0,
        }
    }
}

/// One line of the chat, in the order it happened.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(super) enum Entry {
    User {
        text: String,
    },
    Assistant {
        text: String,
    },
    /// One tool step; `ok` is `None` while it runs.
    Step {
        id: String,
        name: String,
        ok: Option<bool>,
    },
    Progress {
        job: u64,
        percent: u8,
    },
    /// Why a turn ended early, as `end_tag` spells it.
    Ended {
        reason: String,
    },
    Changes {
        counts: Counts,
        decision: Decision,
    },
}

/// The stable tag of an end reason, kept in the history.
pub(super) fn end_tag(reason: EndReason) -> String {
    match reason {
        EndReason::Cancelled => "cancelled".to_owned(),
        EndReason::StepLimit => "step_limit".to_owned(),
        EndReason::SpendLimit => "spend_limit".to_owned(),
        EndReason::ContextLimit => "context_limit".to_owned(),
        EndReason::ConsentRequired => "consent_required".to_owned(),
        EndReason::Provider(category) => format!("provider:{category}"),
    }
}

/// Add what one event says to the transcript.
pub(super) fn fold(entries: &mut Vec<Entry>, event: &AgentEvent) {
    match event {
        AgentEvent::TextDelta { text } => match entries.last_mut() {
            Some(Entry::Assistant { text: existing }) => existing.push_str(text),
            _ => entries.push(Entry::Assistant { text: text.clone() }),
        },
        AgentEvent::ToolStarted { id, name } => entries.push(Entry::Step {
            id: id.clone(),
            name: name.clone(),
            ok: None,
        }),
        AgentEvent::ToolFinished { id, ok, .. } => {
            let at = entries
                .iter()
                .rposition(|entry| matches!(entry, Entry::Step { id: step, .. } if step == id));
            if let Some(Entry::Step { ok: slot, .. }) = at.and_then(|at| entries.get_mut(at)) {
                *slot = Some(*ok);
            }
        }
        AgentEvent::JobProgress { job, percent, .. } => {
            let at = entries.iter().rposition(
                |entry| matches!(entry, Entry::Progress { job: known, .. } if known == job),
            );
            match at.and_then(|at| entries.get_mut(at)) {
                Some(Entry::Progress { percent: known, .. }) => *known = *percent,
                _ => entries.push(Entry::Progress {
                    job: *job,
                    percent: *percent,
                }),
            }
        }
        AgentEvent::TurnDone { .. } => {}
        AgentEvent::TurnError { reason } => entries.push(Entry::Ended {
            reason: end_tag(*reason),
        }),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn events_fold_into_one_line_per_step_with_its_outcome() {
        let mut entries = Vec::new();
        let events = [
            AgentEvent::TextDelta {
                text: "Look".into(),
            },
            AgentEvent::TextDelta { text: "ing".into() },
            AgentEvent::ToolStarted {
                id: "a".into(),
                name: "project_summary".into(),
            },
            AgentEvent::ToolStarted {
                id: "b".into(),
                name: "command_run".into(),
            },
            AgentEvent::ToolFinished {
                id: "b".into(),
                name: "command_run".into(),
                ok: false,
            },
            AgentEvent::ToolFinished {
                id: "a".into(),
                name: "project_summary".into(),
                ok: true,
            },
            AgentEvent::JobProgress {
                job: 7,
                state: "running".into(),
                percent: 10,
            },
            AgentEvent::JobProgress {
                job: 7,
                state: "running".into(),
                percent: 60,
            },
            AgentEvent::TurnError {
                reason: EndReason::Provider("auth"),
            },
        ];
        for event in &events {
            fold(&mut entries, event);
        }
        assert_eq!(
            entries,
            [
                Entry::Assistant {
                    text: "Looking".into()
                },
                Entry::Step {
                    id: "a".into(),
                    name: "project_summary".into(),
                    ok: Some(true)
                },
                Entry::Step {
                    id: "b".into(),
                    name: "command_run".into(),
                    ok: Some(false)
                },
                Entry::Progress {
                    job: 7,
                    percent: 60
                },
                Entry::Ended {
                    reason: "provider:auth".into()
                },
            ]
        );
    }
}

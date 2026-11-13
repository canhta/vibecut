//! Defines the event stream a turn yields, with the typed reasons it ends early.
use engine::ChangeSummary;
use serde::Serialize;

/// Why a turn ended without finishing its work. The fork survives every reason: the user
/// may inspect, apply or discard it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EndReason {
    /// The user cancelled; the stream stopped and the fork's jobs were asked to stop.
    Cancelled,
    /// The turn used its steps and the model still wanted tools.
    StepLimit,
    /// The next request could pass the conversation's spend limit and was not sent.
    SpendLimit,
    /// The next request would pass the context ceiling and was not sent.
    ContextLimit,
    /// The conversation's consent does not cover what the model asked for.
    ConsentRequired,
    /// The provider refused or failed; the tag is the safe category.
    Provider(&'static str),
}

/// One item of the stream a turn yields, in order.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum AgentEvent {
    TextDelta {
        text: String,
    },
    ToolStarted {
        id: String,
        name: String,
    },
    ToolFinished {
        id: String,
        name: String,
        ok: bool,
    },
    JobProgress {
        job: u64,
        state: String,
        percent: u8,
    },
    /// The model finished; `changes` is what the fork's edits did to the project it forked.
    TurnDone {
        changes: ChangeSummary,
    },
    TurnError {
        reason: EndReason,
    },
}

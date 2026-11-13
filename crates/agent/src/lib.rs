//! Runs the in-app agent's turns against a fork of the live session.
mod cancel;
mod error;
mod eval;
mod event;
mod policy;
mod provider;
mod reads;
mod tools;
mod turn;

pub use cancel::CancelFlag;
pub use error::Error;
pub use eval::{CASES, Case, CaseReport, Expect, Report, Setup, evaluate, fixture};
pub use event::{AgentEvent, EndReason};
pub use policy::{AgentPolicy, Reason, TABLE, Verdict, verdict};
pub use provider::{Provider, open_model};
pub use tools::{PRINCIPAL, tool_specs};
pub use turn::{Turn, TurnEnd, TurnLimits, TurnRequest, new_conversation, run_turn};

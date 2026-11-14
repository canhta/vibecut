//! Hosts the agent chat docked beside the timeline: consent, turns on a fork, changes card.
mod backend;
mod history;
mod mention;
mod state;
mod transcript;
mod turn;
mod view;

pub use backend::{Backend, Environment};
pub use state::AgentView;
pub(in crate::editor) use state::{Action, State};
pub(in crate::editor) use view::show;

use crate::editor::Port;
use state::Applied;

/// Apply one request of the panel. The panel owns its state; only the live session and the
/// preview resync come from the port.
pub(in crate::editor) fn apply(state: &mut State, port: &mut Port<'_>, action: Action) {
    match action {
        // The frame records painted controls before it routes the rest.
        Action::Widget(..) => {}
        Action::Accept => {
            state.accept();
        }
        Action::Send { display, sent } => {
            state.begin(&*port.session, port.read.budget, Some(display), sent);
        }
        Action::Stop => state.stop(),
        Action::Apply => match state.apply_changes(&mut *port.session) {
            Applied::Nothing => {}
            Applied::Changed | Applied::Stale | Applied::Failed => port.sync(),
        },
        Action::Discard => {
            let was = state.previewing;
            state.discard();
            if was {
                port.sync();
            }
        }
        Action::Retry => {
            let was = state.previewing;
            state.retry(&*port.session, port.read.budget);
            if was {
                port.sync();
            }
        }
        Action::Preview(on) => {
            state.previewing = on && state.pending.as_ref().is_some_and(|p| p.fork.is_some());
            port.sync();
        }
        Action::Clear => state.clear(),
    }
}

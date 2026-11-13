//! Renders the model manager into the frame requests.
use super::{Action, State};
use crate::editor::{Context, egui};

pub(super) fn show(state: &mut State, cx: &Context<'_>, ctx: &egui::Context) -> Vec<Action> {
    let mut actions = Vec::new();
    if !state.visible() {
        return actions;
    }
    let Some(manager) = state.manager_mut() else {
        return actions;
    };
    let output = manager.ui(ctx, cx.locale, cx.dark);
    for (id, response) in output.widgets {
        actions.push(Action::Remember { id, response });
    }
    actions
}

pub(super) fn apply(port: &mut crate::editor::Port<'_>, action: Action) {
    match action {
        Action::Remember { id, response } => port.widget(id, None, None, &response),
    }
}

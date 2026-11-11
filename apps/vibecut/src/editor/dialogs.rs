//! Keeps the state of every editor dialog across frames.
pub mod export;
pub(crate) mod models;
mod preferences;

use crate::editor::{Context, Desk, egui};

/// Both editor dialogs and the state the frame lends them.
#[derive(Default)]
pub(in crate::editor) struct State {
    pub(in crate::editor) export: export::State,
    pub(in crate::editor) models: models::State,
    pub(in crate::editor) preferences: preferences::State,
}

/// What the frame must do after a dialog painted.
pub(in crate::editor) enum Action {
    Export(export::Action),
    Models(models::Action),
    Preferences(preferences::Action),
}

/// Paint both dialogs for a bench that mounts them alone.
#[cfg(test)]
pub(in crate::editor) fn show(
    state: &mut State,
    cx: &Context<'_>,
    ui: &mut egui::Ui,
) -> Vec<Action> {
    let mut actions = show_export(state, cx, ui);
    actions.extend(show_models(state, cx, ui));
    actions.extend(show_preferences(state, cx, ui));
    actions
}

/// Paint only the export dialog, keeping modal order with the other sheets.
pub(in crate::editor) fn show_export(
    state: &mut State,
    cx: &Context<'_>,
    ui: &mut egui::Ui,
) -> Vec<Action> {
    export::show(&mut state.export, cx, ui)
        .into_iter()
        .map(Action::Export)
        .collect()
}

/// Paint only the model manager, after the other modal sheets.
pub(in crate::editor) fn show_models(
    state: &mut State,
    cx: &Context<'_>,
    ui: &mut egui::Ui,
) -> Vec<Action> {
    models::show(&mut state.models, cx, ui)
        .into_iter()
        .map(Action::Models)
        .collect()
}

/// Apply every dialog request in paint order.
pub(in crate::editor) fn apply(desk: &mut Desk<'_>, actions: Vec<Action>) {
    for action in actions {
        match action {
            Action::Export(action) => export::apply(desk, action),
            Action::Models(action) => models::apply(desk, action),
            Action::Preferences(action) => preferences::apply(desk, action),
        }
    }
}

/// Paint the user-owned remote model settings.
pub(in crate::editor) fn show_preferences(
    state: &mut State,
    cx: &Context<'_>,
    ui: &mut egui::Ui,
) -> Vec<Action> {
    preferences::show(
        &mut state.preferences,
        cx,
        ui.ctx(),
        &engine::providers::Credentials::from_env(),
    )
    .into_iter()
    .map(Action::Preferences)
    .collect()
}

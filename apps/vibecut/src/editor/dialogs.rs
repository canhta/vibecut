//! Keeps the state of every editor dialog across frames.
pub mod export;
pub(crate) mod models;
mod preferences;
pub(in crate::editor) use preferences::{export_shortcuts, import_shortcuts};
pub(in crate::editor) mod scene_split;
pub(crate) mod voiceover;

use crate::editor::{Context, Port, egui};

/// Every editor dialog and the state the frame lends them.
#[derive(Default)]
pub(in crate::editor) struct State {
    pub(in crate::editor) export: export::State,
    pub(in crate::editor) models: models::State,
    pub(in crate::editor) preferences: preferences::State,
    pub(in crate::editor) voiceover: voiceover::State,
    pub(in crate::editor) scene_split: scene_split::State,
}

/// What the frame must do after a dialog painted.
pub(in crate::editor) enum Action {
    Export(export::Action),
    Models(models::Action),
    Preferences(preferences::Action),
    Voiceover(voiceover::Action),
    SceneSplit(scene_split::Action),
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

/// Apply one dialog request; each dialog handles its own.
pub(in crate::editor) fn apply(state: &mut State, port: &mut Port<'_>, action: Action) {
    match action {
        Action::Export(action) => export::apply(&mut state.export, port, action),
        Action::Models(action) => models::apply(port, action),
        Action::Preferences(action) => preferences::apply(port, action),
        Action::Voiceover(action) => voiceover::apply(&mut state.voiceover, port, action),
        Action::SceneSplit(action) => scene_split::apply(&mut state.scene_split, port, action),
    }
}

/// Paint the voice-over recorder.
pub(in crate::editor) fn show_voiceover(
    state: &mut State,
    cx: &Context<'_>,
    ui: &mut egui::Ui,
) -> Vec<Action> {
    voiceover::show(&mut state.voiceover, cx, ui.ctx())
        .into_iter()
        .map(Action::Voiceover)
        .collect()
}

/// Paint the scene split review and route its requests.
pub(in crate::editor) fn show_scene_split(
    state: &mut State,
    cx: &Context<'_>,
    ui: &mut egui::Ui,
) -> Vec<Action> {
    scene_split::show(&mut state.scene_split, cx, ui)
        .into_iter()
        .map(Action::SceneSplit)
        .collect()
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

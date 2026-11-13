//! Edits the single persisted provider/feature model map; refresh is explicitly requested.
pub(in crate::editor) mod settings;
mod shortcuts;

pub(in crate::editor) use settings::Shortcuts;
pub(in crate::editor) use shortcuts::{export as export_shortcuts, import as import_shortcuts};

use crate::{
    editor::{Context, Port, egui},
    i18n::Text,
    services::remote_models::{AiPreferences, Catalogs},
};
use ai::Operation;
use std::{sync::Arc, time::Duration};

#[derive(Default)]
pub(in crate::editor) struct State {
    pub(in crate::editor) open: bool,
    /// The AI model rows; opened from the Settings General tab.
    pub(in crate::editor) models_open: bool,
    tab: settings::Tab,
    shortcut_notice: Option<settings::ShortcutNotice>,
    catalogs: Catalogs,
    draft: Option<Draft>,
    transport: Option<Arc<dyn transport::HttpTransport>>,
}
struct Draft {
    provider: String,
    feature: Operation,
    original: String,
    value: String,
}
pub(in crate::editor) enum Action {
    Models(AiPreferences),
    /// The one text-task provider shared by translation and filler classification.
    TextProvider(String),
    Remember(String, egui::Response),
    Set(&'static str, serde_json::Value),
    ClearCache,
    Shortcuts(settings::Shortcuts),
}
/// Providers the text-task row offers; each one's text-task model is its Translate row.
const TEXT_PROVIDERS: [&str; 2] = ["openai", "anthropic"];
fn provider_name(provider: &str) -> &'static str {
    match provider {
        "openai" => "OpenAI",
        "anthropic" => "Anthropic",
        _ => "ElevenLabs",
    }
}
fn feature_text(feature: Operation) -> Text {
    match feature {
        Operation::Transcribe => Text::ShortsTranscriber,
        Operation::Translate => Text::SubTranslate,
        Operation::SuggestClips => Text::ShortsSuggester,
        Operation::GenerateChapters => Text::ModelRemoteChapters,
        Operation::SelectEmphasis => Text::ModelRemoteEmphasis,
        Operation::Synthesize => Text::ModelRemoteSpeech,
        Operation::ClassifyFillers => Text::ModelRemoteFillers,
        Operation::Align => Text::ShortsTranscriber,
    }
}
pub(super) fn show(
    state: &mut State,
    cx: &Context<'_>,
    ctx: &egui::Context,
    credentials: &engine::providers::Credentials,
) -> Vec<Action> {
    state.catalogs.sync(credentials);
    let mut actions = Vec::new();
    if state.open {
        settings::show(state, cx, ctx, &mut actions);
    }
    if !state.models_open {
        return actions;
    }
    let tr = |text: Text| text.text(cx.locale);
    let mut models = cx.preferences.ai_models.clone();
    if state
        .draft
        .as_ref()
        .is_some_and(|draft| models.model(&draft.provider, draft.feature) != draft.original)
    {
        state.draft = None;
    }
    let modal = egui::Modal::new(egui::Id::unique("preferences.ai"))
        .frame(crate::kit::dialog_frame(ctx))
        .show(ctx, |ui| {
            ui.set_width(640.0);
            ui.spacing_mut().interact_size.y = crate::kit::size::FIELD;
            crate::kit::dialog_title(ui, tr(Text::Settings));
            ui.label(tr(Text::ModelRemoteModels));
            ui.horizontal(|ui| {
                let label = ui.add_sized(
                    egui::vec2(180.0, crate::kit::size::FIELD),
                    egui::Label::new(tr(Text::ModelTextProvider)),
                );
                let current = cx.preferences.subtitles.text_provider.clone();
                let combo =
                    egui::ComboBox::from_id_salt(egui::IdSalt::new("preferences.text.provider"))
                        .selected_text(provider_name(&current))
                        .width(280.0)
                        .show_ui(ui, |ui| {
                            for provider in TEXT_PROVIDERS {
                                let response = ui
                                    .selectable_label(provider == current, provider_name(provider));
                                actions.push(Action::Remember(
                                    format!("preferences.text.provider.{provider}"),
                                    response.clone(),
                                ));
                                if response.clicked() {
                                    actions.push(Action::TextProvider(provider.into()));
                                    ui.close();
                                }
                            }
                        });
                actions.push(Action::Remember(
                    "preferences.text.provider.combo".into(),
                    combo.response.labelled_by(label.id),
                ));
            });
            egui::ScrollArea::vertical()
                .max_height(500.0)
                .show(ui, |ui| {
                    for provider in ["openai", "anthropic", "elevenlabs"] {
                        ui.separator();
                        ui.horizontal(|ui| {
                            ui.strong(provider_name(provider));
                            let Some(catalog) = state.catalogs.get_mut(provider) else {
                                return;
                            };
                            let response = ui.add_enabled(
                                catalog.has_key() && !catalog.loading(),
                                egui::Button::new(tr(Text::ModelRemoteRefresh)),
                            );
                            actions.push(Action::Remember(
                                format!("preferences.ai.{provider}.refresh"),
                                response.clone(),
                            ));
                            if response.clicked() {
                                let transport =
                                    state.transport.clone().map(Ok).unwrap_or_else(|| {
                                        transport::UreqTransport::with_timeout(Duration::from_secs(
                                            15,
                                        ))
                                        .map(
                                            |transport| {
                                                Arc::new(transport)
                                                    as Arc<dyn transport::HttpTransport>
                                            },
                                        )
                                    });
                                match transport {
                                    Ok(transport) => {
                                        if catalog.request(transport).is_err() {
                                            catalog.error = Some(ai::Error::Inference);
                                        }
                                    }
                                    Err(error) => catalog.error = Some(error.into()),
                                }
                            }
                        });
                        let Some(catalog) = state.catalogs.get(provider) else {
                            continue;
                        };
                        let status = if !catalog.has_key() {
                            Some(Text::ModelRemoteNoKey)
                        } else if catalog.loading() {
                            ctx.request_repaint_after(Duration::from_millis(30));
                            Some(Text::ModelRemoteLoading)
                        } else if catalog.error.is_some() {
                            Some(Text::ModelRemoteListFailed)
                        } else if catalog.fetched && catalog.models.is_empty() {
                            Some(Text::ModelRemoteEmpty)
                        } else {
                            None
                        };
                        if let Some(status) = status {
                            ui.label(tr(status));
                        }
                        for row in ai::defaults::DEFAULTS
                            .iter()
                            .filter(|row| row.shipped_default && row.provider == provider)
                        {
                            let id = format!("preferences.ai.{provider}.{:?}", row.feature)
                                .to_lowercase();
                            ui.horizontal(|ui| {
                                let label = ui.add_sized(
                                    egui::vec2(180.0, crate::kit::size::FIELD),
                                    egui::Label::new(tr(feature_text(row.feature))),
                                );
                                let current = models.model(provider, row.feature).to_owned();
                                let choices =
                                    models.options(provider, row.feature, &catalog.models);
                                let combo = egui::ComboBox::from_id_salt(&id)
                                    .selected_text(&current)
                                    .width(280.0)
                                    .show_ui(ui, |ui| {
                                        for (index, model) in choices.iter().enumerate() {
                                            let label = if index == 0 {
                                                format!("{model} ({})", tr(Text::ModelSuggested))
                                            } else {
                                                model.clone()
                                            };
                                            let response =
                                                ui.selectable_label(model == &current, label);
                                            actions.push(Action::Remember(
                                                format!("{id}.option.{index}"),
                                                response.clone(),
                                            ));
                                            if response.clicked() {
                                                let _ = models.select(provider, row.feature, model);
                                                state.draft = None;
                                                ui.close();
                                            }
                                        }
                                        let response = ui.button(tr(Text::ModelRemoteCustom));
                                        actions.push(Action::Remember(
                                            format!("{id}.custom"),
                                            response.clone(),
                                        ));
                                        if response.clicked() {
                                            state.draft = Some(Draft {
                                                provider: provider.into(),
                                                feature: row.feature,
                                                original: current.clone(),
                                                value: String::new(),
                                            });
                                            ui.close();
                                        }
                                    });
                                actions.push(Action::Remember(
                                    format!("{id}.model"),
                                    combo.response.labelled_by(label.id),
                                ));
                            });
                            if let Some(draft) = state.draft.as_mut().filter(|draft| {
                                draft.provider == provider && draft.feature == row.feature
                            }) {
                                ui.horizontal(|ui| {
                                    let response = ui.add(
                                        egui::TextEdit::singleline(&mut draft.value)
                                            .char_limit(ai::defaults::MAX_MODEL_BYTES)
                                            .hint_text(tr(Text::ModelRemoteCustom)),
                                    );
                                    actions.push(Action::Remember(format!("{id}.input"), response));
                                    let valid = ai::defaults::validate_model(&draft.value).is_ok();
                                    let response = ui
                                        .add_enabled(valid, egui::Button::new(tr(Text::CropApply)));
                                    actions.push(Action::Remember(
                                        format!("{id}.apply"),
                                        response.clone(),
                                    ));
                                    if response.clicked() {
                                        let _ = models.select(provider, row.feature, &draft.value);
                                        draft.original = draft.value.clone();
                                    }
                                    if !valid {
                                        ui.label(tr(Text::ModelRemoteInvalid));
                                    }
                                });
                            }
                        }
                    }
                });
            let response = ui.button(tr(Text::ModelClose));
            actions.push(Action::Remember(
                "preferences.ai.close".into(),
                response.clone(),
            ));
            if response.clicked() {
                state.models_open = false;
                state.draft = None;
            }
        });
    if modal.should_close() {
        state.models_open = false;
        state.draft = None;
    }
    if models != cx.preferences.ai_models {
        actions.push(Action::Models(models));
    }
    actions
}
pub(super) fn apply(port: &mut Port<'_>, action: Action) {
    match action {
        Action::Models(models) => {
            if let Ok(value) = serde_json::to_value(models) {
                port.set_preference("ai_models", value);
            }
        }
        Action::TextProvider(provider) => {
            let mut subtitles = port.read.preferences.subtitles.clone();
            subtitles.text_provider = provider;
            if let Ok(value) = serde_json::to_value(subtitles) {
                port.set_preference("subtitles", value);
            }
        }
        Action::Remember(id, response) => port.widget(id, None, None, &response),
        Action::Set(key, value) => {
            port.set_preference(key, value);
        }
        Action::ClearCache => port.request(crate::editor::context::Request::Cache(
            crate::editor::context::CacheAction::Clear,
        )),
        Action::Shortcuts(kind) => {
            let request = match kind {
                Shortcuts::Export => crate::services::lifecycle::EditorAction::KeybindingsExport,
                Shortcuts::Import { replace } => {
                    crate::services::lifecycle::EditorAction::KeybindingsImport { replace }
                }
            };
            port.request(crate::editor::context::Request::Lifecycle(request));
        }
    }
}

#[cfg(test)]
mod tests;

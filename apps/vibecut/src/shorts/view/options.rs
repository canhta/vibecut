//! Generation settings: language, who transcribes and suggests, how many and how long.
use super::{State, named};
use crate::{
    catalog::{Locale, Text},
    egui, kit,
};

pub(super) fn show(state: &mut State, ui: &mut egui::Ui) {
    ui.spacing_mut().item_spacing.y = 8.0;
    ui.set_width(380.0);
    let tr = |state: &State, key| state.tr(key);
    ui.label(tr(state, Text::ShortsSourceLanguage));
    let combo = egui::ComboBox::from_id_salt(egui::Id::unique("shorts.language"))
        .selected_text(&state.request.language)
        .show_ui(ui, |ui| {
            for language in ["vi", "en", "zh"] {
                let response =
                    ui.selectable_value(&mut state.request.language, language.to_owned(), language);
                super::author(&response, &format!("shorts.language.{language}"));
            }
        });
    named(
        &combo.response,
        "shorts.language",
        tr(state, Text::ShortsSourceLanguage),
    );
    providers(state, ui);
    egui::Grid::new("shorts.durations")
        .spacing([16.0, 8.0])
        .show(ui, |ui| {
            for (key, id, value, range) in [
                (
                    Text::ShortsCount,
                    "shorts.count",
                    &mut state.request.count,
                    1..=32,
                ),
                (
                    Text::ShortsMaxSegments,
                    "shorts.max_segments",
                    &mut state.request.max_segments,
                    1..=32,
                ),
            ] {
                ui.label(catalog::text(state.preferences.locale, key));
                named(
                    &ui.add(egui::DragValue::new(value).range(range)),
                    id,
                    catalog::text(state.preferences.locale, key),
                );
                ui.end_row();
            }
            for (key, id, value) in [
                (
                    Text::ShortsMin,
                    "shorts.min",
                    &mut state.request.min_seconds,
                ),
                (
                    Text::ShortsMax,
                    "shorts.max",
                    &mut state.request.max_seconds,
                ),
            ] {
                ui.label(catalog::text(state.preferences.locale, key));
                named(
                    &ui.add(egui::DragValue::new(value).range(1..=180)),
                    id,
                    catalog::text(state.preferences.locale, key),
                );
                ui.end_row();
            }
        });
}

use crate::catalog;

fn providers(state: &mut State, ui: &mut egui::Ui) {
    use engine::subtitles::{CatalogOperation, provider_options_for};
    let locale = state.preferences.locale;
    let language = captions::LanguageTag::parse(&state.request.language).ok();
    ui.label(state.tr(Text::ShortsTranscriber));
    let transcription = provider_options_for(CatalogOperation::Transcribe, language.as_ref());
    let text = provider_label(
        locale,
        &state.request.transcription_provider,
        &state.request.transcription_model,
    );
    let combo = egui::ComboBox::from_id_salt(egui::Id::unique("shorts.transcriber"))
        .width(380.0)
        .selected_text(text)
        .show_ui(ui, |ui| {
            for option in &transcription {
                for model in &option.models {
                    let label = provider_label(locale, option.id, model);
                    let selected = option.id == state.request.transcription_provider
                        && model == &state.request.transcription_model;
                    let response = ui.selectable_label(selected, label);
                    super::author(
                        &response,
                        &format!("shorts.transcriber.{}.{}", option.id, model),
                    );
                    if response.clicked() {
                        state.request.transcription_provider = option.id.into();
                        state.request.transcription_model = model.clone();
                    }
                }
            }
        });
    named(
        &combo.response,
        "shorts.transcriber",
        state.tr(Text::ShortsTranscriber),
    );
    ui.label(state.tr(Text::ShortsSuggester));
    let local = state.tr(Text::ShortsLocal);
    let label = if state.request.suggestion_provider == "heuristic" {
        local.to_owned()
    } else {
        provider_label(
            locale,
            &state.request.suggestion_provider,
            &state.request.suggestion_model,
        )
    };
    let options = engine::clip_suggest::suggestion_options();
    let combo = egui::ComboBox::from_id_salt(egui::Id::unique("shorts.suggester"))
        .width(380.0)
        .selected_text(label)
        .show_ui(ui, |ui| {
            for option in &options {
                for model in &option.models {
                    let label = if option.local {
                        local.to_owned()
                    } else {
                        provider_label(locale, option.id, model)
                    };
                    let selected = option.id == state.request.suggestion_provider
                        && *model == state.request.suggestion_model;
                    let r = ui.selectable_label(selected, label);
                    super::author(&r, &format!("shorts.suggester.{}.{}", option.id, model));
                    if r.clicked() {
                        state.request.suggestion_provider = option.id.into();
                        state.request.suggestion_model = (*model).into();
                    }
                }
            }
        });
    named(
        &combo.response,
        "shorts.suggester",
        state.tr(Text::ShortsSuggester),
    );
    let remote = options
        .iter()
        .find(|o| o.id == state.request.suggestion_provider)
        .and_then(|o| o.policy_url.map(|url| (o.id, url)))
        .or_else(|| {
            transcription
                .iter()
                .find(|o| o.id == state.request.transcription_provider && !o.local)
                .map(|o| {
                    (
                        o.id,
                        if o.id == "openai" {
                            engine::clip_suggest::OPENAI_POLICY_URL
                        } else {
                            "https://elevenlabs.io/terms-of-use"
                        },
                    )
                })
        });
    if let Some((id, url)) = remote {
        ui.label(
            egui::RichText::new(state.tr(Text::ShortsDisclosure))
                .color(kit::secondary(state.preferences.dark)),
        );
        let r = ui.hyperlink_to(id, url);
        super::author(&r, "shorts.suggestion.policy");
    }
}

fn provider_label(locale: Locale, provider: &str, model: &str) -> String {
    catalog::format(
        locale,
        Text::ShortsProviderModel,
        &[("provider", provider), ("model", model)],
    )
}

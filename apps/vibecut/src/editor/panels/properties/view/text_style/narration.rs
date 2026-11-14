//! Renders the Text to speech tab of a text item and the consent sheet of its upload.
use super::super::appearance::FIELD_COLUMN;
use super::Panel;
use crate::editor::properties::workflow::jobs::narration::{
    Action as NarrationAction, Cause, Failed, LANGUAGES, TakeView,
};
use crate::editor::properties::{Action, Id, ItemId, Text, egui, read, theme};
use crate::i18n;
use crate::kit;
use ai::speech::VoiceCategory;
use engine::dubbing::Fit;
use engine::providers::Cost;
use project::{AudioSource, ItemParams, NarrationDucking};
use time::{TICKS_PER_SECOND, Tick};

/// Longest text the box takes: a quarter of the adapters' 64 KiB byte ceiling in characters.
const MAX_CHARACTERS: usize = 16 * 1024;
const DB_SUFFIX: &str = " dB";
const MS_SUFFIX: &str = " ms";
const REPAINT: std::time::Duration = std::time::Duration::from_millis(50);

fn seconds(tick: Tick) -> f64 {
    tick.0 as f64 / TICKS_PER_SECOND as f64
}

fn language_label(tag: &str) -> Text {
    match tag {
        "vi" => Text::NarrationLangVi,
        "zh" => Text::NarrationLangZh,
        _ => Text::NarrationLangEn,
    }
}

fn category_label(category: VoiceCategory) -> Text {
    match category {
        VoiceCategory::BuiltIn => Text::NarrationCategoryBuiltIn,
        VoiceCategory::Stock => Text::NarrationCategoryStock,
    }
}

/// The action that stores a ducking value: the pending take's, or a placed item's own.
fn ducking_action(target: Option<ItemId>, ducking: NarrationDucking) -> Action {
    match target {
        Some(item) => Action::Narration(NarrationAction::SetPlacedDucking { item, ducking }),
        None => Action::Narration(NarrationAction::SetDucking(ducking)),
    }
}

/// The ducking a placed narration item stores, while that item still exists.
fn placed_ducking(project: &project::Project, item: ItemId) -> Option<NarrationDucking> {
    let found = read::item(project, item)?;
    match &found.content {
        ItemParams::Audio(params) => match &params.source {
            AudioSource::Narration { narration } => Some(narration.ducking),
            _ => None,
        },
        _ => None,
    }
}

impl Panel<'_, '_> {
    /// The Text to speech tab: voice, script, the generated take and its ducking.
    pub(in super::super) fn narration_tab(&mut self, ui: &mut egui::Ui, item: ItemId) {
        let Some(content) =
            read::item(self.cx.session.project(), item).and_then(|found| match &found.content {
                ItemParams::Text(params) => Some(params.content.as_str().to_owned()),
                _ => None,
            })
        else {
            return;
        };
        self.state.narration.bind(item, &content);
        let enabled = !read::item_locked(self.cx.session.project(), item);
        if self.state.narration.busy() {
            ui.ctx().request_repaint_after(REPAINT);
        }
        self.panel_section(
            ui,
            "inspector.section.narration_voice",
            "inspector.narration_voice",
            Text::NarrationVoice,
            true,
            false,
            |panel, ui| panel.narration_voice_rows(ui, enabled),
        );
        self.panel_section(
            ui,
            "inspector.section.narration_script",
            "inspector.narration_script",
            Text::NarrationScript,
            true,
            false,
            |panel, ui| panel.narration_script_rows(ui, item, enabled),
        );
        if let Some(take) = self.state.narration.take() {
            self.panel_section(
                ui,
                "inspector.section.narration_take",
                "inspector.narration_take",
                Text::NarrationTake,
                true,
                false,
                |panel, ui| panel.narration_take_rows(ui, item, take, enabled),
            );
        } else if self.state.narration.placed().is_some() {
            self.narration_line(
                ui,
                "inspector.narration.placed",
                self.tr(Text::NarrationPlaced),
                false,
            );
        }
        let ducking = match self.state.narration.take() {
            Some(take) => Some(take.ducking),
            None => self
                .state
                .narration
                .placed()
                .and_then(|placed| placed_ducking(self.cx.session.project(), placed)),
        };
        if let Some(ducking) = ducking {
            self.panel_section(
                ui,
                "inspector.section.narration_ducking",
                "inspector.narration_ducking",
                Text::NarrationDucking,
                true,
                false,
                |panel, ui| panel.narration_ducking_rows(ui, ducking, enabled, None),
            );
        }
    }

    /// The Ducking section of a placed narration item on its Audio tab, where the selection
    /// lands after placement. Other audio items have no such section.
    pub(in super::super) fn narration_audio_ducking(
        &mut self,
        ui: &mut egui::Ui,
        item: ItemId,
        enabled: bool,
    ) {
        let Some(ducking) = placed_ducking(self.cx.session.project(), item) else {
            return;
        };
        self.panel_section(
            ui,
            "inspector.section.narration_ducking",
            "inspector.narration_ducking",
            Text::NarrationDucking,
            true,
            false,
            |panel, ui| panel.narration_ducking_rows(ui, ducking, enabled, Some(item)),
        );
    }

    /// One dropdown row of the voice section; the chosen index, when the person picked one.
    fn narration_choice(
        &mut self,
        ui: &mut egui::Ui,
        id: &'static str,
        label: Text,
        choices: &[String],
        selected: usize,
    ) -> Option<usize> {
        let (row, mut value_ui) = self.row_value_ui(ui, theme::size::PROPERTY_ROW);
        self.row_label(ui, row, self.tr(label));
        let shown = choices.get(selected).cloned().unwrap_or_default();
        let mut chosen = None;
        let response = value_ui
            .scope(|ui| {
                ui.spacing_mut().interact_size.y = theme::size::FIELD;
                egui::ComboBox::from_id_salt(Id::unique(id))
                    .selected_text(shown)
                    .width(ui.available_width().min(FIELD_COLUMN))
                    .show_ui(ui, |ui| {
                        for (index, choice) in choices.iter().enumerate() {
                            let response = ui.selectable_label(index == selected, choice.as_str());
                            self.remember(format!("{id}.{index}"), &response);
                            if response.clicked() {
                                chosen = Some(index);
                                ui.close();
                            }
                        }
                    })
                    .response
            })
            .inner;
        response
            .widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, self.tr(label)));
        self.remember(id, &response);
        chosen
    }

    /// Model and language dropdowns, then the voice section: category chips, the All/Commercial
    /// filter and the grid of voice tiles. A chip chooses its category's first provider, a tile
    /// chooses its provider and voice; nothing is chosen for the person.
    fn narration_voice_rows(&mut self, ui: &mut egui::Ui, enabled: bool) {
        let form = self.state.narration.form;
        // Each provider's category and the labels of its voices, in the order the adapter lists them.
        let options: Vec<(VoiceCategory, Vec<&'static str>)> = self
            .state
            .narration
            .options()
            .iter()
            .map(|option| {
                (
                    option.category,
                    option.voices.iter().map(|(_, label)| *label).collect(),
                )
            })
            .collect();
        let Some(models) = self
            .state
            .narration
            .option()
            .map(|option| option.models.clone())
        else {
            return;
        };
        let mut categories: Vec<VoiceCategory> = Vec::new();
        for (category, _) in &options {
            if !categories.contains(category) {
                categories.push(*category);
            }
        }
        let chosen = options.get(form.provider).map(|(category, _)| *category);
        let languages: Vec<String> = LANGUAGES
            .iter()
            .map(|tag| self.tr(language_label(tag)).to_owned())
            .collect();
        let filters = [
            self.tr(Text::NarrationVoiceAll).to_owned(),
            self.tr(Text::NarrationVoiceCommercial).to_owned(),
        ];
        let commercial = self.state.narration.commercial_only;
        let busy = self.state.narration.busy();
        ui.add_enabled_ui(enabled && !busy, |ui| {
            if let Some(index) = self.narration_choice(
                ui,
                "inspector.narration.model",
                Text::NarrationModel,
                &models,
                form.model,
            ) {
                self.state.narration.form.model = index;
            }
            if let Some(index) = self.narration_choice(
                ui,
                "inspector.narration.language",
                Text::NarrationLanguage,
                &languages,
                form.language,
            ) {
                self.state.narration.form.language = index;
            }
            ui.horizontal_wrapped(|ui| {
                for (index, category) in categories.iter().enumerate() {
                    let label = self.tr(category_label(*category));
                    let response = ui.selectable_label(chosen == Some(*category), label);
                    self.remember(format!("inspector.narration.category.{index}"), &response);
                    if response.clicked()
                        && let Some(first) = options.iter().position(|(owner, _)| owner == category)
                    {
                        let narration = &mut self.state.narration.form;
                        narration.provider = first;
                        narration.model = 0;
                        narration.voice = 0;
                    }
                }
            });
            let selected = usize::from(commercial);
            if let Some(index) = self.narration_choice(
                ui,
                "inspector.narration.voice_filter",
                Text::NarrationVoiceFilter,
                &filters,
                selected,
            ) {
                self.state.narration.commercial_only = index == 1;
            }
            let mut picked = None;
            let mut shown = 0usize;
            ui.horizontal_wrapped(|ui| {
                for (provider, (category, labels)) in options.iter().enumerate() {
                    // Commercial lists the stock voices only; the chosen category is the chip.
                    if Some(*category) != chosen
                        || (commercial && *category != VoiceCategory::Stock)
                    {
                        continue;
                    }
                    for (voice, label) in labels.iter().enumerate() {
                        let response = ui.selectable_label(
                            form.provider == provider && form.voice == voice,
                            *label,
                        );
                        self.remember(
                            format!("inspector.narration.voice.{provider}.{voice}"),
                            &response,
                        );
                        if response.clicked() {
                            picked = Some((provider, voice));
                        }
                        shown += 1;
                    }
                }
            });
            if let Some((provider, voice)) = picked {
                let narration = &mut self.state.narration.form;
                narration.provider = provider;
                narration.model = 0;
                narration.voice = voice;
            }
            if shown == 0 {
                let text = self.tr(Text::NarrationNoVoice);
                self.narration_line(ui, "inspector.narration.voice_empty", text, false);
            }
        });
    }

    /// One status line on a full property row.
    fn narration_line(&mut self, ui: &mut egui::Ui, id: &str, text: &str, error: bool) {
        let color = if error {
            theme::error_text(self.cx.dark)
        } else {
            self.secondary()
        };
        let response = ui
            .allocate_ui_with_layout(
                egui::vec2(ui.available_width(), theme::size::PROPERTY_ROW),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(text)
                                .size(theme::type_size::BODY)
                                .color(color),
                        )
                        .wrap_mode(egui::TextWrapMode::Wrap),
                    )
                },
            )
            .inner;
        self.remember(id, &response);
    }

    /// The text box, its counter, Generate (or the busy line with Cancel) and the failure line.
    fn narration_script_rows(&mut self, ui: &mut egui::Ui, item: ItemId, enabled: bool) {
        let busy = self.state.narration.busy();
        ui.add_enabled_ui(enabled && !busy, |ui| {
            let response = self.state.narration.text.as_mut().map(|(_, text)| {
                ui.add(
                    egui::TextEdit::multiline(text)
                        .id_salt("inspector.narration.text")
                        .desired_rows(4)
                        .desired_width(ui.available_width())
                        .char_limit(MAX_CHARACTERS),
                )
            });
            if let Some(response) = response {
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::Role::MultilineTextInput,
                        true,
                        self.tr(Text::NarrationScript),
                    )
                });
                self.remember("inspector.narration.text", &response);
            }
        });
        let count = self
            .state
            .narration
            .text
            .as_ref()
            .map_or(0, |(_, text)| text.chars().count());
        let cue = read::item(self.cx.session.project(), item)
            .and_then(|found| found.duration().ok())
            .map_or(0.0, seconds);
        let locale = self.cx.locale;
        let counter = i18n::format(
            locale,
            Text::NarrationCounter,
            &[
                ("count", &i18n::number(locale, count as u64)),
                ("seconds", &i18n::decimal(locale, cue, 2)),
            ],
        );
        self.narration_line(ui, "inspector.narration.counter", &counter, false);
        let ready = self
            .state
            .narration
            .text
            .as_ref()
            .is_some_and(|(_, text)| !text.trim().is_empty());
        let (_, mut value_ui) = self.row_value_ui(ui, theme::size::PROPERTY_ROW);
        let width = value_ui.available_width();
        if busy {
            let cancel = value_ui
                .push_id("inspector.narration.cancel", |ui| {
                    ui.add(
                        egui::Button::new(self.tr(Text::NarrationCancel))
                            .min_size(egui::vec2(width, theme::size::CONTROL)),
                    )
                })
                .inner;
            self.remember("inspector.narration.cancel", &cancel);
            if cancel.clicked() {
                self.actions
                    .push(Action::Narration(NarrationAction::Cancel));
            }
            let generating = self.tr(Text::NarrationGenerating);
            self.narration_line(ui, "inspector.narration.status", generating, false);
        } else {
            let generate = value_ui
                .push_id("inspector.narration.generate", |ui| {
                    ui.add_enabled(
                        enabled && ready,
                        egui::Button::new(self.tr(Text::NarrationGenerate))
                            .min_size(egui::vec2(width, theme::size::CONTROL)),
                    )
                })
                .inner;
            self.remember("inspector.narration.generate", &generate);
            if generate.clicked() {
                self.actions
                    .push(Action::Narration(NarrationAction::Generate { item }));
            }
        }
        if let Some(failed) = self.state.narration.failure() {
            let locale = self.cx.locale;
            let text = match failed {
                Failed::PaidPlan(voice) => {
                    i18n::format(locale, Text::NarrationFailPaidPlan, &[("voice", voice)])
                }
                other => self.tr(failure_text(other)).to_owned(),
            };
            self.narration_line(ui, "inspector.narration.failure", &text, true);
        }
    }

    /// The finished take: its length, the fit decision and what the person may do about it.
    fn narration_take_rows(
        &mut self,
        ui: &mut egui::Ui,
        item: ItemId,
        take: TakeView,
        enabled: bool,
    ) {
        let locale = self.cx.locale;
        let summary = i18n::format(
            locale,
            Text::NarrationTakeReady,
            &[
                ("natural", &i18n::decimal(locale, seconds(take.natural), 2)),
                ("cue", &i18n::decimal(locale, seconds(take.cue), 2)),
            ],
        );
        self.narration_line(ui, "inspector.narration.take", &summary, false);
        let speed = |thousandths: u16| i18n::decimal(locale, f64::from(thousandths) / 1000.0, 3);
        let (line, error) = match take.fit {
            Ok(Fit::Exact) => (self.tr(Text::NarrationFitExact).to_owned(), false),
            Ok(Fit::Stretch { thousandths }) => (
                i18n::format(
                    locale,
                    Text::NarrationFitStretch,
                    &[("speed", &speed(thousandths))],
                ),
                false,
            ),
            Ok(Fit::Gap) => (self.tr(Text::NarrationFitGap).to_owned(), false),
            Ok(Fit::ProviderSpeed { thousandths }) => (
                i18n::format(
                    locale,
                    Text::NarrationFitProviderSpeed,
                    &[("speed", &speed(thousandths))],
                ),
                false,
            ),
            Ok(Fit::Rephrase { max_characters }) => (
                i18n::format(
                    locale,
                    Text::NarrationFitRephrase,
                    &[("count", &i18n::number(locale, max_characters))],
                ),
                false,
            ),
            Ok(Fit::Overflow { thousandths, over }) => (
                i18n::format(
                    locale,
                    Text::NarrationFitOverflow,
                    &[
                        ("over", &i18n::decimal(locale, seconds(over), 2)),
                        ("speed", &speed(thousandths)),
                    ],
                ),
                false,
            ),
            Err(_) => (self.tr(Text::NarrationFitRefused).to_owned(), true),
        };
        self.narration_line(ui, "inspector.narration.fit", &line, error);
        let place = match take.fit {
            Ok(Fit::Exact | Fit::Stretch { .. } | Fit::Gap) => Some(Text::NarrationPlace),
            Ok(Fit::Overflow { .. }) => Some(Text::NarrationPlaceOverflow),
            _ => None,
        };
        let busy = self.state.narration.busy();
        let (_, mut value_ui) = self.row_value_ui(ui, theme::size::PROPERTY_ROW);
        let width = value_ui.available_width();
        let mut buttons = Vec::new();
        if let Some(label) = place {
            buttons.push((
                "inspector.narration.place",
                self.tr(label).to_owned(),
                NarrationAction::Place { item },
            ));
        }
        if let Ok(Fit::ProviderSpeed { thousandths }) = take.fit {
            buttons.push((
                "inspector.narration.regenerate",
                i18n::format(
                    locale,
                    Text::NarrationRegenerate,
                    &[("speed", &speed(thousandths))],
                ),
                NarrationAction::Regenerate {
                    item,
                    speed: thousandths,
                },
            ));
        }
        buttons.push((
            "inspector.narration.discard",
            self.tr(Text::NarrationDiscard).to_owned(),
            NarrationAction::Discard,
        ));
        let each = (width / buttons.len() as f32 - theme::space::SM).max(40.0);
        for (id, label, action) in buttons {
            value_ui.add_space(theme::space::SM / 2.0);
            let response = value_ui
                .push_id(id, |ui| {
                    ui.add_enabled(
                        enabled && !busy,
                        egui::Button::new(label.as_str())
                            .min_size(egui::vec2(each, theme::size::CONTROL)),
                    )
                })
                .inner;
            self.remember(id, &response);
            if response.clicked() {
                self.actions.push(Action::Narration(action));
            }
        }
    }

    /// Ducking of the original audio under the take: switch, depth, attack and release. Values
    /// commit when a field is released, so a drag is one edit.
    /// The ducking rows. `target` is the placed item they change, or `None` for the pending take.
    fn narration_ducking_rows(
        &mut self,
        ui: &mut egui::Ui,
        current: NarrationDucking,
        enabled: bool,
        target: Option<ItemId>,
    ) {
        let shown = self.state.narration.ducking_draft.unwrap_or(current);
        ui.add_enabled_ui(enabled, |ui| {
            let (row, mut value_ui) = self.row_value_ui(ui, theme::size::PROPERTY_ROW);
            self.row_label(ui, row, self.tr(Text::NarrationDuckEnabled));
            let mut on = shown.enabled;
            let width = value_ui.available_width();
            value_ui.add_space((width - theme::size::TOGGLE_WIDTH).max(0.0));
            let toggle = kit::toggle(
                &mut value_ui,
                "inspector.narration.ducking.enabled",
                &mut on,
                self.tr(Text::NarrationDuckEnabled),
            );
            self.remember("inspector.narration.ducking.enabled", &toggle);
            if toggle.clicked() {
                self.actions.push(ducking_action(
                    target,
                    NarrationDucking {
                        enabled: on,
                        ..shown
                    },
                ));
            }
            let (row, mut value_ui) = self.row_value_ui(ui, theme::size::PROPERTY_ROW);
            self.row_label(ui, row, self.tr(Text::NarrationDuckDepth));
            let mut depth = f64::from(shown.depth_tenths_db) / 10.0;
            let response = value_ui.add(
                egui::DragValue::new(&mut depth)
                    .range(0.0..=f64::from(project::MAX_DUCK_TENTHS_DB) / 10.0)
                    .speed(0.1)
                    .fixed_decimals(1)
                    .suffix(DB_SUFFIX),
            );
            self.remember("inspector.narration.ducking.depth", &response);
            self.ducking_edit(
                &response,
                NarrationDucking {
                    depth_tenths_db: (depth * 10.0).round() as u16,
                    ..shown
                },
                target,
            );
            let (row, mut value_ui) = self.row_value_ui(ui, theme::size::PROPERTY_ROW);
            self.row_label(ui, row, self.tr(Text::NarrationDuckAttack));
            let mut attack = shown.attack_ms;
            let response = value_ui.add(
                egui::DragValue::new(&mut attack)
                    .range(0..=project::MAX_DUCK_MS)
                    .suffix(MS_SUFFIX),
            );
            self.remember("inspector.narration.ducking.attack", &response);
            self.ducking_edit(
                &response,
                NarrationDucking {
                    attack_ms: attack,
                    ..shown
                },
                target,
            );
            let (row, mut value_ui) = self.row_value_ui(ui, theme::size::PROPERTY_ROW);
            self.row_label(ui, row, self.tr(Text::NarrationDuckRelease));
            let mut release = shown.release_ms;
            let response = value_ui.add(
                egui::DragValue::new(&mut release)
                    .range(0..=project::MAX_DUCK_MS)
                    .suffix(MS_SUFFIX),
            );
            self.remember("inspector.narration.ducking.release", &response);
            self.ducking_edit(
                &response,
                NarrationDucking {
                    release_ms: release,
                    ..shown
                },
                target,
            );
        });
    }

    /// Keep an edited ducking value as a draft and commit it once the field is released.
    fn ducking_edit(
        &mut self,
        response: &egui::Response,
        next: NarrationDucking,
        target: Option<ItemId>,
    ) {
        if response.changed() {
            self.state.narration.ducking_draft = Some(next);
        }
        if (response.drag_stopped() || response.lost_focus())
            && let Some(draft) = self.state.narration.ducking_draft.take()
        {
            self.actions.push(ducking_action(target, draft));
        }
    }

    fn cost_line(&self, cost: &Cost) -> String {
        let locale = self.cx.locale;
        match cost {
            Cost::Free => self.tr(Text::NarrationCostFree).to_owned(),
            Cost::UpperBound { currency, micros } => {
                let amount = format!(
                    "{} {currency}",
                    i18n::decimal(locale, *micros as f64 / 1_000_000.0, 4)
                );
                i18n::format(locale, Text::NarrationConsentCost, &[("cost", &amount)])
            }
            Cost::Unknown => self.tr(Text::ShortsUnknownCost).to_owned(),
        }
    }

    /// The exact-upload sheet: what is sent, to whom, at most what cost. The worker waits on it.
    pub(in super::super) fn narration_dialog(&mut self, ctx: &egui::Context) {
        let Some((upload, price_acknowledged)) = self.state.narration.pending() else {
            return;
        };
        let locale = self.cx.locale;
        let term = |name: &str| {
            upload
                .terms
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| value.clone())
                .unwrap_or_default()
        };
        let unknown = matches!(upload.cost, Cost::Unknown);
        let identity = i18n::format(
            locale,
            Text::NarrationConsentIdentity,
            &[
                ("provider", &upload.provider),
                ("model", &upload.model),
                ("voice", &term("voice")),
                ("language", &upload.languages),
            ],
        );
        let heading = i18n::format(
            locale,
            Text::NarrationConsentText,
            &[("count", &term("characters"))],
        );
        let cost = self.cost_line(&upload.cost);
        let speed = term("speed");
        let secondary = self.secondary();
        let mut answer = None;
        let modal = egui::Modal::new(Id::unique("narration.consent"))
            .frame(kit::dialog_frame(ctx))
            .show(ctx, |ui| {
                ui.set_width(400.0);
                kit::dialog_title(ui, self.tr(Text::NarrationConsentTitle));
                ui.label(identity.as_str());
                ui.label(egui::RichText::new(heading.as_str()).color(secondary));
                ui.add(egui::Label::new(term("text")).wrap_mode(egui::TextWrapMode::Wrap));
                if speed != "default" {
                    ui.label(i18n::format(
                        locale,
                        Text::NarrationConsentSpeed,
                        &[("speed", &speed)],
                    ));
                }
                ui.label(cost.as_str());
                let policy = term("policy");
                if policy.starts_with("https://") {
                    let link = ui.hyperlink_to(self.tr(Text::SubProviderPolicy), policy.as_str());
                    self.remember("narration.consent.policy", &link);
                }
                for name in ["retention", "training", "voice_disclosure"] {
                    ui.add(
                        egui::Label::new(egui::RichText::new(term(name)).color(secondary))
                            .wrap_mode(egui::TextWrapMode::Wrap),
                    );
                }
                let mut acknowledged = price_acknowledged;
                if unknown {
                    let check =
                        ui.checkbox(&mut acknowledged, self.tr(Text::ModelRemoteUnknownPrice));
                    self.remember("narration.consent.unknown_price", &check);
                    if check.changed() {
                        self.state.narration.acknowledge_price(acknowledged);
                    }
                }
                ui.horizontal(|ui| {
                    let send = ui
                        .add_enabled_ui(!unknown || acknowledged, |ui| {
                            kit::dialog_button(
                                ui,
                                "narration.consent.send",
                                self.tr(Text::NarrationConsentSend),
                                true,
                            )
                        })
                        .inner;
                    self.remember("narration.consent.send", &send);
                    if send.clicked() {
                        answer = Some(true);
                    }
                    let deny = kit::dialog_button(
                        ui,
                        "narration.consent.deny",
                        self.tr(Text::NarrationConsentDeny),
                        false,
                    );
                    self.remember("narration.consent.deny", &deny);
                    if deny.clicked() {
                        answer = Some(false);
                    }
                });
            });
        if modal.should_close() && answer.is_none() {
            answer = Some(false);
        }
        if let Some(approved) = answer {
            self.actions
                .push(Action::Narration(NarrationAction::Consent(approved)));
        }
    }
}

fn failure_text(failed: Failed) -> Text {
    match failed {
        Failed::Provider(Cause::Language) => Text::NarrationFailLanguage,
        Failed::Provider(Cause::Voice) => Text::NarrationFailVoice,
        Failed::Provider(Cause::Key) => Text::NarrationFailKey,
        Failed::Provider(Cause::Quota) => Text::NarrationFailQuota,
        Failed::PaidPlan(_) => Text::NarrationFailPaidPlan,
        Failed::Provider(Cause::Policy) => Text::NarrationFailPolicy,
        Failed::Provider(Cause::Rejected) => Text::NarrationFailRejected,
        Failed::Provider(Cause::Cost) => Text::NarrationFailCost,
        Failed::Provider(Cause::Network) => Text::NarrationFailNetwork,
        Failed::Provider(Cause::Cancelled) => Text::NarrationFailCancelled,
        Failed::Provider(Cause::Other) => Text::NarrationFailOther,
        Failed::Declined => Text::NarrationFailDeclined,
        Failed::EmptyText => Text::NarrationEmptyText,
        Failed::Stale => Text::NarrationFailStale,
        Failed::Storage => Text::NarrationFailStorage,
        Failed::Placement => Text::NarrationFailPlace,
    }
}

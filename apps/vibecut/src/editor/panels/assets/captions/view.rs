//! Paints the captions panel and its dialogs from the staged cue, style and job state.
use super::{Options, Picker, Repaint, Subtitles, key};
use crate::editor::panels::assets::CaptionsSection;
use crate::i18n;
use crate::{
    Error, Result, egui,
    i18n::{Locale, Text},
    services::lifecycle::CaptionOutput,
};
use crate::{
    kit as theme,
    kit::{self as icons, Icon},
};
use captions::LanguageRegistry;
use captions::LanguageTag;
use engine::providers::{CatalogOperation, Cost, provider_options_for};
use engine::transcript::Stage;
use engine::{Command, Session};
use project::{ItemId, ItemParams};
use std::{collections::BTreeSet, path::PathBuf, sync::Arc, time::Duration};
use time::{TICKS_PER_SECOND, Tick};

/// Target languages the translation and bilingual choices offer.
const TARGET_LANGUAGES: [&str; 4] = ["vi", "en", "zh-Hans", "zh-Hant"];
/// The Auto captions footer row: the 24 px primary button under a 12 px gap and an 8 px inset.
const AUTO_FOOTER_HEIGHT: f32 = theme::size::CONTROL + theme::space::LG + theme::space::MD;

fn output_languages(options: &Options, source: &LanguageTag) -> Result<Vec<LanguageTag>> {
    let target = LanguageTag::parse(&options.target)?;
    Ok(if options.bilingual && target != *source {
        vec![source.clone(), target]
    } else {
        vec![target]
    })
}

pub(super) struct Controls {
    models: crate::services::remote_models::AiPreferences,
    locale: Locale,
    widgets: Vec<(String, egui::Response)>,
}
impl Controls {
    pub(super) fn tr(&self, text: Text) -> &'static str {
        i18n::text(self.locale, text)
    }
    pub(super) fn remember(&mut self, id: impl Into<String>, r: &egui::Response) {
        let id = id.into();
        r.ctx.accesskit_node_builder(r.id, |n| n.set_author_id(&id));
        self.widgets.push((id, r.clone()));
    }
    pub(super) fn button(
        &mut self,
        ui: &mut egui::Ui,
        id: &str,
        text: Text,
        icon: Icon,
        enabled: bool,
    ) -> bool {
        // One explicit scope per control: the busy block above appears with the
        // async job, and without it every control below would derive a new
        // widget id and drop a press that straddles the update.
        let scope = ui.make_persistent_id(id);
        let r = ui
            .scope_builder(egui::UiBuilder::new().scope_id(scope), |ui| {
                ui.add_enabled(
                    enabled,
                    egui::Button::image_and_text(
                        icons::tinted(icon, 16.0, ui.visuals().text_color()),
                        self.tr(match text {
                            Text::SubImport => Text::Import,
                            Text::SubBurn => Text::SubMp4,
                            _ => text,
                        }),
                    )
                    .wrap_mode(egui::TextWrapMode::Extend)
                    .min_size(egui::vec2(28.0, 26.0)),
                )
            })
            .inner
            .on_hover_text(self.tr(text));
        self.remember(id, &r);
        r.clicked()
    }
    fn icon_button(
        &mut self,
        ui: &mut egui::Ui,
        id: &str,
        text: Text,
        icon: Icon,
        enabled: bool,
    ) -> bool {
        let scope = ui.make_persistent_id(id);
        let r = ui
            .scope_builder(egui::UiBuilder::new().scope_id(scope), |ui| {
                ui.add_enabled(
                    enabled,
                    egui::Button::image(icons::tinted(icon, 16.0, ui.visuals().text_color()))
                        .min_size(egui::vec2(28.0, 28.0)),
                )
            })
            .inner
            .on_hover_text(self.tr(text));
        r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, self.tr(text)));
        self.remember(id, &r);
        r.clicked()
    }
    fn language(&mut self, ui: &mut egui::Ui, id: &str, selected: &mut String, target: bool) {
        let r = egui::ComboBox::from_id_salt(id)
            .width(ui.available_width().max(30.0))
            .selected_text(language_name(self.locale, selected))
            .show_ui(ui, |ui| {
                let tags: &[&str] = if target {
                    &TARGET_LANGUAGES
                } else {
                    &["auto", "vi", "en", "zh"]
                };
                for tag in tags {
                    let r = ui.selectable_value(
                        selected,
                        (*tag).into(),
                        language_name(self.locale, tag),
                    );
                    self.remember(format!("{id}.{tag}"), &r);
                }
            })
            .response;
        r.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::Role::ComboBox,
                r.enabled(),
                self.tr(if target {
                    Text::SubTarget
                } else {
                    Text::ShortsSourceLanguage
                }),
            )
        });
        self.remember(id, &r);
    }
    fn provider(
        &mut self,
        ui: &mut egui::Ui,
        id: &str,
        provider: &mut String,
        model: &mut String,
        operation: CatalogOperation,
        source: &str,
    ) {
        let tag = (source != "auto")
            .then(|| LanguageTag::parse(source).ok())
            .flatten();
        let options = provider_options_for(operation, tag.as_ref());
        let current = options
            .iter()
            .find(|p| p.id == provider && (!p.local || p.models.contains(model)));
        let label = current
            .map(|p| format!("{} / {}", provider_name(self.locale, p), model))
            .unwrap_or_else(|| self.tr(Text::SubChooseProvider).to_owned());
        let r = egui::ComboBox::from_id_salt(id)
            .width(ui.available_width().max(30.0))
            .wrap_mode(egui::TextWrapMode::Wrap)
            .selected_text(label)
            .show_ui(ui, |ui| {
                for option in options {
                    let selected_models = if option.local {
                        option.models.clone()
                    } else {
                        vec![
                            self.models
                                .model(
                                    option.id,
                                    if operation == CatalogOperation::Transcribe {
                                        ai::Operation::Transcribe
                                    } else {
                                        ai::Operation::Translate
                                    },
                                )
                                .into(),
                        ]
                    };
                    for value in &selected_models {
                        let r = ui.selectable_label(
                            option.id == provider && value == model,
                            format!("{} / {}", provider_name(self.locale, &option), value),
                        );
                        self.remember(format!("{id}.{}.{}", option.id, value), &r);
                        if r.clicked() {
                            *provider = option.id.into();
                            *model = value.clone();
                        }
                    }
                }
            })
            .response;
        r.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::Role::ComboBox,
                r.enabled(),
                self.tr(if operation == CatalogOperation::Translate {
                    Text::SubTranslator
                } else {
                    Text::ShortsTranscriber
                }),
            )
        });
        self.remember(id, &r);
    }
}
fn provider_name(locale: Locale, provider: &engine::providers::ProviderOption) -> String {
    let name = provider.label.split(" (").next().unwrap_or(provider.id);
    if provider.local {
        i18n::format(locale, Text::SubProviderLocal, &[("provider", name)])
    } else {
        name.to_owned()
    }
}
fn language_name(locale: Locale, tag: &str) -> String {
    let key = match tag {
        "auto" => Text::SubAuto,
        "vi" => Text::SubVietnamese,
        "en" => Text::SubEnglish,
        "zh-Hans" => Text::SubHans,
        "zh-Hant" => Text::SubHant,
        _ => Text::SubChinese,
    };
    i18n::text(locale, key).into()
}
fn style_row(ui: &mut egui::Ui, label: &str, value: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(label).size(theme::type_size::BODY));
        value(ui);
    });
}
/// One labelled style control whose response receives a stable automation id.
fn style_control(
    ui: &mut egui::Ui,
    label: &str,
    value: impl FnOnce(&mut egui::Ui) -> egui::Response,
) -> egui::Response {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(label).size(theme::type_size::BODY));
        value(ui)
    })
    .inner
}

impl Subtitles {
    /// The assets-panel mount is one call. Engine edits remain in this module.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::editor) fn ui(
        &mut self,
        ui: &mut egui::Ui,
        session: &mut Session,
        options: &mut Options,
        at: Tick,
        locale: Locale,
        dark: bool,
        bank_ready: bool,
        section: CaptionsSection,
    ) -> Vec<(String, egui::Response)> {
        self.refresh(session);
        let frame = ui.ctx().cumulative_frame_nr();
        let continuous = self.bank_frame.is_none_or(|previous| frame <= previous + 1);
        self.reconcile_bank(session, at, bank_ready, continuous);
        self.reconcile_style_edit(session, at, bank_ready, continuous);
        self.bank_frame = Some(frame);
        // The spoken-language choice belongs to the project, not the window.
        // A project without an entry starts from Auto; the first render after
        // construction keeps an explicit caller choice for compatibility.
        let project = session.project().data().id;
        if let Some(previous) = self.project
            && previous != project
        {
            options.load_source(project);
        }
        self.project = Some(project);
        if matches!(options.transcription.as_str(), "openai" | "elevenlabs") {
            options.transcription_model = self
                .models
                .model(&options.transcription, ai::Operation::Transcribe)
                .into();
        }
        options.translation_model = self
            .models
            .model(&options.text_provider, ai::Operation::Translate)
            .into();
        options.fillers_model = self
            .models
            .model(&options.text_provider, ai::Operation::ClassifyFillers)
            .into();
        let previous_options = options.clone();
        let mut controls = Controls {
            models: self.models.clone(),
            locale,
            widgets: Vec::new(),
        };
        self.poll_picker(session, options, at);
        let busy = self.busy();
        let footer = section == CaptionsSection::Auto;
        let scroll_height = if footer {
            (ui.available_height() - AUTO_FOOTER_HEIGHT).max(0.0)
        } else {
            ui.available_height()
        };
        ui.vertical(|ui| {
            egui::ScrollArea::vertical()
                .id_salt("subtitles.panel.scroll")
                .max_height(scroll_height)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.set_max_width(ui.available_width());
                    ui.spacing_mut().item_spacing.x = 4.0;
                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                    match section {
                        CaptionsSection::Auto => {
                            self.auto_page(ui, options, &mut controls, busy);
                        }
                        CaptionsSection::Add => {
                            self.add_page(ui, session, &mut controls, locale, busy)
                        }
                        CaptionsSection::Translate => {
                            self.translate_page(ui, session, options, &mut controls, busy);
                        }
                    }
                    if matches!(
                        options.transcription.as_str(),
                        "whisper.cpp" | "whisper" | "gipformer"
                    ) {
                        options.local_transcription_model = options.transcription_model.clone();
                    }
                    if options.source != previous_options.source {
                        options.remember_source(project);
                    }
                    if *options != previous_options {
                        let result = self.flush_drafts(session).map(|_| ());
                        self.ui_result(result);
                    }
                    if let Some(problem) = self.problem {
                        ui.colored_label(theme::error_text(dark), controls.tr(problem));
                        if matches!(problem, Text::SubNoSpeech | Text::SubLowConfidence)
                            && options.source != "auto"
                            && self.retry_source.is_some()
                            && controls.button(
                                ui,
                                "subtitles.retry_auto",
                                Text::SubRetryAuto,
                                Icon::ArrowClockwise,
                                !busy,
                            )
                        {
                            let result = self
                                .flush_drafts(session)
                                .and_then(|_| self.retry_auto(session, options));
                            self.ui_result(result);
                        }
                        if problem == Text::SubDraftConflict && self.caption(session).is_none() {
                            let label = ui.label(controls.tr(Text::SubDraftRecovery));
                            for ((id, language), draft) in &mut self.drafts {
                                let r = ui
                                    .add(
                                        egui::TextEdit::multiline(draft)
                                            .desired_rows(2)
                                            .char_limit(64 * 1024)
                                            .desired_width(ui.available_width())
                                            .id_salt((
                                                "subtitles.recovery",
                                                id.as_str(),
                                                language.as_str(),
                                            )),
                                    )
                                    .labelled_by(label.id);
                                if r.has_focus() {
                                    self.text_focus = Some(r.id);
                                }
                                controls.remember(
                                    format!(
                                        "subtitles.recovery.{}.{}",
                                        id.as_str(),
                                        language.as_str()
                                    ),
                                    &r,
                                );
                            }
                        }
                        if let Some(id) = self.missing_model.clone()
                            && controls.button(
                                ui,
                                "subtitles.models",
                                Text::Models,
                                Icon::File,
                                !busy,
                            )
                        {
                            self.requested_model = Some(id);
                        }
                        if controls.button(
                            ui,
                            "subtitles.dismiss",
                            Text::ShortsDismiss,
                            Icon::X,
                            true,
                        ) {
                            if self.problem == Some(Text::SubDraftConflict) {
                                self.drafts.clear();
                                self.draft_project = Some(session.project().clone());
                            }
                            self.problem = None;
                            self.completed = None;
                        }
                    } else if let Some(count) = self.completed {
                        ui.label(i18n::format(
                            locale,
                            Text::SubDone,
                            &[("count", &i18n::number(locale, count as u64))],
                        ));
                    }
                    if busy {
                        let snapshot = self.inspect(session);
                        ui.spinner();
                        if let Some(job) = self
                            .export
                            .as_ref()
                            .and_then(|(id, _)| self.runner.snapshot(*id))
                        {
                            crate::services::progress::job_progress(
                                ui,
                                "subtitles.progress",
                                &job,
                                locale,
                            );
                        } else {
                            let status = controls.tr(if snapshot.cancelling {
                                Text::ShortsCancelling
                            } else {
                                stage_text(snapshot.stage)
                            });
                            if let Some(progress) = snapshot.progress {
                                crate::services::progress::line(
                                    ui,
                                    "subtitles.progress",
                                    locale,
                                    status,
                                    progress,
                                );
                            } else {
                                ui.label(status);
                            }
                        }
                        if controls.button(
                            ui,
                            "subtitles.cancel",
                            Text::Cancel,
                            Icon::X,
                            !snapshot.cancelling,
                        ) {
                            self.cancel();
                        }
                        ui.ctx().request_repaint_after(Duration::from_millis(20));
                    }
                    ui.separator();
                    if self.item.is_none() {
                        self.text_focus = None;
                        ui.label(controls.tr(Text::SubEmpty));
                        self.style_ui(ui, session, options, at, bank_ready, &mut controls);
                        // Words picked in another scene stay selected here, so the pane still
                        // answers a cut with its scene refusal instead of vanishing.
                        self.transcript_ui(
                            ui,
                            session,
                            controls.locale,
                            dark,
                            &mut controls.widgets,
                        );
                        return;
                    }
                    let items: Vec<_> = session
                        .project()
                        .data()
                        .scene()
                        .timeline
                        .iter()
                        .flat_map(|t| &t.items)
                        .filter(|i| matches!(i.content, ItemParams::Caption(_)))
                        .map(|i| i.id)
                        .collect();
                    if items.len() > 1 {
                        let mut selected = self.item;
                        let r = egui::ComboBox::from_id_salt(egui::Id::unique("subtitles.item"))
                            .selected_text(i18n::format(
                                locale,
                                Text::SubItem,
                                &[("id", &self.item.map_or(String::new(), |i| i.0.to_string()))],
                            ))
                            .show_ui(ui, |ui| {
                                for item in items {
                                    let r = ui.selectable_value(
                                        &mut selected,
                                        Some(item),
                                        i18n::format(
                                            locale,
                                            Text::SubItem,
                                            &[("id", &item.0.to_string())],
                                        ),
                                    );
                                    controls.remember(format!("subtitles.item.{}", item.0), &r);
                                }
                            })
                            .response;
                        controls.remember("subtitles.item", &r);
                        if selected != self.item
                            && let Some(item) = selected
                        {
                            let result = self
                                .flush_drafts(session)
                                .and_then(|_| self.select_item(session, item));
                            self.ui_result(result);
                        }
                    }
                    self.style_ui(ui, session, options, at, bank_ready, &mut controls);
                    self.text_focus = None;
                    ui.add_enabled_ui(
                        !busy && self.bank.is_none() && self.style_edit.is_none(),
                        |ui| {
                            ui.horizontal_wrapped(|ui| {
                                if controls.icon_button(
                                    ui,
                                    "subtitles.select_all",
                                    Text::SubSelectAll,
                                    Icon::Plus,
                                    true,
                                ) && let Some((_, p)) = self.caption(session)
                                {
                                    self.selected =
                                        p.cues.cues.iter().map(|c| c.id.clone()).collect();
                                }
                                if controls.icon_button(
                                    ui,
                                    "subtitles.split",
                                    Text::Split,
                                    Icon::Scissors,
                                    self.selected.len() == 1,
                                ) {
                                    let result = self
                                        .flush_drafts(session)
                                        .and_then(|_| self.split(session, at));
                                    self.ui_result(result);
                                }
                                if controls.icon_button(
                                    ui,
                                    "subtitles.merge",
                                    Text::SubMerge,
                                    Icon::ArrowsInLineHorizontal,
                                    self.selected.len() > 1,
                                ) {
                                    let result = self
                                        .flush_drafts(session)
                                        .and_then(|_| self.merge(session));
                                    self.ui_result(result);
                                }
                                if controls.icon_button(
                                    ui,
                                    "subtitles.earlier",
                                    Text::SubEarlier,
                                    Icon::SkipBack,
                                    !self.selected.is_empty(),
                                ) {
                                    let result = self
                                        .flush_drafts(session)
                                        .and_then(|_| self.nudge(session, false));
                                    self.ui_result(result);
                                }
                                if controls.icon_button(
                                    ui,
                                    "subtitles.later",
                                    Text::SubLater,
                                    Icon::SkipForward,
                                    !self.selected.is_empty(),
                                ) {
                                    let result = self
                                        .flush_drafts(session)
                                        .and_then(|_| self.nudge(session, true));
                                    self.ui_result(result);
                                }
                            });
                            self.cues_ui(ui, session, options, &mut controls, dark);
                            self.transcript_ui(
                                ui,
                                session,
                                controls.locale,
                                dark,
                                &mut controls.widgets,
                            );
                        },
                    );
                    if self.identify_fillers {
                        self.fillers_ui(ui, session, &mut controls, locale, options);
                    }
                });
            if footer {
                self.auto_footer(ui, session, options, &mut controls, busy);
            }
        });
        self.bank_ui(ui, session, at, &mut controls);
        self.style_editor_ui(ui, session, at, &mut controls);
        controls.widgets
    }
    /// The consent sheet, the only modal the panel still owns.
    pub(crate) fn dialogs(
        &mut self,
        ctx: &egui::Context,
        locale: Locale,
    ) -> Vec<(String, egui::Response)> {
        let mut controls = Controls {
            models: self.models.clone(),
            locale,
            widgets: Vec::new(),
        };
        self.consent_ui(ctx, &mut controls);
        controls.widgets
    }
    /// Auto captions: spoken language, the bilingual choice, the filler switch and our
    /// transcription service below them.
    fn auto_page(
        &mut self,
        ui: &mut egui::Ui,
        options: &mut Options,
        c: &mut Controls,
        busy: bool,
    ) {
        ui.add_enabled_ui(!busy, |ui| {
            ui.label(c.tr(Text::SubSpokenLanguage));
            c.language(ui, "subtitles.source", &mut options.source, false);
            ui.label(c.tr(Text::SubBilingualCaptions));
            self.bilingual_select(ui, "subtitles.bilingual.language", options, c);
            let r = crate::kit::toggle(
                ui,
                "subtitles.identify_fillers",
                &mut self.identify_fillers,
                c.tr(Text::SubIdentifyFillers),
            );
            c.remember("subtitles.identify_fillers", &r);
            ui.label(c.tr(Text::ShortsTranscriber));
            c.provider(
                ui,
                "subtitles.transcriber",
                &mut options.transcription,
                &mut options.transcription_model,
                CatalogOperation::Transcribe,
                &options.source,
            );
        });
    }
    /// The second-language choice: Off, or the language the bilingual export and the
    /// translation target share.
    fn bilingual_select(
        &mut self,
        ui: &mut egui::Ui,
        id: &str,
        options: &mut Options,
        c: &mut Controls,
    ) {
        let before = options.bilingual.then(|| options.target.clone());
        let mut choice = before.clone();
        let off = c.tr(Text::SubBilingualOff);
        let locale = c.locale;
        let shown = choice
            .as_deref()
            .map_or_else(|| off.to_owned(), |tag| language_name(locale, tag));
        let r = egui::ComboBox::from_id_salt(id)
            .width(ui.available_width().max(30.0))
            .selected_text(shown)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut choice, None, off);
                for tag in TARGET_LANGUAGES {
                    ui.selectable_value(
                        &mut choice,
                        Some(tag.to_owned()),
                        language_name(locale, tag),
                    );
                }
            })
            .response;
        r.widget_info(|| {
            egui::WidgetInfo::labeled(
                egui::Role::ComboBox,
                r.enabled(),
                c.tr(Text::SubBilingualCaptions),
            )
        });
        c.remember("subtitles.bilingual.language", &r);
        if choice != before {
            match choice {
                Some(tag) => {
                    options.bilingual = true;
                    options.target = tag;
                }
                None => options.bilingual = false,
            }
        }
    }
    /// Auto captions footer: the delete-current checkbox at the left, the primary button at
    /// the right. The button is disabled with its reason as its tooltip.
    fn auto_footer(
        &mut self,
        ui: &mut egui::Ui,
        session: &mut Session,
        options: &mut Options,
        c: &mut Controls,
        busy: bool,
    ) {
        let source = source_clip(session);
        let reason = if busy {
            Some(Text::SubTranscribing)
        } else if source.is_none() {
            Some(Text::SubSelectMedia)
        } else if !options.transcription_supported() {
            Some(Text::SubChooseProvider)
        } else {
            None
        };
        ui.horizontal(|ui| {
            let r = ui.checkbox(&mut self.delete_current, c.tr(Text::SubDeleteCurrent));
            c.remember("subtitles.delete_current", &r);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let r = ui.add_enabled(
                    reason.is_none(),
                    egui::Button::new(c.tr(Text::SubGenerate))
                        .min_size(egui::vec2(0.0, theme::size::CONTROL)),
                );
                let r = match reason {
                    Some(text) => r.on_disabled_hover_text(c.tr(text)),
                    None => r,
                };
                c.remember("subtitles.generate", &r);
                if r.clicked()
                    && let Some(source) = source
                {
                    let result = self
                        .flush_drafts(session)
                        .and_then(|_| self.generate(session, source, options));
                    self.ui_result(result);
                }
            });
        });
    }
    /// Add captions: file import. Manual caption entry is not offered here.
    fn add_page(
        &mut self,
        ui: &mut egui::Ui,
        session: &mut Session,
        c: &mut Controls,
        locale: Locale,
        busy: bool,
    ) {
        let enabled = !busy && self.picker.is_none();
        let r = ui.add_enabled(
            enabled,
            egui::Button::new(c.tr(Text::SubImport))
                .min_size(egui::vec2(0.0, theme::size::CONTROL)),
        );
        c.remember("subtitles.import", &r);
        if r.clicked() {
            self.prepare_import(ui.ctx(), session, locale);
        }
    }
    /// Translate: target language, translator, the replace-edited choice, the translate
    /// action and the glossary.
    fn translate_page(
        &mut self,
        ui: &mut egui::Ui,
        session: &mut Session,
        options: &mut Options,
        c: &mut Controls,
        busy: bool,
    ) {
        ui.add_enabled_ui(!busy, |ui| {
            ui.label(c.tr(Text::SubTarget));
            c.language(ui, "subtitles.target", &mut options.target, true);
            ui.label(c.tr(Text::SubTranslator));
            c.provider(
                ui,
                "subtitles.translator",
                &mut options.text_provider,
                &mut options.translation_model,
                CatalogOperation::Translate,
                &options.source,
            );
            let r = ui.checkbox(&mut self.force, c.tr(Text::SubReplaceEdited));
            c.remember("subtitles.force", &r);
            if c.button(
                ui,
                "subtitles.translate",
                Text::SubTranslate,
                Icon::Translate,
                !self.selected.is_empty() && self.item.is_some() && options.validate().is_ok(),
            ) {
                let result = self
                    .flush_drafts(session)
                    .and_then(|_| self.translate(session, options));
                self.ui_result(result);
            }
            self.glossary_ui(ui, c);
        });
    }
    fn ui_result(&mut self, result: Result<()>) {
        if let Err(e) = result {
            match e {
                Error::Engine(e) => self.fail(Arc::new(e)),
                _ => {
                    if self.problem != Some(Text::SubDraftConflict) {
                        self.problem = Some(Text::SubEditFailed);
                    }
                }
            }
        }
    }
    fn glossary_ui(&mut self, ui: &mut egui::Ui, c: &mut Controls) {
        let r = ui.label(egui::RichText::new(c.tr(Text::SubGlossary)).strong());
        c.remember("subtitles.glossary", &r);
        ui.vertical(|ui| {
            let mut remove = None;
            for (i, term) in self.glossary.iter_mut().enumerate() {
                let mut source = term.term.as_str().to_owned();
                let mut target = term
                    .translation
                    .as_ref()
                    .map_or(String::new(), |t| t.as_str().to_owned());
                let label = ui.label(c.tr(Text::SubTerm));
                let r = ui.add(
                    egui::TextEdit::singleline(&mut source)
                        .id_salt(("subtitles.glossary.term", i))
                        .hint_text(c.tr(Text::SubTerm))
                        .char_limit(256)
                        .desired_width(ui.available_width()),
                );
                if r.has_focus() {
                    self.text_focus = Some(r.id);
                }
                let r = r.labelled_by(label.id);
                c.remember(format!("subtitles.glossary.{i}.term"), &r);
                if r.changed() {
                    term.term = source.into();
                }
                let label = ui.label(c.tr(Text::SubTermTranslation));
                let r = ui.add(
                    egui::TextEdit::singleline(&mut target)
                        .id_salt(("subtitles.glossary.translation", i))
                        .hint_text(c.tr(Text::SubTermTranslation))
                        .char_limit(256)
                        .desired_width(ui.available_width()),
                );
                if r.has_focus() {
                    self.text_focus = Some(r.id);
                }
                let r = r.labelled_by(label.id);
                c.remember(format!("subtitles.glossary.{i}.translation"), &r);
                if r.changed() {
                    term.translation = (!target.is_empty()).then(|| target.into());
                }
                if c.button(
                    ui,
                    &format!("subtitles.glossary.{i}.remove"),
                    Text::Delete,
                    Icon::Trash,
                    true,
                ) {
                    remove = Some(i);
                }
            }
            if let Some(i) = remove {
                self.glossary.remove(i);
            }
            if c.button(
                ui,
                "subtitles.glossary.add",
                Text::SubAddTerm,
                Icon::Plus,
                self.glossary.len() < 128,
            ) {
                self.glossary.push(ai::GlossaryTerm {
                    term: "".into(),
                    translation: None,
                });
            }
        });
    }
    fn style_ui(
        &mut self,
        ui: &mut egui::Ui,
        session: &mut Session,
        options: &mut Options,
        at: Tick,
        bank_ready: bool,
        c: &mut Controls,
    ) {
        ui.horizontal_wrapped(|ui| {
            for (id, text, save) in [
                ("subtitles.style.save", Text::SubBankSave, true),
                ("subtitles.style.manage", Text::SubBankManage, false),
            ] {
                if save && self.caption(session).is_none() {
                    continue;
                }
                if !save && session.project().data().caption_presets.is_empty() {
                    continue;
                }
                let r = ui.add_enabled(
                    bank_ready && self.bank_available(session, save),
                    egui::Button::new(egui::RichText::new(c.tr(text)).size(theme::type_size::BODY))
                        .min_size(egui::vec2(0.0, theme::size::FIELD)),
                );
                c.remember(id, &r);
                if r.clicked() {
                    let result = self.activate(session, options, None, at, id);
                    self.ui_result(result);
                }
            }
        });
        let Some((item, params)) = self.caption(session) else {
            return;
        };
        if params.cues.cues.is_empty() {
            return;
        }
        let highlighted = params.word_highlight;
        let item = item.id;
        let selected = params.preset;
        let patched = !params.overrides.is_empty();
        let first = params
            .cues
            .cues
            .first()
            .and_then(|cue| params.cues.style_of(cue));
        let label = if let Some(preset) = selected {
            let name = session
                .project()
                .data()
                .caption_presets
                .iter()
                .find(|value| value.id == preset)
                .map_or("", |value| value.name.as_str());
            if patched {
                format!("{name} · {}", c.tr(Text::SubStyleEdited))
            } else {
                name.to_owned()
            }
        } else if params
            .cues
            .cues
            .iter()
            .any(|cue| params.cues.style_of(cue) != first)
        {
            c.tr(Text::SubStyleMixed).to_owned()
        } else if first.is_some_and(|style| engine::CaptionPreset::Outline.matches(style)) {
            c.tr(Text::SubStyleOutline).to_owned()
        } else if first.is_some_and(|style| engine::CaptionPreset::Box.matches(style)) {
            c.tr(Text::SubStyleBox).to_owned()
        } else {
            c.tr(Text::SubStyleCustom).to_owned()
        };
        let snapshot = session.project().clone();
        let enabled = self.style_available(session);
        let mut chosen = None;
        let mut saved = None;
        let mut overrides_open = false;
        ui.add_enabled_ui(enabled, |ui| {
            ui.horizontal(|ui| {
                let caption = ui
                    .label(egui::RichText::new(c.tr(Text::SubStyle)).size(theme::type_size::BODY));
                ui.spacing_mut().interact_size.y = theme::size::FIELD;
                let response = egui::ComboBox::from_id_salt((
                    "subtitles.style",
                    snapshot.data().id,
                    snapshot.data().current_scene_id,
                    item,
                    Arc::as_ptr(&snapshot),
                ))
                .selected_text(label.clone())
                .show_ui(ui, |ui| {
                    for (id, text) in [
                        ("subtitles.style.outline", Text::SubStyleOutline),
                        ("subtitles.style.box", Text::SubStyleBox),
                    ] {
                        let response = ui.selectable_label(label == c.tr(text), c.tr(text));
                        c.remember(id, &response);
                        if response.clicked() {
                            chosen = Some(id);
                            ui.close();
                        }
                    }
                    if !snapshot.data().caption_presets.is_empty() {
                        ui.separator();
                        ui.label(
                            egui::RichText::new(c.tr(Text::SubBankSaved))
                                .size(theme::type_size::BODY),
                        );
                        for preset in &snapshot.data().caption_presets {
                            let r = ui.add_enabled(
                                bank_ready,
                                egui::Button::new(preset.name.as_str())
                                    .selected(selected == Some(preset.id)),
                            );
                            c.remember(format!("subtitles.style.saved.{}", preset.id.0), &r);
                            if r.clicked() {
                                saved = Some(preset.id);
                                ui.close();
                            }
                        }
                    }
                })
                .response
                .labelled_by(caption.id);
                c.remember("subtitles.style", &response);
                if selected.is_some() {
                    let r = ui.add_enabled(
                        bank_ready,
                        egui::Button::new(
                            egui::RichText::new(c.tr(Text::SubStyleOverrides))
                                .size(theme::type_size::BODY),
                        )
                        .min_size(egui::vec2(0.0, theme::size::FIELD)),
                    );
                    c.remember("subtitles.style.overrides", &r);
                    if r.clicked() {
                        overrides_open = true;
                    }
                }
            });
        });
        if overrides_open {
            let result = self.activate(session, options, None, at, "subtitles.style.overrides");
            self.ui_result(result);
        }
        let mut highlighted = highlighted;
        let response = ui
            .add_enabled_ui(enabled && bank_ready, |ui| {
                ui.add_sized(
                    egui::vec2(ui.available_width(), theme::size::FIELD),
                    egui::Checkbox::new(&mut highlighted, c.tr(Text::SubWordHighlight)),
                )
            })
            .inner
            .on_hover_text(c.tr(Text::SubWordHint));
        c.remember("subtitles.style.word_highlight", &response);
        if response.changed() {
            chosen = Some("subtitles.style.word_highlight");
        }
        let languages = snapshot
            .data()
            .scene()
            .timeline
            .iter()
            .flat_map(|track| &track.items)
            .find(|value| value.id == item)
            .and_then(|item| match &item.content {
                ItemParams::Caption(params) => params.cues.cues.first().map(|cue| &cue.source_lang),
                _ => None,
            })
            .and_then(|source| output_languages(options, source).ok())
            .unwrap_or_default();
        let used = self
            .budget
            .snapshot()
            .map_or(u64::MAX, |snapshot| snapshot.limited_used());
        if self.word_warnings.as_ref().is_none_or(|state| {
            !Arc::ptr_eq(&state.project, &snapshot)
                || state.item != item
                || state.languages != languages
                || state.fonts != text::generation()
                || ((state.preview.is_err() || state.export.is_err()) && state.used != used)
        }) {
            let read = |settings| {
                render::Renderer::new(&snapshot, settings)
                    .and_then(|renderer| {
                        renderer.caption_word_warnings(
                            Some(item),
                            &engine::ByteBudgetHooks::new(&self.budget, None),
                        )
                    })
                    .map_err(Arc::new)
            };
            let preview = read(render::RenderSettings::from_project(&snapshot));
            let mut settings = render::RenderSettings::from_project(&snapshot);
            settings.caption_languages = languages.clone();
            let export = read(settings);
            self.word_warnings = Some(super::WordWarnings {
                project: snapshot.clone(),
                item,
                languages,
                fonts: text::generation(),
                used,
                preview,
                export,
            });
        }
        if let Some(state) = &self.word_warnings {
            let same = match (&state.preview, &state.export) {
                (Ok(preview), Ok(export)) => preview == export,
                (Err(_), Err(_)) => true,
                _ => false,
            };
            for (scope, prefix, warnings) in [
                (
                    "preview",
                    if same { None } else { Some(Text::Preview) },
                    &state.preview,
                ),
                ("export", Some(Text::Export), &state.export),
            ]
            .into_iter()
            .take(if same { 1 } else { 2 })
            {
                match warnings {
                    Ok(warnings) => {
                        for warning in &warnings.warnings {
                            let key = match warning.reason {
                                captions::WordHighlightReason::MissingTiming => {
                                    Text::SubWordMissing
                                }
                                captions::WordHighlightReason::SourceChanged => {
                                    Text::SubWordChanged
                                }
                                captions::WordHighlightReason::SourceNotDisplayed => {
                                    Text::SubWordLanguage
                                }
                                captions::WordHighlightReason::AmbiguousTiming => {
                                    Text::SubWordAmbiguous
                                }
                                captions::WordHighlightReason::ShapedBoundary => {
                                    Text::SubWordShaped
                                }
                            };
                            let message = prefix.map_or_else(
                                || c.tr(key).to_owned(),
                                |prefix| format!("{}: {}", c.tr(prefix), c.tr(key)),
                            );
                            let response = ui.label(message);
                            c.remember(
                                format!(
                                    "subtitles.word_highlight.warning.{scope}.{:?}",
                                    warning.reason
                                ),
                                &response,
                            );
                        }
                    }
                    Err(error) => {
                        ui.label(c.tr(Text::SubWordUnavailable))
                            .on_hover_text(error.to_string());
                    }
                }
            }
        }
        if let Some(id) = chosen
            && self.item == Some(item)
            && Arc::ptr_eq(&snapshot, session.project())
        {
            let result = self.activate(session, options, None, at, id);
            self.ui_result(result);
        }
        if let Some(preset) = saved
            && self.item == Some(item)
            && Arc::ptr_eq(&snapshot, session.project())
            && bank_ready
            && self.style_available(session)
        {
            let result = session
                .apply(Command::ApplyCaptionStyle {
                    item,
                    style: engine::CaptionStyleApply::Saved {
                        preset,
                        overrides: captions::CaptionOverrides::default(),
                    },
                })
                .map(|_| ())
                .map_err(Error::from);
            self.ui_result(result);
        }
    }

    fn style_editor_ui(
        &mut self,
        parent: &egui::Ui,
        session: &mut Session,
        at: Tick,
        c: &mut Controls,
    ) {
        let Some(edit) = &mut self.style_edit else {
            return;
        };
        let mut apply = false;
        let mut reset = false;
        let mut cancel = parent.input(|i| i.key_pressed(egui::Key::Escape));
        egui::Area::new(egui::Id::unique("subtitles.style.editor"))
            .order(egui::Order::Foreground)
            .fixed_pos(parent.min_rect().min + egui::vec2(8.0, 56.0))
            .show(parent.ctx(), |ui| {
                egui::Frame::popup(ui.style())
                    .inner_margin(theme::space::SM)
                    .show(ui, |ui| {
                        ui.set_width(264.0);
                        ui.spacing_mut().interact_size.y = theme::size::FIELD;
                        ui.label(
                            egui::RichText::new(c.tr(Text::SubStyleOverrides))
                                .size(theme::type_size::BODY),
                        );
                        let r = style_control(ui, c.tr(Text::SubStyleFont), |ui| {
                            ui.add_sized(
                                egui::vec2(160.0, theme::size::FIELD),
                                egui::TextEdit::singleline(&mut edit.draft.font)
                                    .font(egui::FontId::proportional(theme::type_size::BODY)),
                            )
                        });
                        c.remember("subtitles.style.font", &r);
                        let r = style_control(ui, c.tr(Text::SubStyleFontStyle), |ui| {
                            ui.add_sized(
                                egui::vec2(160.0, theme::size::FIELD),
                                egui::TextEdit::singleline(&mut edit.draft.font_style)
                                    .font(egui::FontId::proportional(theme::type_size::BODY)),
                            )
                        });
                        c.remember("subtitles.style.font_style", &r);
                        let r = style_control(ui, c.tr(Text::SubStyleSize), |ui| {
                            ui.add(
                                egui::DragValue::new(&mut edit.draft.size)
                                    .range(4.0..=256.0)
                                    .speed(1.0),
                            )
                        });
                        c.remember("subtitles.style.size", &r);
                        style_row(ui, c.tr(Text::BlendColor), |ui| {
                            for channel in edit.draft.color.iter_mut() {
                                ui.add(egui::DragValue::new(channel).range(0..=255));
                            }
                        });
                        let r = style_control(ui, c.tr(Text::Position), |ui| {
                            egui::ComboBox::from_id_salt(egui::Id::unique(
                                "subtitles.style.position",
                            ))
                            .selected_text(match edit.draft.position {
                                captions::CaptionPosition::Top => c.tr(Text::SubStyleTop),
                                captions::CaptionPosition::Middle => c.tr(Text::SubStyleMiddle),
                                captions::CaptionPosition::Bottom => c.tr(Text::SubStyleBottom),
                            })
                            .show_ui(ui, |ui| {
                                for (position, text) in [
                                    (captions::CaptionPosition::Top, Text::SubStyleTop),
                                    (captions::CaptionPosition::Middle, Text::SubStyleMiddle),
                                    (captions::CaptionPosition::Bottom, Text::SubStyleBottom),
                                ] {
                                    ui.selectable_value(
                                        &mut edit.draft.position,
                                        position,
                                        c.tr(text),
                                    );
                                }
                            })
                            .response
                        });
                        c.remember("subtitles.style.position", &r);
                        let r = style_control(ui, c.tr(Text::SubStyleSafeArea), |ui| {
                            ui.add(
                                egui::DragValue::new(&mut edit.draft.safe_area_percent)
                                    .range(0.0..=40.0)
                                    .speed(0.5),
                            )
                        });
                        c.remember("subtitles.style.safe_area", &r);
                        let r = style_control(ui, c.tr(Text::SubStyleMaxLines), |ui| {
                            ui.add(egui::DragValue::new(&mut edit.draft.max_lines).range(1..=16))
                        });
                        c.remember("subtitles.style.max_lines", &r);
                        let mut outline = edit.draft.outline.is_some();
                        let r = ui.checkbox(
                            &mut outline,
                            egui::RichText::new(c.tr(Text::SubStyleOutline))
                                .size(theme::type_size::BODY),
                        );
                        c.remember("subtitles.style.outline", &r);
                        if r.changed() {
                            let base_outline = edit.base.outline.clone();
                            let replacement = outline.then(|| {
                                base_outline.unwrap_or(captions::CaptionOutline {
                                    color: [0, 0, 0, 255],
                                    width: 2.0,
                                })
                            });
                            edit.draft.outline = replacement;
                        }
                        if let Some(value) = &mut edit.draft.outline {
                            let r = style_control(ui, c.tr(Text::SubStyleOutlineWidth), |ui| {
                                ui.add(
                                    egui::DragValue::new(&mut value.width)
                                        .range(0.0..=64.0)
                                        .speed(0.5),
                                )
                            });
                            c.remember("subtitles.style.outline_width", &r);
                            style_row(ui, c.tr(Text::BlendColor), |ui| {
                                for channel in value.color.iter_mut() {
                                    ui.add(egui::DragValue::new(channel).range(0..=255));
                                }
                            });
                        }
                        let mut background = edit.draft.background.is_some();
                        let r = ui.checkbox(
                            &mut background,
                            egui::RichText::new(c.tr(Text::SubStyleBox))
                                .size(theme::type_size::BODY),
                        );
                        c.remember("subtitles.style.background", &r);
                        if r.changed() {
                            let base_background = edit.base.background.clone();
                            let replacement = background.then(|| {
                                base_background.unwrap_or(captions::CaptionBackground {
                                    color: [0, 0, 0, 255],
                                    padding: 8.0,
                                    radius: 0.0,
                                })
                            });
                            edit.draft.background = replacement;
                        }
                        if let Some(value) = &mut edit.draft.background {
                            let r = style_control(ui, c.tr(Text::SubStylePadding), |ui| {
                                ui.add(egui::DragValue::new(&mut value.padding).range(0.0..=128.0))
                            });
                            c.remember("subtitles.style.padding", &r);
                            let r = style_control(ui, c.tr(Text::SubStyleRadius), |ui| {
                                ui.add(egui::DragValue::new(&mut value.radius).range(0.0..=128.0))
                            });
                            c.remember("subtitles.style.radius", &r);
                            style_row(ui, c.tr(Text::BlendColor), |ui| {
                                for channel in value.color.iter_mut() {
                                    ui.add(egui::DragValue::new(channel).range(0..=255));
                                }
                            });
                        }
                        ui.horizontal(|ui| {
                            for (id, text, intent) in [
                                ("subtitles.style.apply", Text::SubStyleApply, 0),
                                ("subtitles.style.reset", Text::ResetValue, 1),
                                ("subtitles.style.cancel", Text::Cancel, 2),
                            ] {
                                let r = ui.button(
                                    egui::RichText::new(c.tr(text)).size(theme::type_size::BODY),
                                );
                                c.remember(id, &r);
                                if r.clicked() {
                                    match intent {
                                        0 => apply = true,
                                        1 => reset = true,
                                        _ => cancel = true,
                                    }
                                }
                            }
                        });
                    });
            });
        if cancel {
            self.style_edit = None;
        } else if apply || reset {
            let result = self.confirm_style_edit(session, at, reset);
            self.ui_result(result);
        }
    }

    fn bank_ui(&mut self, parent: &egui::Ui, session: &mut Session, at: Tick, c: &mut Controls) {
        let Some(edit) = &mut self.bank else {
            return;
        };
        let mut confirm = std::mem::take(&mut edit.confirm_pending);
        let mut delete = false;
        let mut cancel = false;
        let mut blur = false;
        let popup = egui::Area::new(egui::Id::unique("subtitles.bank.editor"))
            .order(egui::Order::Foreground)
            .fixed_pos(parent.min_rect().min + egui::vec2(8.0, 56.0))
            .show(parent.ctx(), |ui| {
                egui::Frame::popup(ui.style())
                    .inner_margin(theme::space::SM)
                    .show(ui, |ui| {
                        ui.set_width(228.0);
                        ui.spacing_mut().interact_size.y = theme::size::FIELD;
                        ui.label(
                            egui::RichText::new(c.tr(if edit.save {
                                Text::SubBankSave
                            } else {
                                Text::SubBankManage
                            }))
                            .size(theme::type_size::BODY),
                        );
                        if !edit.save {
                            let previous = edit.preset;
                            let label = session
                                .project()
                                .data()
                                .caption_presets
                                .iter()
                                .find(|p| Some(p.id) == edit.preset)
                                .map_or("", |p| p.name.as_str());
                            let r = egui::ComboBox::from_id_salt(egui::Id::unique(
                                "subtitles.bank.choice",
                            ))
                            .selected_text(label)
                            .show_ui(ui, |ui| {
                                for preset in &session.project().data().caption_presets {
                                    let r = ui.selectable_value(
                                        &mut edit.preset,
                                        Some(preset.id),
                                        preset.name.as_str(),
                                    );
                                    c.remember(format!("subtitles.bank.entry.{}", preset.id.0), &r);
                                }
                            })
                            .response;
                            c.remember("subtitles.bank.choice", &r);
                            if edit.preset != previous {
                                edit.name = session
                                    .project()
                                    .data()
                                    .caption_presets
                                    .iter()
                                    .find(|p| Some(p.id) == edit.preset)
                                    .map_or(String::new(), |p| p.name.as_str().into());
                            }
                        }
                        let r = ui.add_sized(
                            egui::vec2(216.0, theme::size::FIELD),
                            egui::TextEdit::singleline(&mut edit.name)
                                .id(egui::Id::unique("subtitles.bank.name"))
                                .font(egui::FontId::proportional(theme::type_size::BODY))
                                .desired_width(216.0),
                        );
                        c.remember("subtitles.bank.name", &r);
                        if !edit.focused {
                            r.request_focus();
                            edit.focused = true;
                        }
                        let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                        cancel = ui.input(|i| i.key_pressed(egui::Key::Escape));
                        blur = r.lost_focus();
                        confirm |= !cancel && r.has_focus() && enter;
                        ui.horizontal(|ui| {
                            for (id, text, intent) in [
                                (
                                    "subtitles.bank.confirm",
                                    if edit.save {
                                        Text::Save
                                    } else {
                                        Text::SubBankRename
                                    },
                                    0,
                                ),
                                ("subtitles.bank.delete", Text::Delete, 1),
                                ("subtitles.bank.cancel", Text::Cancel, 2),
                            ] {
                                if edit.save && intent == 1 {
                                    continue;
                                }
                                let r = ui.button(
                                    egui::RichText::new(c.tr(text)).size(theme::type_size::BODY),
                                );
                                c.remember(id, &r);
                                if r.clicked() {
                                    match intent {
                                        0 => confirm = true,
                                        1 => delete = true,
                                        _ => cancel = true,
                                    }
                                }
                            }
                        });
                    });
            });
        if blur
            && parent.input(|i| {
                !i.pointer.primary_pressed()
                    || i.pointer
                        .interact_pos()
                        .is_none_or(|p| !popup.response.rect.contains(p))
            })
        {
            // Reconcile after the other panels process this blur before publishing it.
            if let Some(edit) = &mut self.bank {
                edit.confirm_pending = true;
            }
        }
        if cancel {
            self.bank = None;
        } else if confirm || delete {
            let result = self.confirm_bank(session, at, delete);
            self.ui_result(result);
        }
    }

    fn cues_ui(
        &mut self,
        ui: &mut egui::Ui,
        session: &mut Session,
        options: &Options,
        c: &mut Controls,
        dark: bool,
    ) {
        let Some((_, params)) = self.caption(session) else {
            return;
        };
        let cues = params.cues.cues.clone();
        let rate = session.project().data().canvas.fps;
        let profiles = LanguageRegistry::standard().ok();
        let mut selected = None;
        let mut save = false;
        let mut split_at = None;
        let mut merge = None;
        const PAGE: usize = 50;
        self.page = self.page.min(cues.len().saturating_sub(1) / PAGE);
        if cues.len() > PAGE {
            ui.horizontal_wrapped(|ui| {
                if c.button(
                    ui,
                    "subtitles.page.previous",
                    Text::SubPrevious,
                    Icon::SkipBack,
                    self.page > 0,
                ) {
                    let result = self.flush_drafts(session).map(|_| {
                        self.page = self.page.saturating_sub(1);
                    });
                    self.ui_result(result);
                }
                ui.label(i18n::format(
                    c.locale,
                    Text::SubPage,
                    &[
                        ("page", &(self.page + 1).to_string()),
                        ("pages", &cues.len().div_ceil(PAGE).to_string()),
                    ],
                ));
                if c.button(
                    ui,
                    "subtitles.page.next",
                    Text::SubNext,
                    Icon::SkipForward,
                    (self.page + 1) * PAGE < cues.len(),
                ) {
                    let result = self.flush_drafts(session).map(|_| {
                        self.page += 1;
                    });
                    self.ui_result(result);
                }
            });
        }
        for (index, cue) in cues.iter().enumerate().skip(self.page * PAGE).take(PAGE) {
            ui.separator();
            let time = format!(
                "{} – {}",
                time_text(cue.range.start(), rate, c.locale),
                time_text(cue.range.end(), rate, c.locale)
            );
            // The chip is as wide as its text; the keyboard hints are its tooltip.
            let r = ui
                .horizontal(|ui| {
                    ui.push_id(("subtitles.cue", cue.id.as_str()), |ui| {
                        ui.selectable_label(
                            self.selected.contains(&cue.id),
                            egui::RichText::new(time).color(if self.selected.contains(&cue.id) {
                                theme::on_accent(dark)
                            } else {
                                theme::text(dark)
                            }),
                        )
                        .on_hover_text(c.tr(Text::SubCueKeys))
                    })
                    .inner
                })
                .inner;
            c.remember(format!("subtitles.cue.{}", cue.id.as_str()), &r);
            if r.clicked() {
                selected = Some((index, ui.input(|i| i.modifiers)));
            }
            let mut languages: BTreeSet<_> = cue.texts.keys().cloned().collect();
            if let Ok(target) = LanguageTag::parse(&options.target) {
                languages.insert(target);
            }
            let mut languages: Vec<_> = languages.into_iter().collect();
            languages.sort_by_key(|l| l != &cue.source_lang);
            for language in languages {
                let text = cue.texts.get(&language);
                let original = text.map_or("", |t| t.text.as_str());
                if !self.selected.contains(&cue.id) && text.is_none() {
                    continue;
                }
                let label = ui
                    .horizontal_wrapped(|ui| {
                        let label = ui.label(language_name(c.locale, language.as_str()));
                        if text.is_some_and(|t| t.edited) {
                            ui.label(c.tr(Text::SubEdited));
                        }
                        if profiles
                            .as_ref()
                            .is_some_and(|p| Self::cps_warning(cue, &language, p).unwrap_or(false))
                        {
                            ui.colored_label(theme::warning(dark), c.tr(Text::SubCps));
                        }
                        label
                    })
                    .inner;
                if !self.selected.contains(&cue.id) {
                    if text.is_some() {
                        ui.label(original);
                    }
                    continue;
                }
                let draft = self
                    .drafts
                    .entry((cue.id.clone(), language.clone()))
                    .or_insert_with(|| original.into());
                // Enter splits at the caret; Shift+Enter is the only newline.
                let output = egui::TextEdit::multiline(draft)
                    .id_salt(("subtitles.text", cue.id.as_str(), language.as_str()))
                    .return_key(egui::KeyboardShortcut::new(
                        egui::Modifiers::SHIFT,
                        egui::Key::Enter,
                    ))
                    .desired_rows(2)
                    .char_limit(64 * 1024)
                    .desired_width(ui.available_width())
                    .show(ui);
                let r = output.response.response.labelled_by(label.id);
                if r.has_focus() {
                    self.text_focus = Some(r.id);
                    if let Some(cursor) = output.cursor_range
                        && cursor.is_empty()
                    {
                        let caret = cursor.primary.index.0;
                        let (enter, backspace, delete) = ui.input(|i| {
                            (
                                i.key_pressed(egui::Key::Enter)
                                    && !i.modifiers.shift
                                    && !i.modifiers.command
                                    && !i.modifiers.alt,
                                i.key_pressed(egui::Key::Backspace),
                                i.key_pressed(egui::Key::Delete),
                            )
                        });
                        if enter && let Some(byte) = byte_offset(draft, caret) {
                            split_at =
                                Some((cue.id.clone(), language.clone(), draft.clone(), byte));
                        } else if backspace && caret == 0 {
                            merge = Some(false);
                        } else if delete && caret == draft.chars().count() {
                            merge = Some(true);
                        }
                    }
                }
                c.remember(
                    format!("subtitles.text.{}.{}", cue.id.as_str(), language.as_str()),
                    &r,
                );
                if r.lost_focus() && draft != original {
                    save = true;
                }
            }
        }
        // Apply after rendering all fields so the snapshot cannot recreate old drafts.
        if let Some((cue, language, text, caret)) = split_at {
            let result = self
                .split_text(session, &cue, language, &text, caret)
                .map(|_| ());
            self.ui_result(result);
            ui.ctx().request_repaint();
        } else if let Some(down) = merge {
            let result = self.flush_drafts(session).and_then(|_| {
                if down {
                    self.merge_down(session)
                } else {
                    self.merge_up(session)
                }
            });
            self.ui_result(result);
            ui.ctx().request_repaint();
        } else if let Some((index, modifiers)) = selected {
            match self
                .flush_drafts(session)
                .and_then(|_| self.select_cue(session, index, modifiers.shift, modifiers.command))
            {
                Ok(at) => self.requested_seek = Some(at),
                Err(error) => self.ui_result(Err(error)),
            }
            ui.ctx().request_repaint();
        } else if save {
            let result = self.flush_drafts(session).map(|_| ());
            self.ui_result(result);
        }
    }
    fn consent_ui(&mut self, ctx: &egui::Context, c: &mut Controls) {
        let upload = self.consent_events().and_then(|events| {
            events
                .state
                .lock()
                .ok()
                .and_then(|s| s.upload.as_ref().map(|u| u.request.clone()))
        });
        let Some(upload) = upload else {
            return;
        };
        let modal = egui::Modal::new(egui::Id::unique("subtitles.consent"))
            .frame(crate::kit::dialog_frame(ctx))
            .show(ctx, |ui| {
                ui.set_width(380.0);
                crate::kit::dialog_title(ui, c.tr(Text::SubApprove));
                ui.label(i18n::format(
                    c.locale,
                    Text::ShortsUploadIdentity,
                    &[
                        ("provider", &upload.provider),
                        ("model", &upload.model),
                        ("languages", &upload.languages),
                    ],
                ));
                ui.label(c.tr(if upload.operation == CatalogOperation::Transcribe {
                    Text::ShortsUploadAudio
                } else {
                    Text::ShortsUploadTranscript
                }));
                if matches!(upload.cost, Cost::Unknown) {
                    ui.label(i18n::format(
                        c.locale,
                        Text::ShortsValue,
                        &[
                            ("label", c.tr(Text::ShortsSeconds)),
                            ("value", &i18n::decimal(c.locale, upload.audio_seconds, 2)),
                        ],
                    ));
                    ui.label(c.tr(Text::ShortsUnknownCost));
                } else {
                    ui.label(i18n::format(
                        c.locale,
                        Text::SubDisclosure,
                        &[
                            ("seconds", &i18n::decimal(c.locale, upload.audio_seconds, 2)),
                            ("cost", &cost_text(c.locale, &upload.cost)),
                        ],
                    ));
                }
                for (i, (_, url)) in upload.terms.iter().enumerate() {
                    if url.starts_with("https://") {
                        let r = ui.hyperlink_to(c.tr(Text::SubProviderPolicy), url);
                        c.remember(format!("subtitles.consent.policy.{i}"), &r);
                    }
                }
                let mut acknowledged = false;
                if let Some(events) = self.consent_events()
                    && let Ok(mut state) = events.state.lock()
                    && let Some(pending) = &mut state.upload
                {
                    if matches!(upload.cost, engine::providers::Cost::Unknown) {
                        let response = ui.checkbox(
                            &mut pending.price_acknowledged,
                            c.tr(Text::ModelRemoteUnknownPrice),
                        );
                        c.remember("subtitles.consent.unknown_price", &response);
                    }
                    acknowledged = pending.price_acknowledged;
                }
                ui.horizontal(|ui| {
                    if c.button(
                        ui,
                        "subtitles.consent.approve",
                        Text::ShortsConsent,
                        Icon::UploadSimple,
                        !matches!(upload.cost, engine::providers::Cost::Unknown) || acknowledged,
                    ) {
                        let result = self.approve_upload(true);
                        self.ui_result(result);
                    }
                    if c.button(
                        ui,
                        "subtitles.consent.deny",
                        Text::ShortsDeny,
                        Icon::X,
                        true,
                    ) {
                        let result = self.approve_upload(false);
                        self.ui_result(result);
                    }
                });
            });
        if modal.should_close() {
            let result = self.approve_upload(false);
            self.ui_result(result);
        }
    }
    /// The import button opens the SRT/VTT picker after the drafts are flushed.
    fn prepare_import(&mut self, ctx: &egui::Context, session: &mut Session, locale: Locale) {
        match self.flush_drafts(session) {
            Ok(_) => self.pick(ctx, locale),
            Err(_) => self.problem = Some(Text::SubEditFailed),
        }
    }
    fn pick(&mut self, ctx: &egui::Context, locale: Locale) {
        let dialog = rfd::AsyncFileDialog::new()
            .add_filter(i18n::text(locale, Text::SubImport), &["srt", "vtt"]);
        let future: std::pin::Pin<
            Box<dyn std::future::Future<Output = Option<rfd::FileHandle>> + Send>,
        > = Box::pin(dialog.pick_file());
        self.picker = Some(Picker {
            future,
            waker: std::task::Waker::from(Arc::new(Repaint(ctx.clone()))),
        });
    }
    fn poll_picker(&mut self, session: &Session, options: &Options, at: Tick) {
        let Some(picker) = self.picker.as_mut() else {
            return;
        };
        let mut context = std::task::Context::from_waker(&picker.waker);
        if let std::task::Poll::Ready(file) = picker.future.as_mut().poll(&mut context) {
            self.picker = None;
            if let Some(file) = file {
                let path = file.path().to_owned();
                let result = self.import_file(session, path, &options.source, at);
                self.ui_result(result);
            }
        }
    }
    /// Native picker injection uses the same engine export admission as the panel.
    pub fn export_subtitles(
        &mut self,
        session: &Session,
        path: PathBuf,
        format: captions::Format,
        options: &Options,
        overwrite: bool,
    ) -> Result<()> {
        self.export_file(
            session,
            path,
            CaptionOutput::File(format),
            options,
            overwrite,
        )
    }
    pub fn burn_subtitles(
        &mut self,
        session: &Session,
        path: PathBuf,
        options: &Options,
        overwrite: bool,
    ) -> Result<()> {
        self.export_file(session, path, CaptionOutput::Burned, options, overwrite)
    }
    fn export_file(
        &mut self,
        session: &Session,
        path: PathBuf,
        output: CaptionOutput,
        options: &Options,
        overwrite: bool,
    ) -> Result<()> {
        if self.busy() {
            return Err(Error::Invalid("subtitle job busy"));
        }
        let (item, p) = self
            .caption(session)
            .ok_or(Error::Invalid("caption item"))?;
        let source = p
            .cues
            .cues
            .first()
            .ok_or(Error::Invalid("no cues"))?
            .source_lang
            .clone();
        let languages = output_languages(options, &source)?;
        let track = session
            .project()
            .data()
            .scene()
            .timeline
            .iter()
            .find(|t| t.items.iter().any(|i| i.id == item.id))
            .ok_or(Error::Invalid("caption track"))?
            .id;
        let mut count = 0usize;
        for selected in session
            .project()
            .data()
            .scene()
            .timeline
            .iter()
            .filter(|t| {
                if matches!(output, CaptionOutput::Burned) {
                    !t.hidden
                } else {
                    t.id == track
                }
            })
        {
            for item in &selected.items {
                if let ItemParams::Caption(params) = &item.content {
                    if params.cues.cues.iter().any(|cue| {
                        languages.iter().any(|l| {
                            !cue.texts
                                .get(l)
                                .is_some_and(|text| !text.text.as_str().trim().is_empty())
                        })
                    }) {
                        return Err(Error::Invalid("translate selected export languages first"));
                    }
                    count = count.saturating_add(params.cues.cues.len());
                }
            }
        }
        let command = match output {
            CaptionOutput::Burned => Command::BurnSubtitles {
                range: time::TimeRange::new(Tick::ZERO, session.project().duration())?,
                languages,
                style: render::CaptionStyle::default(),
                portrait: false,
                path,
                overwrite,
            },
            CaptionOutput::File(format) => Command::ExportSubtitles {
                track,
                format,
                languages,
                path,
                overwrite,
            },
        };
        self.export = Some(session.submit_export_command(&mut self.runner, key()?, command)?);
        self.export_count = Some(count);
        self.problem = None;
        self.completed = None;
        Ok(())
    }
}
/// The first selected clip the Auto captions run can transcribe (video or audio).
fn source_clip(session: &Session) -> Option<ItemId> {
    session
        .selection()
        .iter()
        .find(|id| {
            session
                .project()
                .data()
                .scene()
                .timeline
                .iter()
                .flat_map(|t| &t.items)
                .any(|i| {
                    i.id == **id && matches!(i.content, ItemParams::Video(_) | ItemParams::Audio(_))
                })
        })
        .copied()
}
/// Character index from an egui text cursor to a UTF-8 byte offset.
fn byte_offset(text: &str, chars: usize) -> Option<usize> {
    text.char_indices()
        .map(|(byte, _)| byte)
        .chain(std::iter::once(text.len()))
        .nth(chars)
}
/// Player timecode (same formatter as the transport).
pub(super) fn time_text(tick: Tick, rate: time::FrameRate, locale: Locale) -> String {
    time::format_timecode(tick, rate)
        .unwrap_or_else(|_| i18n::decimal(locale, tick.0 as f64 / TICKS_PER_SECOND as f64, 3))
}
fn stage_text(stage: Option<Stage>) -> Text {
    match stage {
        Some(Stage::Transcribe) => Text::SubTranscribing,
        Some(Stage::Align) => Text::SubAligning,
        Some(Stage::Translate) => Text::SubTranslating,
        Some(Stage::Segment | Stage::Assemble) => Text::SubAssembling,
        _ => Text::Loading,
    }
}
fn cost_text(locale: Locale, cost: &Cost) -> String {
    match cost {
        Cost::Free => i18n::text(locale, Text::ShortsFree).into(),
        Cost::Unknown => i18n::text(locale, Text::ShortsUnknownCost).into(),
        Cost::UpperBound { currency, micros } => format!(
            "{} {}",
            currency,
            i18n::decimal(locale, *micros as f64 / 1_000_000.0, 6)
        ),
    }
}

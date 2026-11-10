use super::{Options, Pick, Picker, Repaint, Subtitles, key};
use crate::i18n;
use crate::{
    Error, Result, egui,
    i18n::{Locale, Text},
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

struct Controls {
    locale: Locale,
    widgets: Vec<(String, egui::Response)>,
}
impl Controls {
    fn tr(&self, text: Text) -> &'static str {
        i18n::text(self.locale, text)
    }
    fn remember(&mut self, id: impl Into<String>, r: &egui::Response) {
        let id = id.into();
        r.ctx.accesskit_node_builder(r.id, |n| n.set_author_id(&id));
        self.widgets.push((id, r.clone()));
    }
    fn button(
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
    /// Text-only button of a fixed width, for the aligned import/export row.
    fn row_button(
        &mut self,
        ui: &mut egui::Ui,
        id: &str,
        text: Text,
        enabled: bool,
        width: f32,
    ) -> bool {
        let label = self.tr(match text {
            Text::SubImport => Text::Import,
            Text::SubBurn => Text::SubMp4,
            _ => text,
        });
        let scope = ui.make_persistent_id(id);
        let r = ui
            .scope_builder(egui::UiBuilder::new().scope_id(scope), |ui| {
                ui.add_enabled(
                    enabled,
                    egui::Button::new(label)
                        .wrap_mode(egui::TextWrapMode::Truncate)
                        .min_size(egui::vec2(width, 26.0)),
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
                    &["vi", "en", "zh-Hans", "zh-Hant"]
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
            .find(|p| p.id == provider && p.models.contains(model));
        let label = current
            .map(|p| format!("{} / {}", provider_name(self.locale, p), model))
            .unwrap_or_else(|| self.tr(Text::SubChooseProvider).to_owned());
        let r = egui::ComboBox::from_id_salt(id)
            .width(ui.available_width().max(30.0))
            .wrap_mode(egui::TextWrapMode::Wrap)
            .selected_text(label)
            .show_ui(ui, |ui| {
                for option in options {
                    for value in &option.models {
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
impl Subtitles {
    /// The assets-panel mount is one call. Engine edits remain in this module.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn ui(
        &mut self,
        ui: &mut egui::Ui,
        session: &mut Session,
        options: &mut Options,
        selection: &BTreeSet<ItemId>,
        at: Tick,
        locale: Locale,
        dark: bool,
    ) -> Vec<(String, egui::Response)> {
        self.refresh(session);
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
        let previous_options = options.clone();
        self.text_focus = None;
        let mut controls = Controls {
            locale,
            widgets: Vec::new(),
        };
        self.poll_picker(session, options, at);
        let busy = self.busy();
        egui::ScrollArea::vertical()
            .id_salt("subtitles.panel.scroll")
            .show(ui, |ui| {
                ui.set_max_width(ui.available_width());
                ui.spacing_mut().item_spacing.x = 4.0;
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                ui.heading(controls.tr(Text::Caption));
                // One aligned row: Import then the four exports, equal widths.
                let gap = ui.spacing().item_spacing.x;
                let width = ((ui.available_width() - gap * 4.0) / 5.0).max(28.0);
                ui.horizontal(|ui| {
                    for (id, label, pick) in [
                        ("subtitles.import", Text::SubImport, Pick::Import),
                        ("subtitles.export.srt", Text::SubSrt, Pick::Srt),
                        ("subtitles.export.vtt", Text::SubVtt, Pick::Vtt),
                        ("subtitles.export.txt", Text::SubTxt, Pick::Txt),
                        ("subtitles.export.burn", Text::SubBurn, Pick::Burn),
                    ] {
                        let enabled = !busy
                            && self.picker.is_none()
                            && (matches!(pick, Pick::Import) || self.item.is_some());
                        if controls.row_button(ui, id, label, enabled, width) {
                            self.prepare_pick(ui.ctx(), session, pick, locale);
                        }
                    }
                });
                let r = egui::CollapsingHeader::new(controls.tr(Text::SubGenerate))
                    .id_salt("subtitles.generate.settings")
                    .default_open(self.item.is_none())
                    .show(ui, |ui| {
                        ui.add_enabled_ui(!busy, |ui| {
                            ui.label(controls.tr(Text::ShortsSourceLanguage));
                            controls.language(ui, "subtitles.source", &mut options.source, false);
                            ui.label(controls.tr(Text::ShortsTranscriber));
                            controls.provider(
                                ui,
                                "subtitles.transcriber",
                                &mut options.transcription,
                                &mut options.transcription_model,
                                CatalogOperation::Transcribe,
                                &options.source,
                            );
                            ui.label(controls.tr(Text::SubSelectMedia));
                            let source = selection
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
                                            i.id == **id
                                                && matches!(
                                                    i.content,
                                                    ItemParams::Video(_) | ItemParams::Audio(_)
                                                )
                                        })
                                })
                                .copied();
                            if controls.button(
                                ui,
                                "subtitles.generate",
                                Text::SubGenerate,
                                Icon::Sparkle,
                                source.is_some() && options.transcription_supported(),
                            ) && let Some(source) = source
                            {
                                let result = self
                                    .flush_drafts(session)
                                    .and_then(|_| self.generate(session, source, options));
                                self.ui_result(result);
                            }
                        });
                    });
                controls.remember("subtitles.generate.settings", &r.header_response);
                let r = egui::CollapsingHeader::new(controls.tr(Text::SubTranslate))
                    .id_salt("subtitles.translation.settings")
                    .show(ui, |ui| {
                        ui.add_enabled_ui(!busy, |ui| {
                            ui.label(controls.tr(Text::SubTarget));
                            controls.language(ui, "subtitles.target", &mut options.target, true);
                            ui.label(controls.tr(Text::SubTranslator));
                            controls.provider(
                                ui,
                                "subtitles.translator",
                                &mut options.translation,
                                &mut options.translation_model,
                                CatalogOperation::Translate,
                                &options.source,
                            );
                            let r = ui
                                .checkbox(&mut options.bilingual, controls.tr(Text::SubBilingual));
                            controls.remember("subtitles.bilingual", &r);
                            let r =
                                ui.checkbox(&mut self.force, controls.tr(Text::SubReplaceEdited));
                            controls.remember("subtitles.force", &r);
                            if controls.button(
                                ui,
                                "subtitles.translate",
                                Text::SubTranslate,
                                Icon::Translate,
                                !self.selected.is_empty()
                                    && self.item.is_some()
                                    && options.validate().is_ok(),
                            ) {
                                let result = self
                                    .flush_drafts(session)
                                    .and_then(|_| self.translate(session, options));
                                self.ui_result(result);
                            }
                            self.glossary_ui(ui, &mut controls);
                        });
                    });
                controls.remember("subtitles.translation.settings", &r.header_response);
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
                                format!("subtitles.recovery.{}.{}", id.as_str(), language.as_str()),
                                &r,
                            );
                        }
                    }
                    if let Some(id) = self.missing_model.clone()
                        && controls.button(ui, "subtitles.models", Text::Models, Icon::File, !busy)
                    {
                        self.requested_model = Some(id);
                    }
                    if controls.button(ui, "subtitles.dismiss", Text::ShortsDismiss, Icon::X, true)
                    {
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
                        crate::progress::job_progress(ui, &job, locale);
                    } else {
                        ui.label(controls.tr(if snapshot.cancelling {
                            Text::ShortsCancelling
                        } else {
                            stage_text(snapshot.stage)
                        }));
                        if let Some(progress) = snapshot.progress {
                            ui.add(egui::ProgressBar::new(progress).show_percentage());
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
                    ui.label(controls.tr(Text::SubEmpty));
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
                ui.add_enabled_ui(!busy, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        if controls.icon_button(
                            ui,
                            "subtitles.select_all",
                            Text::SubSelectAll,
                            Icon::Plus,
                            true,
                        ) && let Some((_, p)) = self.caption(session)
                        {
                            self.selected = p.cues.cues.iter().map(|c| c.id.clone()).collect();
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
                            let result =
                                self.flush_drafts(session).and_then(|_| self.merge(session));
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
                });
            });
        controls.widgets
    }
    pub(crate) fn dialogs(
        &mut self,
        ctx: &egui::Context,
        session: &Session,
        options: &Options,
        locale: Locale,
    ) -> Vec<(String, egui::Response)> {
        let mut controls = Controls {
            locale,
            widgets: Vec::new(),
        };
        self.consent_ui(ctx, &mut controls);
        self.overwrite_ui(ctx, session, options, &mut controls);
        controls.widgets
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
        let r = egui::CollapsingHeader::new(c.tr(Text::SubGlossary))
            .id_salt("subtitles.glossary")
            .show(ui, |ui| {
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
        c.remember("subtitles.glossary", &r.header_response);
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
        let upload = self.active.as_ref().and_then(|a| {
            a.events
                .state
                .lock()
                .ok()
                .and_then(|s| s.upload.as_ref().map(|u| u.request.clone()))
        });
        let Some(upload) = upload else {
            return;
        };
        let modal = egui::Modal::new(egui::Id::unique("subtitles.consent")).show(ctx, |ui| {
            ui.set_width(380.0);
            ui.heading(c.tr(Text::SubApprove));
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
            ui.label(i18n::format(
                c.locale,
                Text::SubDisclosure,
                &[
                    ("seconds", &i18n::decimal(c.locale, upload.audio_seconds, 2)),
                    ("cost", &cost_text(c.locale, &upload.cost)),
                ],
            ));
            for (i, (_, url)) in upload.terms.iter().enumerate() {
                if url.starts_with("https://") {
                    let r = ui.hyperlink_to(c.tr(Text::SubProviderPolicy), url);
                    c.remember(format!("subtitles.consent.policy.{i}"), &r);
                }
            }
            ui.horizontal(|ui| {
                if c.button(
                    ui,
                    "subtitles.consent.approve",
                    Text::ShortsConsent,
                    Icon::UploadSimple,
                    true,
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
    fn prepare_pick(
        &mut self,
        ctx: &egui::Context,
        session: &mut Session,
        kind: Pick,
        locale: Locale,
    ) {
        match self.flush_drafts(session) {
            Ok(_) => self.pick(ctx, kind, locale),
            Err(_) => self.problem = Some(Text::SubEditFailed),
        }
    }
    fn pick(&mut self, ctx: &egui::Context, kind: Pick, locale: Locale) {
        let picker = rfd::AsyncFileDialog::new();
        let future: std::pin::Pin<
            Box<dyn std::future::Future<Output = Option<rfd::FileHandle>> + Send>,
        > = match kind {
            Pick::Import => Box::pin(
                picker
                    .add_filter(i18n::text(locale, Text::SubImport), &["srt", "vtt"])
                    .pick_file(),
            ),
            Pick::Srt => Box::pin(
                picker
                    .set_file_name(format!("{}.srt", i18n::text(locale, Text::SubFileBase)))
                    .save_file(),
            ),
            Pick::Vtt => Box::pin(
                picker
                    .set_file_name(format!("{}.vtt", i18n::text(locale, Text::SubFileBase)))
                    .save_file(),
            ),
            Pick::Txt => Box::pin(
                picker
                    .set_file_name(format!("{}.txt", i18n::text(locale, Text::SubFileBase)))
                    .save_file(),
            ),
            Pick::Burn => Box::pin(
                picker
                    .set_file_name(format!("{}.mp4", i18n::text(locale, Text::SubFileBase)))
                    .save_file(),
            ),
        };
        self.picker = Some(Picker {
            kind,
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
            let kind = picker.kind;
            self.picker = None;
            if let Some(file) = file {
                let path = file.path().to_owned();
                let result = match kind {
                    Pick::Import => self.import_file(session, path, &options.source, at),
                    _ => {
                        if path.exists() {
                            self.overwrite = Some((path, kind));
                            Ok(())
                        } else {
                            self.export_file(session, path, kind, options, false)
                        }
                    }
                };
                self.ui_result(result);
            }
        }
    }
    fn overwrite_ui(
        &mut self,
        ctx: &egui::Context,
        session: &Session,
        options: &Options,
        c: &mut Controls,
    ) {
        if self.overwrite.is_none() {
            return;
        }
        let mut confirmed = false;
        let mut cancelled = false;
        let modal = egui::Modal::new(egui::Id::unique("subtitles.overwrite")).show(ctx, |ui| {
            ui.heading(c.tr(Text::SubOverwrite));
            if c.button(
                ui,
                "subtitles.overwrite.confirm",
                Text::SubOverwrite,
                Icon::Export,
                true,
            ) {
                confirmed = true;
            }
            if c.button(
                ui,
                "subtitles.overwrite.cancel",
                Text::Cancel,
                Icon::X,
                true,
            ) {
                cancelled = true;
            }
        });
        if confirmed && let Some((path, kind)) = self.overwrite.take() {
            let result = self.export_file(session, path, kind, options, true);
            self.ui_result(result);
        } else if cancelled || modal.should_close() {
            self.overwrite = None;
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
            match format {
                captions::Format::Srt => Pick::Srt,
                captions::Format::WebVtt => Pick::Vtt,
                captions::Format::Txt => Pick::Txt,
            },
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
        self.export_file(session, path, Pick::Burn, options, overwrite)
    }
    fn export_file(
        &mut self,
        session: &Session,
        path: PathBuf,
        kind: Pick,
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
        let target = LanguageTag::parse(&options.target)?;
        let languages = if options.bilingual && target != source {
            vec![source, target]
        } else {
            vec![target]
        };
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
                if matches!(kind, Pick::Burn) {
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
        let command = match kind {
            Pick::Burn => Command::BurnSubtitles {
                range: time::TimeRange::new(Tick::ZERO, session.project().duration())?,
                languages,
                style: render::CaptionStyle::default(),
                portrait: false,
                path,
                overwrite,
            },
            Pick::Srt | Pick::Vtt | Pick::Txt => Command::ExportSubtitles {
                track,
                format: match kind {
                    Pick::Srt => captions::Format::Srt,
                    Pick::Vtt => captions::Format::WebVtt,
                    Pick::Txt => captions::Format::Txt,
                    _ => return Err(Error::Invalid("export format")),
                },
                languages,
                path,
                overwrite,
            },
            Pick::Import => return Err(Error::Invalid("export format")),
        };
        self.export = Some(session.submit_export_command(&mut self.runner, key()?, command)?);
        self.export_count = Some(count);
        self.problem = None;
        self.completed = None;
        Ok(())
    }
}
/// Character index from an egui text cursor to a UTF-8 byte offset.
fn byte_offset(text: &str, chars: usize) -> Option<usize> {
    text.char_indices()
        .map(|(byte, _)| byte)
        .chain(std::iter::once(text.len()))
        .nth(chars)
}
/// Player timecode (same formatter and drop-frame rule as the transport).
fn time_text(tick: Tick, rate: time::FrameRate, locale: Locale) -> String {
    let drop = matches!(
        rate,
        time::FrameRate::FPS_29_97 | time::FrameRate::FPS_59_94
    );
    time::format_timecode(tick, rate, drop)
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

//! Review window widgets; state and edit validation stay with Shorts.
use super::{
    Decision, Pick, Picker, Repaint, Shorts, failure_text, parse_seconds, seconds, terminal,
};
use crate::{
    Editor, Error,
    catalog::{self, Locale, Text},
    crop, egui,
    kit::{self as icons, Icon},
};
use engine::{ClipRole, ClipSource};
use engine::{
    jobs::JobState,
    subtitles::{Cost, Stage},
};
use std::{sync::Arc, time::Duration};
use time::Tick;
impl Shorts {
    fn tr(&self, key: Text) -> &'static str {
        catalog::text(self.preferences.locale, key)
    }
    fn button(&self, ui: &mut egui::Ui, id: &str, key: Text, icon: Icon, enabled: bool) -> bool {
        let primary = enabled && matches!(key, Text::ShortsGenerate | Text::ShortsExport);
        let color = if primary {
            ui.visuals().selection.stroke.color
        } else {
            ui.visuals().text_color()
        };
        let mut button = egui::Button::image_and_text(
            icons::tinted(icon, 16.0, color),
            egui::RichText::new(self.tr(key)).color(color),
        );
        if primary {
            button = button.fill(ui.visuals().selection.bg_fill);
        }
        let response = ui.push_id(id, |ui| ui.add_enabled(enabled, button)).inner;
        author(&response, id);
        response.clicked()
    }
    fn pick(&mut self, ctx: &egui::Context, kind: Pick) {
        let future: std::pin::Pin<Box<dyn std::future::Future<Output = Option<rfd::FileHandle>>>> =
            match kind {
                Pick::Video => Box::pin(
                    rfd::AsyncFileDialog::new()
                        .add_filter(self.tr(Text::Video), &["mp4", "mov", "mkv", "webm", "m4v"])
                        .pick_file(),
                ),
                Pick::Export => Box::pin(rfd::AsyncFileDialog::new().pick_folder()),
            };
        self.picker = Some(Picker {
            future,
            kind,
            waker: std::task::Waker::from(Arc::new(Repaint(ctx.clone()))),
        });
    }
    pub(super) fn poll_picker(&mut self) {
        let Some(mut picker) = self.picker.take() else {
            return;
        };
        match picker
            .future
            .as_mut()
            .poll(&mut std::task::Context::from_waker(&picker.waker))
        {
            std::task::Poll::Pending => self.picker = Some(picker),
            std::task::Poll::Ready(None) => {}
            std::task::Poll::Ready(Some(file)) => {
                let result = match picker.kind {
                    Pick::Video => self.pick_video(file.path().to_owned()),
                    Pick::Export => self.export_to(file.path().to_owned()),
                };
                if let Err(e) = result {
                    self.fail(e, Text::ShortsError);
                }
            }
        }
    }
    /// Drawn in its own native viewport by Workspace, directly by headless audits.
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        if !self.configured {
            Editor::fonts(ui.ctx());
            icons::install(ui.ctx());
            self.configured = true;
        }
        Editor::theme(ui.ctx(), self.preferences.dark);
        ui.style_mut().spacing.interact_size.y = 26.0;
        ui.painter()
            .rect_filled(ui.max_rect(), 0.0, ui.visuals().panel_fill);
        self.tick();
        let drops = ui.input(|i| i.raw.dropped_files.clone());
        if !drops.is_empty() {
            let result = if drops.len() == 1 {
                drops
                    .first()
                    .map(|d| d.path().to_owned())
                    .ok_or(Error::Invalid("native video path"))
                    .and_then(|path| self.pick_video(path))
            } else {
                Err(Error::Invalid("choose one video"))
            };
            if let Err(e) = result {
                self.fail(e, Text::ShortsError);
            }
        }
        ui.horizontal(|ui| {
            ui.heading(egui::RichText::new(self.tr(Text::Shorts)).size(28.0));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if self.button(ui, "shorts.locale", Text::Language, Icon::Translate, true) {
                    self.preferences.locale = if self.preferences.locale == Locale::En {
                        Locale::Vi
                    } else {
                        Locale::En
                    };
                }
                if self.button(
                    ui,
                    "shorts.redo",
                    Text::Redo,
                    Icon::ArrowClockwise,
                    !self.redo.is_empty(),
                ) {
                    self.redo();
                }
                if self.button(
                    ui,
                    "shorts.undo",
                    Text::Undo,
                    Icon::ArrowCounterClockwise,
                    !self.undo.is_empty(),
                ) {
                    self.undo();
                }
            });
        });
        ui.separator();
        if self.cancelled {
            ui.label(self.tr(Text::ShortsCancelled));
        }
        if self.error.is_some() {
            ui.colored_label(ui.visuals().error_fg_color, self.tr(self.error_text));
            if self.button(
                ui,
                "shorts.error.dismiss",
                Text::ShortsDismiss,
                Icon::X,
                true,
            ) {
                self.error = None;
            }
        }
        if self.generating.is_some() {
            self.progress_ui(ui);
        } else if self.source.is_some() {
            self.review_ui(ui);
        } else {
            self.setup_ui(ui);
        }
        self.export_ui(ui);
        ui.ctx().request_repaint_after(Duration::from_millis(16));
    }
    fn setup_ui(&mut self, ui: &mut egui::Ui) {
        let available = ui.available_rect_before_wrap();
        let width = available.width().min(760.0);
        let rect = egui::Rect::from_min_size(
            egui::pos2(available.center().x - width / 2.0, available.top() + 24.0),
            egui::vec2(width, available.height() - 24.0),
        );
        ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
            egui::ScrollArea::vertical()
                .id_salt("shorts.setup.scroll")
                .show(ui, |ui| {
                    ui.label(self.tr(Text::ShortsIntro));
                    ui.add_space(20.0);
                    if self.request.path.as_os_str().is_empty() {
                        let available_width = ui.available_width();
                        let hovered = ui.input(|i| !i.raw.hovered_files.is_empty());
                        let stroke = if hovered {
                            egui::Stroke::new(1.0, ui.visuals().selection.bg_fill)
                        } else {
                            ui.visuals().window_stroke
                        };
                        egui::Frame::group(ui.style())
                            .inner_margin(24.0)
                            .stroke(stroke)
                            .show(ui, |ui| {
                                ui.set_min_size(egui::vec2(
                                    (available_width - 48.0).max(0.0),
                                    112.0,
                                ));
                                ui.vertical_centered(|ui| {
                                    let (_, rect) = ui.allocate_space(egui::vec2(24.0, 24.0));
                                    icons::tinted(Icon::FilmStrip, 24.0, ui.visuals().text_color())
                                        .paint_at(ui, rect);
                                    ui.label(self.tr(Text::ShortsDrop));
                                    self.choose_video_button(ui);
                                });
                            });
                    } else {
                        ui.label(
                            self.request
                                .path
                                .file_name()
                                .unwrap_or_default()
                                .to_string_lossy(),
                        );
                        self.choose_video_button(ui);
                    }
                    ui.add_space(20.0);
                    ui.label(self.tr(Text::ShortsSourceLanguage));
                    let combo = egui::ComboBox::from_id_salt(egui::Id::unique("shorts.language"))
                        .selected_text(&self.request.language)
                        .show_ui(ui, |ui| {
                            for language in ["vi", "en", "zh"] {
                                let label =
                                    choice_label(ui, self.request.language == language, language);
                                let response = ui.selectable_value(
                                    &mut self.request.language,
                                    language.to_owned(),
                                    label,
                                );
                                author(&response, &format!("shorts.language.{language}"));
                            }
                        });
                    named(
                        &combo.response,
                        "shorts.language",
                        self.tr(Text::ShortsSourceLanguage),
                    );
                    self.provider_ui(ui);
                    ui.add_space(16.0);
                    egui::Grid::new("shorts.durations")
                        .spacing([24.0, 22.0])
                        .show(ui, |ui| {
                            ui.label(self.tr(Text::ShortsCount));
                            let r =
                                ui.add(egui::DragValue::new(&mut self.request.count).range(1..=32));
                            named(&r, "shorts.count", self.tr(Text::ShortsCount));
                            ui.end_row();
                            ui.label(self.tr(Text::ShortsMin));
                            let r = ui.add(
                                egui::DragValue::new(&mut self.request.min_seconds).range(1..=180),
                            );
                            named(&r, "shorts.min", self.tr(Text::ShortsMin));
                            ui.end_row();
                            ui.label(self.tr(Text::ShortsMax));
                            let r = ui.add(
                                egui::DragValue::new(&mut self.request.max_seconds).range(1..=180),
                            );
                            named(&r, "shorts.max", self.tr(Text::ShortsMax));
                            ui.end_row();
                            ui.label(self.tr(Text::ShortsMaxSegments));
                            let r = ui.add(
                                egui::DragValue::new(&mut self.request.max_segments).range(1..=32),
                            );
                            named(&r, "shorts.max_segments", self.tr(Text::ShortsMaxSegments));
                            ui.end_row();
                        });
                    ui.add_space(20.0);
                    if self.button(
                        ui,
                        "shorts.generate",
                        Text::ShortsGenerate,
                        Icon::Sparkle,
                        !self.busy() && !self.request.path.as_os_str().is_empty(),
                    ) && let Err(e) = self.generate()
                    {
                        self.fail(e, Text::ShortsError);
                    }
                });
        });
    }
    fn choose_video_button(&mut self, ui: &mut egui::Ui) {
        let label = if self.request.path.as_os_str().is_empty() {
            Text::ShortsPick
        } else {
            Text::ShortsReplace
        };
        if self.button(ui, "shorts.pick", label, Icon::FolderOpen, !self.busy()) {
            self.pick(ui.ctx(), Pick::Video);
        }
    }
    fn provider_ui(&mut self, ui: &mut egui::Ui) {
        use engine::subtitles::{CatalogOperation, provider_options_for};
        let language = captions::LanguageTag::parse(&self.request.language).ok();
        ui.label(self.tr(Text::ShortsTranscriber));
        let transcription = provider_options_for(CatalogOperation::Transcribe, language.as_ref());
        let text = provider_label(
            self.preferences.locale,
            &self.request.transcription_provider,
            &self.request.transcription_model,
        );
        let combo = egui::ComboBox::from_id_salt(egui::Id::unique("shorts.transcriber"))
            .width(400.0)
            .selected_text(text)
            .show_ui(ui, |ui| {
                for option in &transcription {
                    for model in &option.models {
                        let label = provider_label(self.preferences.locale, option.id, model);
                        let selected = option.id == self.request.transcription_provider
                            && model == &self.request.transcription_model;
                        let response =
                            ui.selectable_label(selected, choice_label(ui, selected, label));
                        author(
                            &response,
                            &format!("shorts.transcriber.{}.{}", option.id, model),
                        );
                        if response.clicked() {
                            self.request.transcription_provider = option.id.into();
                            self.request.transcription_model = model.clone();
                        }
                    }
                }
            });
        named(
            &combo.response,
            "shorts.transcriber",
            self.tr(Text::ShortsTranscriber),
        );
        ui.label(self.tr(Text::ShortsSuggester));
        let local = self.tr(Text::ShortsLocal);
        let label = if self.request.suggestion_provider == "heuristic" {
            local.to_owned()
        } else {
            provider_label(
                self.preferences.locale,
                &self.request.suggestion_provider,
                &self.request.suggestion_model,
            )
        };
        let options = engine::clip_suggest::suggestion_options();
        let combo = egui::ComboBox::from_id_salt(egui::Id::unique("shorts.suggester"))
            .width(400.0)
            .selected_text(label)
            .show_ui(ui, |ui| {
                for option in &options {
                    for model in &option.models {
                        let label = if option.local {
                            local.to_owned()
                        } else {
                            provider_label(self.preferences.locale, option.id, model)
                        };
                        let selected = option.id == self.request.suggestion_provider
                            && *model == self.request.suggestion_model;
                        let r = ui.selectable_label(selected, choice_label(ui, selected, label));
                        author(&r, &format!("shorts.suggester.{}.{}", option.id, model));
                        if r.clicked() {
                            self.request.suggestion_provider = option.id.into();
                            self.request.suggestion_model = (*model).into();
                        }
                    }
                }
            });
        named(
            &combo.response,
            "shorts.suggester",
            self.tr(Text::ShortsSuggester),
        );
        if let Some(option) = options
            .iter()
            .find(|o| o.id == self.request.suggestion_provider)
            && let Some(url) = option.policy_url
        {
            ui.label(self.tr(Text::ShortsDisclosure));
            let r = ui.hyperlink_to(option.id, url);
            author(&r, "shorts.suggestion.policy");
        }
        if let Some(option) = transcription
            .iter()
            .find(|o| o.id == self.request.transcription_provider)
            && !option.local
        {
            ui.label(self.tr(Text::ShortsDisclosure));
            let url = if option.id == "openai" {
                engine::clip_suggest::OPENAI_POLICY_URL
            } else {
                "https://elevenlabs.io/terms-of-use"
            };
            let r = ui.hyperlink(url);
            author(&r, "shorts.transcription.policy");
        }
    }
    fn progress_ui(&mut self, ui: &mut egui::Ui) {
        ui.add_space(24.0);
        let cancelling = self.cancelling();
        ui.heading(self.tr(if cancelling {
            Text::ShortsCancelling
        } else {
            Text::ShortsCreating
        }));
        let mut response = None;
        if let Some(job) = &self.generating
            && let Ok(state) = job.events.state.lock()
        {
            if let Some(stage) = state.stage {
                ui.label(self.tr(stage_text(stage)));
            }
            ui.add(egui::ProgressBar::new(state.fraction.unwrap_or(0.0)).show_percentage());
            if let Some(upload) = &state.upload
                && !cancelling
            {
                ui.separator();
                ui.label(self.tr(Text::ShortsUploadScope));
                ui.label(catalog::format(
                    self.preferences.locale,
                    Text::ShortsUploadIdentity,
                    &[
                        ("provider", &upload.request.provider),
                        ("model", &upload.request.model),
                        ("languages", &upload.request.languages),
                    ],
                ));
                ui.label(catalog::format(
                    self.preferences.locale,
                    Text::ShortsValue,
                    &[
                        ("label", self.tr(Text::ShortsSeconds)),
                        (
                            "value",
                            &catalog::decimal(
                                self.preferences.locale,
                                upload.request.audio_seconds,
                                2,
                            ),
                        ),
                    ],
                ));
                ui.label(catalog::format(
                    self.preferences.locale,
                    Text::ShortsValue,
                    &[
                        ("label", self.tr(Text::ShortsBytes)),
                        (
                            "value",
                            &catalog::number(self.preferences.locale, upload.request.upload_bytes),
                        ),
                    ],
                ));
                ui.label(catalog::format(
                    self.preferences.locale,
                    Text::ShortsValue,
                    &[
                        ("label", self.tr(Text::ShortsCost)),
                        (
                            "value",
                            &cost_text(self.preferences.locale, &upload.request.cost),
                        ),
                    ],
                ));
                ui.label(self.tr(
                    if upload.request.operation == engine::subtitles::CatalogOperation::Transcribe {
                        Text::ShortsUploadAudio
                    } else {
                        Text::ShortsUploadTranscript
                    },
                ));
                for (index, (kind, url)) in upload.request.terms.iter().enumerate() {
                    if !url.starts_with("https://") {
                        continue;
                    }
                    let label = match *kind {
                        "policy" | "retention_training_policy" => Text::ShortsPolicy,
                        "endpoint" => Text::ShortsEndpoint,
                        "pricing" => Text::ShortsCost,
                        _ => Text::Details,
                    };
                    let r = ui.hyperlink_to(self.tr(label), url);
                    author(&r, &format!("shorts.consent.policy.{index}"));
                }
                ui.horizontal(|ui| {
                    if self.button(
                        ui,
                        "shorts.consent.approve",
                        Text::ShortsConsent,
                        Icon::UploadSimple,
                        true,
                    ) {
                        response = Some(true);
                    }
                    if self.button(ui, "shorts.consent.deny", Text::ShortsDeny, Icon::X, true) {
                        response = Some(false);
                    }
                });
            }
        }
        if let Some(approve) = response
            && let Err(e) = self.approve_upload(approve)
        {
            self.fail(e, Text::ShortsError);
        }
        if self.button(ui, "shorts.cancel", Text::Cancel, Icon::X, !cancelling) {
            self.cancel();
        }
    }
    fn review_ui(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading(self.tr(Text::ShortsReview));
            if self.button(
                ui,
                "shorts.setup",
                Text::ShortsBack,
                Icon::ArrowLeft,
                !self.busy(),
            ) && let Err(e) = self.pick_video(self.request.path.clone())
            {
                self.fail(e, Text::ShortsError);
            }
            if self.button(
                ui,
                "shorts.replace",
                Text::ShortsReplace,
                Icon::FolderOpen,
                !self.busy(),
            ) {
                self.pick(ui.ctx(), Pick::Video);
            }
            let label = self.tr(Text::ShortsCaptions);
            let r = ui.checkbox(&mut self.captions, label);
            author(&r, "shorts.captions");
            if r.changed() {
                self.preview_dirty = true;
            }
            if self.button(
                ui,
                "shorts.edit",
                Text::ShortsEdit,
                Icon::PencilSimple,
                !self.reviews.is_empty() && !self.busy() && self.can_open_editor,
            ) {
                self.open_editor = true;
            }
            let accepted = self
                .reviews
                .iter()
                .any(|r| r.decision == Decision::Accepted);
            if self.button(
                ui,
                "shorts.export",
                Text::ShortsExport,
                Icon::Export,
                accepted && !self.busy(),
            ) {
                self.pick(ui.ctx(), Pick::Export);
            }
        });
        if !self.can_open_editor {
            ui.label(self.tr(Text::ShortsEditorBusy));
        }
        if self.reviews.is_empty() {
            ui.label(self.tr(Text::ShortsNoResults));
            return;
        }
        ui.separator();
        let width = ui.available_width();
        let height =
            (ui.available_height() - if self.batch.is_some() { 160.0 } else { 0.0 }).max(320.0);
        let list_width = (width * 0.23).clamp(220.0, 290.0);
        let tools_width = (width * 0.28).clamp(300.0, 380.0);
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(list_width, height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("shorts.candidates")
                        .max_height(height)
                        .show(ui, |ui| {
                            let mut selected = None;
                            if let Some(source) = &self.source {
                                for (index, (candidate, review)) in
                                    source.candidates.iter().zip(&self.reviews).enumerate()
                                {
                                    let r = ui.selectable_label(
                                        index == self.selected,
                                        choice_label(
                                            ui,
                                            index == self.selected,
                                            candidate.proposal.title.as_str(),
                                        )
                                        .size(16.0)
                                        .strong(),
                                    );
                                    author(&r, &format!("shorts.candidate.{index}"));
                                    if r.clicked() {
                                        selected = Some(index);
                                    }
                                    let duration = review
                                        .segments
                                        .iter()
                                        .fold(Tick::ZERO, |sum, s| sum + s.range.duration());
                                    ui.label(catalog::format(
                                        self.preferences.locale,
                                        Text::ShortsMetadata,
                                        &[
                                            ("duration_label", self.tr(Text::ShortsDuration)),
                                            (
                                                "duration",
                                                &seconds(self.preferences.locale, duration),
                                            ),
                                            ("score_label", self.tr(Text::ShortsScore)),
                                            (
                                                "score",
                                                &catalog::decimal(
                                                    self.preferences.locale,
                                                    candidate.proposal.score * 100.0,
                                                    0,
                                                ),
                                            ),
                                        ],
                                    ));
                                    ui.label(self.tr(match candidate.source {
                                        ClipSource::Heuristic => Text::ShortsHeuristic,
                                        ClipSource::Llm => Text::ShortsAi,
                                    }));
                                    ui.label(self.tr(decision_text(review.decision)));
                                    ui.separator();
                                }
                            }
                            if let Some(index) = selected
                                && let Err(e) = self.select(index)
                            {
                                self.fail(e, Text::ShortsError);
                            }
                        });
                },
            );
            ui.separator();
            ui.allocate_ui_with_layout(
                egui::vec2((width - list_width - tools_width - 36.0).max(160.0), height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    if let Some(preview) = &mut self.preview {
                        preview.preferences = self.preferences.clone();
                        preview.preview_ui(ui);
                    } else {
                        ui.spinner();
                    }
                },
            );
            ui.separator();
            ui.allocate_ui_with_layout(
                egui::vec2(tools_width, height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("shorts.tools")
                        .max_height(height)
                        .show(ui, |ui| self.tools_ui(ui));
                },
            );
        });
        if !ui.ctx().text_edit_focused() && self.crop_entry.is_none() {
            let redo = ui.input_mut(|i| {
                i.consume_key(
                    egui::Modifiers::COMMAND.plus(egui::Modifiers::SHIFT),
                    egui::Key::Z,
                )
            });
            let undo = ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Z));
            if redo {
                self.redo();
            } else if undo {
                self.undo();
            }
            for (key, id) in [
                (egui::Key::Space, "transport.play"),
                (egui::Key::K, "transport.pause"),
                (egui::Key::J, "transport.back"),
                (egui::Key::L, "transport.forward"),
                (egui::Key::Home, "transport.start"),
                (egui::Key::End, "transport.end"),
            ] {
                if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, key))
                    && let Some(preview) = &mut self.preview
                    && let Err(e) = preview.activate(id)
                {
                    self.fail(e, Text::ShortsError);
                }
            }
            if !self.crop_focused {
                for (key, id) in [
                    (egui::Key::ArrowLeft, "transport.frame_back"),
                    (egui::Key::ArrowRight, "transport.frame_forward"),
                ] {
                    if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, key))
                        && let Some(preview) = &mut self.preview
                        && let Err(e) = preview.activate(id)
                    {
                        self.fail(e, Text::ShortsError);
                    }
                }
            }
        }
    }
    fn tools_ui(&mut self, ui: &mut egui::Ui) {
        if let Some(candidate) = self
            .source
            .as_ref()
            .and_then(|s| s.candidates.get(self.selected))
        {
            ui.label(candidate.proposal.reason.as_str());
        }
        ui.horizontal(|ui| {
            if self.button(ui, "shorts.accept", Text::ShortsAccept, Icon::Plus, true)
                && let Err(e) = self.decide(Decision::Accepted)
            {
                self.fail(e, Text::ShortsError);
            }
            if self.button(ui, "shorts.reject", Text::ShortsReject, Icon::X, true)
                && let Err(e) = self.decide(Decision::Rejected)
            {
                self.fail(e, Text::ShortsError);
            }
        });
        ui.separator();
        ui.heading(self.tr(Text::ShortsSegments));
        let segments = self
            .reviews
            .get(self.selected)
            .map(|r| r.segments.clone())
            .unwrap_or_default();
        let mut trim = None;
        for (index, segment) in segments.iter().enumerate() {
            ui.label(catalog::format(
                self.preferences.locale,
                Text::ShortsRanks,
                &[
                    ("play_label", self.tr(Text::ShortsPlayOrder)),
                    (
                        "play",
                        &catalog::number(self.preferences.locale, (index + 1) as u64),
                    ),
                    ("source_label", self.tr(Text::ShortsSourceOrder)),
                    (
                        "source",
                        &catalog::number(
                            self.preferences.locale,
                            (segment.source_order + 1) as u64,
                        ),
                    ),
                    ("role", self.tr(role_text(segment.role))),
                ],
            ));
            let in_label = self.tr(Text::ShortsIn);
            let out_label = self.tr(Text::ShortsOut);
            if let Some((start, end)) = self.trim_drafts.get_mut(index) {
                ui.horizontal(|ui| {
                    ui.label(in_label);
                    let r = ui.add(
                        egui::TextEdit::singleline(start)
                            .char_limit(32)
                            .desired_width(76.0),
                    );
                    named(&r, &format!("shorts.segment.{index}.in"), in_label);
                    ui.label(out_label);
                    let r = ui.add(
                        egui::TextEdit::singleline(end)
                            .char_limit(32)
                            .desired_width(76.0),
                    );
                    named(&r, &format!("shorts.segment.{index}.out"), out_label);
                });
            }
            if self.button(
                ui,
                &format!("shorts.segment.{index}.trim"),
                Text::ShortsApplyTrim,
                Icon::Scissors,
                true,
            ) {
                trim = Some(index);
            }
            ui.add_space(8.0);
        }
        if let Some(index) = trim {
            let result = self
                .trim_drafts
                .get(index)
                .ok_or(Error::Invalid("trim draft"))
                .and_then(|(s, e)| {
                    Ok((
                        parse_seconds(self.preferences.locale, s)?,
                        parse_seconds(self.preferences.locale, e)?,
                    ))
                })
                .and_then(|(s, e)| self.trim(index, s, e));
            if let Err(e) = result {
                self.fail(e, Text::ShortsTrimError);
            }
        }
        ui.separator();
        ui.heading(self.tr(Text::ShortsCrop));
        let Some(before) = self.reviews.get(self.selected).cloned() else {
            return;
        };
        let mut rect = before.crop;
        let aspect = self
            .source
            .as_ref()
            .and_then(|s| s.project.data().assets.iter().find(|a| a.id() == s.asset))
            .and_then(|a| a.probe().streams.iter().find_map(|s| s.video.as_ref()))
            .map(|v| v.width as f32 / v.height.max(1) as f32)
            .unwrap_or(16.0 / 9.0);
        let response = match crop::control(
            ui,
            "shorts.crop",
            &mut rect,
            aspect,
            self.tr(Text::ShortsCrop),
        ) {
            Ok(response) => response,
            Err(e) => {
                self.fail(e, Text::ShortsError);
                return;
            }
        };
        self.crop_focused = response.has_focus();
        if response.drag_started() {
            self.crop_entry = Some(before.clone());
            self.crop_cancelled = false;
        }
        if rect != before.crop && !self.crop_cancelled {
            if response.dragged() {
                if let Some(row) = self.reviews.get_mut(self.selected) {
                    row.crop = rect;
                }
                self.preview_dirty = true;
            } else if let Err(e) = self.set_crop(rect) {
                self.fail(e, Text::ShortsError);
            }
        }
        if ui.input(|i| i.key_pressed(egui::Key::Escape))
            && let Some(entry) = self.crop_entry.take()
        {
            if let Some(row) = self.reviews.get_mut(self.selected) {
                *row = entry;
            }
            self.preview_dirty = true;
            self.crop_cancelled = true;
        }
        if response.drag_stopped()
            && let Some(entry) = self.crop_entry.take()
        {
            let after = self.reviews.get(self.selected).cloned();
            if let Some(after) = after {
                if let Some(row) = self.reviews.get_mut(self.selected) {
                    *row = entry;
                }
                if let Err(e) = self.commit(self.selected, after) {
                    self.fail(e, Text::ShortsError);
                }
            }
        }
        if response.drag_stopped() {
            self.crop_cancelled = false;
        }
        if self.button(
            ui,
            "shorts.crop.center",
            Text::ShortsCenter,
            Icon::CornersOut,
            true,
        ) {
            let mut center = before.crop;
            center.x = (1.0 - center.width) / 2.0;
            center.y = (1.0 - center.height) / 2.0;
            let result = self.set_crop(center);
            if let Err(e) = result {
                self.fail(e, Text::ShortsError);
            }
        }
        ui.label(self.tr(Text::ShortsCropTransient));
    }
    fn export_ui(&mut self, ui: &mut egui::Ui) {
        let Some(batch) = &self.batch else { return };
        ui.separator();
        ui.heading(self.tr(Text::ShortsExporting));
        let mut retry = None;
        let mut cancel = None;
        egui::ScrollArea::vertical()
            .id_salt("shorts.export.jobs")
            .max_height(130.0)
            .show(ui, |ui| {
                for index in 0..batch.len() {
                    if let Some(snapshot) = batch.snapshot(&self.runner, index) {
                        ui.horizontal(|ui| {
                            if let Some(paths) = batch.paths(index) {
                                let name = paths
                                    .video
                                    .file_name()
                                    .unwrap_or_default()
                                    .to_string_lossy();
                                ui.add_sized(
                                    [240.0, 26.0],
                                    egui::Label::new(name.as_ref()).truncate(),
                                )
                                .on_hover_text(name.as_ref());
                            }
                            let key = match snapshot.state {
                                JobState::Completed => Text::ShortsDone,
                                JobState::Failed => Text::ShortsJobFailed,
                                JobState::Cancelled => Text::ShortsJobCancelled,
                                _ => Text::Processing,
                            };
                            ui.label(self.tr(key));
                            if let Some(error) = &snapshot.error {
                                ui.label(self.tr(failure_text(error)));
                            }
                            if !terminal(snapshot.state) {
                                ui.vertical(|ui| {
                                    ui.set_width(180.0);
                                    crate::export::job_progress(
                                        ui,
                                        &snapshot,
                                        self.preferences.locale,
                                    );
                                });
                            }
                            if matches!(snapshot.state, JobState::Failed | JobState::Cancelled)
                                && self.button(
                                    ui,
                                    &format!("shorts.export.{index}.retry"),
                                    Text::Retry,
                                    Icon::ArrowClockwise,
                                    true,
                                )
                            {
                                retry = Some(index);
                            }
                            if !terminal(snapshot.state)
                                && self.button(
                                    ui,
                                    &format!("shorts.export.{index}.cancel"),
                                    Text::Cancel,
                                    Icon::X,
                                    true,
                                )
                            {
                                cancel = Some(index);
                            }
                        });
                    }
                }
            });
        if let Some(index) = cancel
            && let Some(batch) = &self.batch
            && let Err(e) = batch.cancel(index)
        {
            self.fail(e.into(), Text::ShortsError);
        }
        if let Some(index) = retry
            && let Err(e) = self.retry_export(index)
        {
            self.fail(e, Text::ShortsError);
        }
    }
}
fn author(response: &egui::Response, id: &str) {
    response
        .ctx
        .accesskit_node_builder(response.id, |node| node.set_author_id(id));
}
fn decision_text(decision: Decision) -> Text {
    match decision {
        Decision::Pending => Text::ShortsPending,
        Decision::Accepted => Text::ShortsAccepted,
        Decision::Rejected => Text::ShortsRejected,
    }
}
fn role_text(role: ClipRole) -> Text {
    match role {
        ClipRole::Hook => Text::ShortsHook,
        ClipRole::Context => Text::ShortsContext,
        ClipRole::Payoff => Text::ShortsPayoff,
    }
}
fn stage_text(stage: Stage) -> Text {
    match stage {
        Stage::SuggestClips => Text::ShortsGenerate,
        Stage::Snap => Text::Snap,
        Stage::Transcribe | Stage::Align | Stage::Assemble | Stage::Segment => {
            Text::ShortsTranscriber
        }
        Stage::Extract => Text::Importing,
        Stage::Translate => Text::Language,
        Stage::ExportSrt | Stage::ExportBurn => Text::ShortsExporting,
    }
}
fn cost_text(locale: Locale, cost: &Cost) -> String {
    match cost {
        Cost::Free => catalog::text(locale, Text::ShortsFree).into(),
        Cost::Unknown => catalog::text(locale, Text::ShortsUnknownCost).into(),
        Cost::UpperBound { currency, micros } => catalog::format(
            locale,
            Text::ShortsMoney,
            &[
                ("currency", currency),
                (
                    "amount",
                    &catalog::decimal(locale, *micros as f64 / 1_000_000.0, 6),
                ),
            ],
        ),
    }
}

fn named(response: &egui::Response, id: &str, label: &str) {
    author(response, id);
    response
        .ctx
        .accesskit_node_builder(response.id, |node| node.set_label(label));
}

fn choice_label(ui: &egui::Ui, selected: bool, text: impl Into<String>) -> egui::RichText {
    egui::RichText::new(text).color(if selected {
        ui.visuals().selection.stroke.color
    } else {
        ui.visuals().text_color()
    })
}

fn provider_label(locale: Locale, provider: &str, model: &str) -> String {
    catalog::format(
        locale,
        Text::ShortsProviderModel,
        &[("provider", provider), ("model", model)],
    )
}

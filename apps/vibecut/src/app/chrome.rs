//! The window chrome: loading and picker placeholders, recovery, error and close prompts.
use super::{picker::PickerAction, window::EditorWindow, work::Work};
use crate::{
    Editor, Error, egui,
    i18n::{self, Text},
    kit::{self as icons, Icon},
    services::lifecycle::{EditorAction, FEEDBACK_URL, HELP_URL},
};
use engine::control::Error as EngineError;
use std::time::Duration;

impl EditorWindow {
    pub(super) fn tr(&self, text: Text) -> &'static str {
        i18n::text(self.preferences.locale, text)
    }
    fn button(&self, ui: &mut egui::Ui, id: &'static str, icon: Icon, text: Text) -> bool {
        let image = icons::tinted(icon, 16.0, ui.visuals().text_color());
        let response = ui
            .push_id(id, |ui| {
                ui.add_sized(
                    [160.0, 34.0],
                    egui::Button::image_and_text(image, self.tr(text)),
                )
            })
            .inner;
        response
            .ctx
            .accesskit_node_builder(response.id, |node| node.set_author_id(id));
        response.clicked()
    }
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        if !self.configured {
            Editor::fonts(ui.ctx());
            icons::install(ui.ctx());
            self.configured = true;
        }
        Editor::theme(ui.ctx(), self.preferences.dark);
        if self.project_window {
            self.tick();
        }
        if !self.busy()
            && let Some(item) = self
                .editor
                .as_mut()
                .and_then(|editor| editor.take_replace_request())
            && let Err(error) = self.pick(ui.ctx(), PickerAction::ReplaceMedia { item })
        {
            self.error = Some(error);
        }
        if self.open_file_requested && !self.busy() {
            self.open_file_requested = false;
            if let Err(error) = self.pick(ui.ctx(), PickerAction::Open) {
                self.error = Some(error);
            }
        }
        if self.project_window
            && ui.input(|i| i.viewport().close_requested())
            && !self.ready_to_close
        {
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.request_close();
        }
        if let Some(editor) = &mut self.editor {
            editor.overlay_open = self.close_prompt
                || self.error.is_some()
                || self.picker.is_some()
                || self.pending.is_some()
                || self.preference_warning
                || !self.diagnostics.is_empty();
            editor.ui(ui);
            self.title = editor.session.project().data().name.as_str().to_owned();
            self.preferences = editor.preferences.clone();
            if let Some(action) = editor.lifecycle_action.take() {
                let result = match action {
                    EditorAction::ExportSave => self.pick(ui.ctx(), PickerAction::ExportSave),
                    EditorAction::SnapshotSave => self.pick(ui.ctx(), PickerAction::SnapshotSave),
                    EditorAction::ImportMedia => self.pick(ui.ctx(), PickerAction::ImportMedia),
                    EditorAction::ImportLut => self.pick(ui.ctx(), PickerAction::ImportLut),
                    EditorAction::ExportClips => self.pick(ui.ctx(), PickerAction::ExportClips),
                    EditorAction::ExportCaption(output) => {
                        self.pick(ui.ctx(), PickerAction::ExportCaption(output))
                    }
                    EditorAction::ImportSubtitles { at } => {
                        self.pick(ui.ctx(), PickerAction::ImportSubtitles { at })
                    }
                    EditorAction::FontUpload => self.pick(ui.ctx(), PickerAction::FontUpload),
                    EditorAction::KeybindingsExport => {
                        self.pick(ui.ctx(), PickerAction::KeybindingsExport)
                    }
                    EditorAction::KeybindingsImport { replace } => {
                        self.pick(ui.ctx(), PickerAction::KeybindingsImport { replace })
                    }
                    EditorAction::FocusHome => {
                        self.focus_home = true;
                        Ok(())
                    }
                    EditorAction::ExitProject => {
                        self.request_close();
                        Ok(())
                    }
                    EditorAction::DeleteProject => {
                        self.delete_requested = true;
                        Ok(())
                    }
                    EditorAction::OpenHelp => {
                        ui.ctx().open_url(egui::OpenUrl::new_tab(HELP_URL));
                        Ok(())
                    }
                    EditorAction::OpenFeedback => {
                        ui.ctx().open_url(egui::OpenUrl::new_tab(FEEDBACK_URL));
                        Ok(())
                    }
                    EditorAction::SaveAs => self.save_from_ui(ui.ctx(), false, false, true),
                    EditorAction::Save => self.save_from_ui(ui.ctx(), false, false, false),
                };
                if let Err(e) = result {
                    self.error = Some(e);
                }
            }
        } else if let Some(pending) = self.pending.as_ref() {
            let text = pending.text;
            let mut cancel = false;
            ui.vertical_centered(|ui| {
                ui.add_space(100.0);
                ui.heading(egui::RichText::new(self.tr(Text::AppName)).size(28.0));
                ui.add_space(28.0);
                ui.spinner();
                ui.label(self.tr(text));
                if matches!(text, Text::LoadingProject)
                    && self.button(ui, "project.operation.cancel", Icon::X, Text::Cancel)
                {
                    cancel = true;
                }
            });
            if cancel {
                self.cancel_pending();
            }
        } else if self.picker.is_some() {
            ui.vertical_centered(|ui| {
                ui.add_space(100.0);
                ui.heading(egui::RichText::new(self.tr(Text::AppName)).size(28.0));
                ui.add_space(28.0);
                ui.label(self.tr(Text::ChooseProject));
            });
        }
        if self.editor.is_none() && !self.busy() && !self.candidates.is_empty() {
            egui::Modal::new(egui::Id::unique("recovery.prompt"))
                .frame(crate::kit::dialog_frame(ui.ctx()))
                .show(ui.ctx(), |ui| {
                    crate::kit::dialog_title(ui, self.tr(Text::Recovery));
                    if let Some(candidate) = self.candidates.first() {
                        ui.label(&candidate.journal.name);
                    }
                    ui.label(self.tr(Text::SaveAsNeeded));
                    ui.horizontal(|ui| {
                        if self.button(
                            ui,
                            "recovery.recover",
                            Icon::ArrowCounterClockwise,
                            Text::Recover,
                        ) {
                            let candidate = self.candidates.remove(0);
                            if let Err(e) = self
                                .submit(Work::Recover(Box::new(candidate)), Text::LoadingProject)
                            {
                                self.error = Some(e);
                            }
                        }
                        if self.button(ui, "recovery.discard", Icon::Trash, Text::Discard) {
                            let candidate = self.candidates.remove(0);
                            if let Err(e) =
                                self.submit(Work::Discard(Box::new(candidate)), Text::Discovering)
                            {
                                self.error = Some(e);
                            }
                        }
                    });
                });
        }
        if !self.diagnostics.is_empty() {
            egui::Window::new(self.tr(Text::RecoveryDiagnostic))
                .id(egui::Id::unique("recovery.diagnostic"))
                .show(ui.ctx(), |ui| {
                    for e in &self.diagnostics {
                        ui.label(e.to_string());
                    }
                    if self.button(ui, "recovery.diagnostic.close", Icon::X, Text::Cancel) {
                        self.diagnostics.clear();
                    }
                });
        }
        if self.preference_warning {
            egui::Window::new(self.tr(Text::PreferencesWarning))
                .id(egui::Id::unique("preferences.warning"))
                .collapsible(false)
                .show(ui.ctx(), |ui| {
                    ui.label(self.tr(Text::PreferencesReset));
                    if self.button(ui, "preferences.warning.close", Icon::X, Text::Ok) {
                        self.preference_warning = false;
                    }
                });
        }
        if self.error.is_some() {
            self.close_prompt = false;
            egui::Window::new(self.tr(Text::ProjectError))
                .id(egui::Id::unique("project.error"))
                .collapsible(false)
                .show(ui.ctx(), |ui| {
                    let text = match self.error.as_ref() {
                        Some(Error::Engine(EngineError::Edit(engine::EditError::Project(
                            project::Error::TooNew { .. },
                        ))))
                        | Some(Error::Project(project::Error::TooNew { .. })) => Text::TooNew,
                        Some(Error::Engine(EngineError::Edit(engine::EditError::Project(
                            project::Error::TooOld { .. },
                        ))))
                        | Some(Error::Project(project::Error::TooOld { .. })) => Text::TooOld,
                        Some(Error::Engine(EngineError::Edit(engine::EditError::Project(
                            project::Error::Busy,
                        ))))
                        | Some(Error::Project(project::Error::Busy)) => Text::Busy,
                        Some(Error::Engine(EngineError::Edit(engine::EditError::Project(
                            project::Error::Conflict,
                        ))))
                        | Some(Error::Project(project::Error::Conflict)) => Text::Conflict,
                        _ => Text::ProjectError,
                    };
                    ui.label(self.tr(text));
                    if let Some(e) = &self.error {
                        ui.label(e.to_string());
                        let mut source = std::error::Error::source(e);
                        for _ in 0..8 {
                            let Some(error) = source else {
                                break;
                            };
                            ui.label(error.to_string());
                            source = error.source();
                        }
                    }
                    ui.horizontal(|ui| {
                        if self.button(
                            ui,
                            "project.retry",
                            Icon::ArrowClockwise,
                            Text::RetryProject,
                        ) {
                            self.error = None;
                            let result = if let Some((path, leave, close, journal_only)) =
                                self.retry_save.take()
                            {
                                self.persist(path, leave, close, journal_only)
                            } else if self.retry_discovery {
                                self.candidates.clear();
                                self.submit(Work::Discover, Text::Discovering)
                            } else if let Some(path) = self.retry_path.clone() {
                                self.open(path)
                            } else if self.editor.is_none() {
                                self.create()
                            } else {
                                Ok(())
                            };
                            if let Err(e) = result {
                                self.error = Some(e);
                            }
                        }
                        if self.button(
                            ui,
                            "project.error.back",
                            Icon::ArrowLeft,
                            if self.editor.is_some() {
                                Text::Cancel
                            } else {
                                Text::BackProjects
                            },
                        ) {
                            self.error = None;
                            self.retry_path = None;
                            self.startup_path = None;
                        }
                    });
                });
        }
        if self.close_prompt && !self.busy() {
            egui::Modal::new(egui::Id::unique("project.close.prompt"))
                .frame(crate::kit::dialog_frame(ui.ctx()))
                .show(ui.ctx(), |ui| {
                    crate::kit::dialog_title(ui, self.tr(Text::ClosePrompt));
                    if self.button(ui, "project.close.save", Icon::FloppyDisk, Text::SaveClose)
                        && let Err(e) = self.save_from_ui(ui.ctx(), true, true, false)
                    {
                        self.error = Some(e);
                    }
                    if self.button(ui, "project.close.recover", Icon::File, Text::Close)
                        && let Err(e) = self.persist(None, true, true, true)
                    {
                        self.error = Some(e);
                    }
                    if self.button(ui, "project.close.cancel", Icon::X, Text::Cancel) {
                        self.close_prompt = false;
                    }
                });
        }
        if self.ready_to_close {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
        ui.ctx().request_repaint_after(Duration::from_millis(16));
    }
}

//! Draws Home, every project viewport and the Shorts viewport from the registry.
use super::{geometry::remember_geometry, registry::Workspace, work::Work};
use crate::{
    Editor, egui, home,
    i18n::{self, Text},
    kit as icons,
    preferences::WindowGeometry,
};

impl Workspace {
    /// Draw a specific project through the same path as its native viewport.
    /// Kittest callers select the project explicitly, preserving automation ids.
    pub fn window_ui(&mut self, id: project::ProjectId, ui: &mut egui::Ui) {
        let owner = self
            .windows
            .get(&id)
            .and_then(|w| w.editor.as_ref())
            .map(|e| {
                egui::Id::unique((
                    e.session.session_id(),
                    e.session.project().data().id,
                    e.session.project().data().current_scene_id,
                    e.selection(),
                ))
            })
            .unwrap_or_else(|| egui::Id::unique(("loading", id)));
        self.text_clipboard.bridge(ui.ctx(), owner);
        if let Some(window) = self.windows.get_mut(&id) {
            window.preferences = self.preferences.clone();
            if let Some(editor) = &mut window.editor {
                editor.preferences = self.preferences.clone();
                editor.set_models_visible(self.focused == Some(id));
                self.lending.lend_models(editor);
            }
            window.ui(ui);
            self.lending.reclaim_models(window);
            self.preferences = window.preferences.clone();
            if let Some((from, to)) = window.saved_as.take() {
                self.preferences.copy_timeline_lanes(from, to);
            }
            if std::mem::take(&mut window.focus_home) {
                self.home_open = true;
                ui.ctx().send_viewport_cmd_to(
                    egui::ViewportId::ROOT,
                    egui::ViewportCommand::Visible(true),
                );
                ui.ctx().send_viewport_cmd_to(
                    egui::ViewportId::ROOT,
                    egui::ViewportCommand::Minimized(false),
                );
                ui.ctx()
                    .send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Focus);
            }
        }
    }
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        self.attach(ui.ctx());
        self.tick();
        if !self.home.configured {
            Editor::fonts(ui.ctx());
            icons::install(ui.ctx());
            self.home.configured = true;
        }
        Editor::theme(ui.ctx(), self.preferences.dark);
        let ctx = ui.ctx().clone();
        if self.home_open {
            self.text_clipboard.bridge(
                ui.ctx(),
                egui::Id::unique(("home.text", self.projects.selection())),
            );
        }
        // Our own final `Close` returns here as `close_requested`; cancelling it
        // would leave a hidden process running after the last window closed.
        if ui.input(|i| i.viewport().close_requested()) && !self.ready_to_close() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.request_close();
        }
        self.show_home(ui);
        self.show_projects(&ctx);
        self.show_shorts(&ctx);
        if let Some(geometry) = WindowGeometry::read(ui) {
            remember_geometry(&mut self.preferences, "home".into(), geometry);
        }
        self.retain_text_viewports(&ctx);
        if self.ready_to_close() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        // Work the UI just asked for starts on a tick, and nothing polls while idle: run it
        // now. What it publishes wakes the loop again on its own.
        self.tick();
    }
    /// The root viewport: Home's catalog over its lifecycle worker's chrome, or hidden.
    fn show_home(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        if self.home_open {
            self.home.preferences = self.preferences.clone();
            self.home.ui(ui);
            if !self.home.busy() && self.home.candidates.is_empty() && self.home.error.is_none() {
                let status = self.shorts.as_ref().and_then(|shorts| {
                    if shorts.cancelling() {
                        Some(Text::ShortsCancelling)
                    } else if shorts.awaiting_consent() {
                        Some(Text::ShortsApproval)
                    } else if shorts.exporting() {
                        Some(Text::ShortsExporting)
                    } else if shorts.busy() {
                        Some(Text::ShortsCreating)
                    } else if shorts.source().is_some() {
                        Some(Text::ShortsReview)
                    } else {
                        None
                    }
                });
                let cx = home::Context {
                    look: self.preferences.look(),
                    default_fps: self.preferences.default_fps,
                    projects: &mut self.preferences.projects,
                    status,
                };
                for action in home::show(&mut self.projects, cx, ui) {
                    self.project_action(action);
                }
            }
            self.home.preferences = self.preferences.clone();
        } else {
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaintWhileHidden);
        }
    }
    /// One immediate viewport per open project, plus the focus request a reopen made.
    fn show_projects(&mut self, ctx: &egui::Context) {
        let ids: Vec<_> = self.windows.keys().copied().collect();
        for id in ids {
            let Some(viewport) = self.viewport(id) else {
                continue;
            };
            let title = self
                .windows
                .get(&id)
                .map(|window| window.title().to_owned())
                .unwrap_or_else(|| i18n::text(self.preferences.locale, Text::AppName).to_owned());
            let geometry = self
                .preferences
                .windows
                .get(&format!("editor.{}", id.0))
                .cloned()
                .unwrap_or_default();
            ctx.show_viewport_immediate(viewport, geometry.builder().with_title(title), |ui, _| {
                if ui.input(|i| i.focused) {
                    self.focused = Some(id);
                }
                self.window_ui(id, ui);
                if let Some(geometry) = WindowGeometry::read(ui) {
                    remember_geometry(&mut self.preferences, format!("editor.{}", id.0), geometry);
                }
            });
        }
        if let Some(id) = self.focus_requested.take()
            && let Some(viewport) = self.viewport(id)
        {
            ctx.send_viewport_cmd_to(viewport, egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd_to(viewport, egui::ViewportCommand::Focus);
        }
    }
    /// The long-video-to-shorts viewport and the editor session it may ask for.
    fn show_shorts(&mut self, ctx: &egui::Context) {
        if self.shorts_window
            && let Some(shorts) = &mut self.shorts
        {
            let cx = crate::shorts::Context {
                editor_available: !self.home.busy(),
            };
            let mut close = false;
            let mut actions = Vec::new();
            ctx.show_viewport_immediate(
                egui::ViewportId::from_hash_of("shorts.window"),
                self.preferences
                    .windows
                    .get("shorts")
                    .cloned()
                    .unwrap_or(WindowGeometry {
                        size: crate::shorts::WINDOW,
                        ..Default::default()
                    })
                    .builder()
                    .with_title(i18n::text(shorts.preferences.locale, Text::Shorts)),
                |ui, _| {
                    shorts.apply_model_preferences(&self.preferences.ai_models);
                    self.text_clipboard.bridge(
                        ui.ctx(),
                        egui::Id::unique((
                            "shorts.text",
                            shorts.source().map(|s| s.project.data().id),
                            shorts.preview_session().map(|s| s.session_id()),
                            shorts.selected(),
                        )),
                    );
                    actions = crate::shorts::show(shorts, cx, ui);
                    close = ui.input(|i| i.viewport().close_requested());
                    if let Some(geometry) = WindowGeometry::read(ui) {
                        remember_geometry(&mut self.preferences, "shorts".into(), geometry);
                    }
                },
            );
            if close {
                shorts.pause_preview();
                self.shorts_window = false;
            }
            for action in &actions {
                if let crate::shorts::Action::Pick(kind) = action {
                    self.shorts_picker = Some(super::picker::shorts_picker(
                        ctx,
                        *kind,
                        shorts.preferences.locale,
                    ));
                }
            }
            if actions.contains(&crate::shorts::Action::OpenEditor) && !self.home.busy() {
                let result = shorts.editor_session().and_then(|(s, settings)| {
                    self.home.submit(
                        Work::Short(Box::new(s), Box::new(settings)),
                        Text::LoadingProject,
                    )
                });
                if let Err(e) = result {
                    self.home.error = Some(e);
                }
            }
        }
    }
    /// Tell the text service which viewports are alive so it drops the others' requests.
    fn retain_text_viewports(&mut self, ctx: &egui::Context) {
        let mut text_viewports: Vec<_> = self
            .windows
            .keys()
            .filter_map(|id| self.viewport(*id))
            .collect();
        if self.home_open {
            text_viewports.push(egui::ViewportId::ROOT);
        }
        if self.shorts_window {
            text_viewports.push(egui::ViewportId::from_hash_of("shorts.window"));
        }
        if ctx.embed_viewports() && !text_viewports.is_empty() {
            text_viewports = vec![egui::ViewportId::ROOT];
        }
        self.text_clipboard.retain_viewports(ctx, &text_viewports);
    }
}

//! Composes the editor window.
use crate::editor::{
    Arc, Editor, Id, Rect, Text, Vec2, drag_panel, egui, error_detail, icons, theme,
};

impl Editor {
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        if self.color_picker.as_ref().is_some_and(|picker| {
            picker
                .target
                .item()
                .is_some_and(|item| self.selected_item_id() != Some(item))
        }) || self
            .color_base
            .as_ref()
            .is_some_and(|base| !Arc::ptr_eq(base, self.session.project()))
        {
            self.close_color_picker();
        }
        // The gesture belongs to the item and field visible at pointer-down.
        // Agents can change selection or panel state between pointer events.
        if self
            .effect_reorder
            .as_ref()
            .is_some_and(|drag| self.selected_item_id() != Some(drag.item) || self.details)
        {
            self.effect_reorder = None;
        }
        if self
            .scrub
            .as_ref()
            .is_some_and(|scrub| self.selected_item_id() != Some(scrub.item) || self.details)
        {
            self.finish_scrub(false);
        }
        // History and external commands can remove the track being renamed.
        if self.track_rename.as_ref().is_some_and(|(id, _)| {
            !self
                .session
                .project()
                .data()
                .scene()
                .timeline
                .iter()
                .any(|track| track.id == *id)
        }) {
            self.track_rename = None;
            self.track_rename_focus = false;
        }
        if !self.session.transaction_active()
            && let Err(e) = self.session.set_selection(self.selection.clone())
        {
            self.error = Some(e.to_string());
        }
        if !self.configured {
            Self::fonts(ui.ctx());
            icons::install(ui.ctx());
            self.configured = true;
        }
        let (dropped, hovering) = ui.ctx().input(|input| {
            (
                input
                    .raw
                    .dropped_files
                    .iter()
                    .map(|file| file.path().to_owned())
                    .collect::<Vec<_>>(),
                !input.raw.hovered_files.is_empty(),
            )
        });
        if !dropped.is_empty()
            && let Err(error) = self.drop_files(dropped)
        {
            self.import_notice = Some(error_detail(&error));
        }
        Self::theme(ui.ctx(), self.preferences.dark);
        crate::kit::install_editor_density(ui.ctx());
        ui.style_mut().interaction.selectable_labels = false;
        self.widgets.clear();
        self.panels.clear();
        self.tick();
        // Undo and commands can remove a scene without the sheet's delete action.
        if self
            .scene_rename
            .as_ref()
            .is_some_and(|(scene, _)| self.session.project().data().scenes.get(*scene).is_none())
        {
            self.scene_rename = None;
        }
        if !self.export.as_ref().is_some_and(|d| d.open)
            && !self.subtitles.as_ref().is_some_and(|s| s.modal_open())
        {
            self.keyboard(ui.ctx());
        }
        if self.preview_fullscreen != self.fullscreen_commanded {
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.preview_fullscreen));
            self.fullscreen_commanded = self.preview_fullscreen;
        }
        if self.preview_fullscreen {
            let rect = ui.max_rect();
            let rect = Rect::from_min_size(rect.min, rect.size().max(Vec2::ZERO));
            ui.painter().rect_filled(
                rect,
                theme::radius::PANEL,
                theme::window(self.preferences.dark),
            );
            let mut child = ui.new_child(
                egui::UiBuilder::new()
                    .id_salt("preview.fullscreen")
                    .max_rect(rect.shrink2(Vec2::new(
                        16.0_f32.min(rect.width() / 2.0),
                        16.0_f32.min(rect.height() / 2.0),
                    ))),
            );
            child.set_clip_rect(rect);
            super::super::player::show(self, &mut child);
            self.toast(ui.ctx());
            return;
        }
        let rect = ui.max_rect();
        let rect = Rect::from_min_size(rect.min, rect.size().max(Vec2::ZERO));
        ui.painter().rect_filled(
            rect,
            theme::radius::PANEL,
            theme::window(self.preferences.dark),
        );
        let super::layout::Layout {
            top,
            body,
            upper,
            left,
            player: preview,
            right,
            timeline,
        } = super::layout::Layout::new(rect, self.preferences.panels);
        for (id, area) in [
            ("header", top),
            ("media", left),
            ("preview", preview),
            ("inspector", right),
            ("timeline", timeline),
        ] {
            self.panels.insert(id, area);
            let fill = if id == "header" {
                theme::window(self.preferences.dark)
            } else {
                theme::panel(self.preferences.dark)
            };
            let radius = if id == "header" {
                theme::radius::PANEL
            } else {
                theme::radius::FRAME_PANEL
            };
            ui.painter().rect_filled(area, radius, fill);
            let mut child = ui.new_child(egui::UiBuilder::new().id_salt(id).max_rect(area));
            child.set_clip_rect(area);
            if area.width() <= 0.0 || area.height() <= 0.0 {
                continue;
            }
            match id {
                "header" => self.header(&mut child),
                "media" => super::super::assets::show(self, &mut child),
                "preview" => super::super::player::show(self, &mut child),
                "inspector" => super::super::properties::show(self, &mut child),
                _ => super::super::timeline::show(self, &mut child),
            }
        }
        for (id, area, axis) in [
            (
                "layout.media",
                Rect::from_center_size(
                    egui::pos2((left.right() + preview.left()) / 2.0, upper.center().y),
                    Vec2::new(theme::space::PANEL_GUTTER, upper.height()),
                ),
                0,
            ),
            (
                "layout.inspector",
                Rect::from_center_size(
                    egui::pos2((preview.right() + right.left()) / 2.0, upper.center().y),
                    Vec2::new(theme::space::PANEL_GUTTER, upper.height()),
                ),
                1,
            ),
            (
                "layout.timeline",
                Rect::from_center_size(
                    egui::pos2(timeline.center().x, (upper.bottom() + timeline.top()) / 2.0),
                    Vec2::new(timeline.width(), theme::space::SM),
                ),
                2,
            ),
        ] {
            let response = ui.interact(area, Id::unique(id), egui::Sense::drag());
            self.remember(id, &response);
            if response.drag_started() {
                self.frame.layout_drag = Some(self.preferences.panels);
            }
            if response.dragged() {
                let delta = response.drag_delta();
                let fraction = match axis {
                    0 | 1 => delta.x / (left.width() + preview.width() + right.width()).max(1.0),
                    _ => delta.y / (body.height() - theme::space::SM).max(1.0),
                };
                self.preferences.panels = drag_panel(self.preferences.panels, axis, fraction);
            }
            if response.drag_stopped()
                && let Some(before) = self.frame.layout_drag.take()
            {
                self.record_layout_change(before);
            }
        }
        super::super::timeline::show_scenes(self, ui.ctx());
        self.media_delete_dialog(ui.ctx());
        self.import_overlay(ui.ctx());
        self.drop_overlay(ui.ctx(), hovering);
        self.toast(ui.ctx());
        super::super::export::show(self, ui.ctx());
        if let Some(subtitles) = &mut self.subtitles {
            let widgets = subtitles.dialogs(
                ui.ctx(),
                &self.session,
                &self.preferences.subtitles,
                self.preferences.locale,
            );
            for (id, response) in widgets {
                self.remember(id, &response);
            }
        }
        super::super::models::show(self, ui.ctx());
        super::super::properties::show_dialog(self, ui.ctx());
        self.shortcuts_dialog(ui.ctx());
        if self
            .scrub
            .as_ref()
            .is_some_and(|scrub| !self.widgets.contains_key(&scrub.target.widget_id()))
        {
            self.finish_scrub(false);
        }
        if self.effect_reorder.as_ref().is_some_and(|drag| {
            !self
                .widgets
                .contains_key(&format!("effects.row.{}.handle", drag.effect))
        }) {
            self.effect_reorder = None;
        }
        self.finish_effect_drag(ui.ctx());
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(
                if self.playback.playing() { 8 } else { 33 },
            ));
    }
    pub(in crate::editor) fn header(&mut self, ui: &mut egui::Ui) {
        let save_status = if self.persistence_error.is_some() {
            Text::SaveFailed
        } else {
            match &self.session {
                crate::lifecycle::EditorSession::Memory(_) => Text::NotSaved,
                crate::lifecycle::EditorSession::Persistent(autosave) => {
                    if autosave.saving() || autosave.journaled_revision() < autosave.revision() {
                        Text::SavingProject
                    } else {
                        Text::AutoSaved
                    }
                }
            }
        };
        let cx = crate::editor::Context {
            session: &self.session,
            selection: &self.selection,
            locale: self.preferences.locale,
            dark: self.preferences.dark,
            save_status,
        };
        let actions = super::show(&mut self.frame, &cx, ui);
        for (id, icon, response) in std::mem::take(&mut self.frame.widgets) {
            self.remember_with_icon(id, icon, &response);
        }
        for action in actions {
            match action {
                super::Action::Activate(id) => self.action(id),
                super::Action::Rename(name) => {
                    if let Err(error) = self
                        .session
                        .execute("project.rename", serde_json::json!({"name":name}))
                    {
                        self.error = Some(error.to_string());
                        self.frame.rename = Some(name);
                    }
                }
            }
        }
    }
}

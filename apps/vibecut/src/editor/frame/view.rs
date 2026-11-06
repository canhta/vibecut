//! Composes the editor window.
use crate::editor::{
    Arc, BUTTON_ICON, Editor, Icon, Id, Rect, Text, Vec2, drag_panel, egui, error_detail, icons,
    theme,
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
        let top = Rect::from_min_size(rect.min, Vec2::new(rect.width(), rect.height().min(52.0)));
        let body = Rect::from_min_max(egui::pos2(rect.left(), top.bottom()), rect.max);
        let timeline_height = body.height() * self.preferences.panels[2];
        let upper = Rect::from_min_max(
            body.min,
            egui::pos2(body.right(), body.bottom() - timeline_height),
        );
        let timeline = Rect::from_min_max(egui::pos2(body.left(), upper.bottom()), body.max);
        let left_w = upper.width() * self.preferences.panels[0];
        let right_w = upper.width() * self.preferences.panels[1];
        let left = Rect::from_min_size(upper.min, Vec2::new(left_w, upper.height()));
        let right = Rect::from_min_max(egui::pos2(upper.right() - right_w, upper.top()), upper.max);
        let preview = Rect::from_min_max(
            egui::pos2(left.right(), upper.top()),
            egui::pos2(right.left(), upper.bottom()),
        );
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
            ui.painter().rect_filled(area, theme::radius::PANEL, fill);
            let mut child = ui.new_child(egui::UiBuilder::new().id_salt(id).max_rect(
                area.shrink2(Vec2::new(
                    12.0_f32.min(area.width() / 2.0),
                    12.0_f32.min(area.height() / 2.0),
                )),
            ));
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
        let separator = theme::separator(self.preferences.dark);
        for (id, area, axis) in [
            (
                "layout.media",
                Rect::from_center_size(
                    egui::pos2(left.right(), upper.center().y),
                    Vec2::new(8.0, upper.height()),
                ),
                0,
            ),
            (
                "layout.inspector",
                Rect::from_center_size(
                    egui::pos2(right.left(), upper.center().y),
                    Vec2::new(8.0, upper.height()),
                ),
                1,
            ),
            (
                "layout.timeline",
                Rect::from_center_size(
                    egui::pos2(timeline.center().x, timeline.top()),
                    Vec2::new(timeline.width(), 8.0),
                ),
                2,
            ),
        ] {
            let response = ui.interact(area, Id::unique(id), egui::Sense::drag());
            self.remember(id, &response);
            if response.drag_started() {
                self.layout_drag = Some(self.preferences.panels);
            }
            if response.dragged() {
                let delta = response.drag_delta();
                let fraction = match axis {
                    0 | 1 => delta.x / upper.width(),
                    _ => delta.y / body.height(),
                };
                self.preferences.panels = drag_panel(self.preferences.panels, axis, fraction);
            }
            if response.drag_stopped()
                && let Some(before) = self.layout_drag.take()
            {
                self.record_layout_change(before);
            }
        }
        for (a, b) in [
            (top.left_bottom(), top.right_bottom()),
            (left.right_top(), left.right_bottom()),
            (right.left_top(), right.left_bottom()),
            (timeline.left_top(), timeline.right_top()),
        ] {
            ui.painter()
                .line_segment([a, b], egui::Stroke::new(1.0, separator));
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
        ui.horizontal_centered(|ui| {
            ui.label(
                egui::RichText::new(self.tr(Text::AppName))
                    .strong()
                    .size(18.0),
            );
            ui.separator();
            let window_menu = ui.menu_button(self.tr(Text::Windows), |ui| {
                if self.button(ui, "window.home", Icon::SquaresFour, Text::Home, true) {
                    self.action("window.home");
                    ui.close();
                }
            });
            self.remember("window.menu", &window_menu.response);
            if self.rename.is_some() {
                let mut name = self.rename.take().unwrap_or_default();
                let response = ui.add(
                    egui::TextEdit::singleline(&mut name)
                        .id_salt("project.rename.input")
                        .desired_width(230.0),
                );
                self.remember("project.rename.input", &response);
                if self.rename_focus {
                    response.request_focus();
                    self.rename_focus = false;
                }
                let cancel = ui.input(|i| i.key_pressed(egui::Key::Escape));
                let commit = ui.input(|i| i.key_pressed(egui::Key::Enter));
                if commit && !cancel {
                    if let Err(e) = self
                        .session
                        .execute("project.rename", serde_json::json!({"name": name}))
                    {
                        self.error = Some(e.to_string());
                        self.rename = Some(name);
                    }
                } else if !cancel {
                    self.rename = Some(name);
                }
            } else {
                let image = icons::tinted(Icon::PencilSimple, BUTTON_ICON, self.text_color());
                let response = ui.add_sized(
                    [220.0, 26.0],
                    egui::Button::image_and_text(
                        image,
                        egui::RichText::new(self.session.project().data().name.as_str()),
                    )
                    .truncate(),
                );
                self.remember_with_icon(
                    "project.rename",
                    Some(Icon::PencilSimple.name()),
                    &response,
                );
                if response.clicked() {
                    self.action("project.rename");
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if self.button(ui, "file.export", Icon::Export, Text::Export, true) {
                    self.action("file.export");
                }
                if self.button(
                    ui,
                    "project.save_as",
                    Icon::FloppyDiskBack,
                    Text::SaveAs,
                    true,
                ) {
                    self.action("project.save_as");
                }
                if self.button(ui, "project.save", Icon::FloppyDisk, Text::Save, true) {
                    self.action("project.save");
                }
                if self.button(ui, "appearance.theme", Icon::CircleHalf, Text::Theme, true) {
                    self.action("appearance.theme");
                }
                if self.button(
                    ui,
                    "appearance.locale",
                    Icon::Translate,
                    Text::Language,
                    true,
                ) {
                    self.action("appearance.locale");
                }
                if self.button(ui, "layout.reset", Icon::CornersOut, Text::Reset, true) {
                    self.action("layout.reset");
                }
                if self.button(ui, "models.open", Icon::File, Text::Models, true) {
                    self.action("models.open");
                }
                if self.button(ui, "shortcuts.open", Icon::GearSix, Text::Shortcuts, true) {
                    self.action("shortcuts.open");
                }
            });
        });
    }
}

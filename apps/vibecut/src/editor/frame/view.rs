//! Composes the editor window.
use crate::editor::{
    Command, Editor, Icon, Id, Rect, Text, Vec2, actions, drag_panel, egui, error_detail, i18n,
    icons, theme,
};

impl Editor {
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        // Agents can change selection or the project between pointer events.
        let actions = self.properties.reconcile(
            self.session.project(),
            crate::editor::context::single(&self.selection),
        );
        crate::editor::properties::apply(&mut self.desk(), actions);
        self.drain_requests();
        // The eyedropper belongs to one selected clip's panel.
        if self.player.key_color_pick.as_ref().is_some_and(|pick| {
            crate::editor::context::single(&self.selection) != Some(pick.item)
                || self.properties.details()
        }) {
            self.player.key_color_pick = None;
        }
        if !self.session.transaction_active() {
            self.desk().select();
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
        self.frame.widgets.clear();
        self.frame.panels.clear();
        self.tick();
        // History and external commands can remove what the timeline is editing.
        let missing_fade = self.timeline.forget_missing(&self.session);
        let interrupted_fade = (!ui.input(|input| input.focused) || self.player.fullscreen)
            && self.timeline.cancel_fade_drag();
        if missing_fade || interrupted_fade {
            self.discard_gesture();
        }
        if !self.dialogs.export.is_open()
            && !self.subtitles.as_ref().is_some_and(|s| s.modal_open())
            && self.player.crop.is_none()
        {
            self.keyboard(ui.ctx());
        }
        if self.player.fullscreen != self.player.fullscreen_commanded {
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.player.fullscreen));
            self.player.fullscreen_commanded = self.player.fullscreen;
        }
        if self.player.fullscreen {
            // The timeline is not painted here, so the pointer necessarily left it.
            self.timeline.clear_hover();
            if let Err(error) = self.playback.set_hover(None) {
                self.desk().fail(error.to_string());
            }
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
            self.show_player(&mut child);
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
            self.frame.panels.insert(id, area);
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
                "media" => self.show_assets(&mut child),
                "preview" => self.show_player(&mut child),
                "inspector" => self.inspector(&mut child),
                _ => self.timeline_panel(&mut child),
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
        self.timeline_scenes(ui.ctx());
        self.media_delete_dialog(ui.ctx());
        self.import_overlay(ui.ctx());
        self.drop_overlay(ui.ctx(), hovering);
        self.toast(ui.ctx());
        self.show_export(ui);
        super::super::panels::player::crop::show(self, ui.ctx());
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
        self.show_models(ui);
        self.show_preferences(ui);
        self.shortcuts_dialog(ui.ctx());
        if self
            .properties
            .scrub_widget()
            .is_some_and(|id| !self.frame.widgets.contains_key(&id))
        {
            self.finish_scrub(false);
        }
        if self
            .properties
            .reorder_handle()
            .is_some_and(|id| !self.frame.widgets.contains_key(&id))
        {
            self.properties.cancel_reorder();
        }
        self.finish_effect_drag(ui.ctx());
        self.finish_transition_drag(ui.ctx());
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(
                if self.playback.playing() { 8 } else { 33 },
            ));
    }
    /// Draw the properties area and route what it asks for.
    fn inspector(&mut self, ui: &mut egui::Ui) {
        let mut properties = std::mem::take(&mut self.properties);
        let mut cx = self.context();
        if !self.text_animation_available(&properties) {
            cx.disabled.extend([
                "text.animation.fade_in",
                "text.animation.fade_out",
                "text.animation.fade_in_out",
            ]);
        }
        let actions = crate::editor::properties::show(&mut properties, &cx, ui);
        drop(cx);
        self.properties = properties;
        for widget in self.properties.take_widgets() {
            self.remember_with_icon(widget.id.clone(), widget.icon, &widget.response);
            if let (Some(tooltip), Some(state)) =
                (widget.tooltip, self.frame.widgets.get_mut(&widget.id))
            {
                state.tooltip = Some(tooltip);
            }
        }
        if !actions.is_empty() {
            ui.ctx().request_repaint();
        }
        crate::editor::properties::apply(&mut self.desk(), actions);
        self.drain_requests();
    }
    /// Paint the export dialog and route its requests.
    fn show_export(&mut self, ui: &mut egui::Ui) {
        let actions = {
            let mut state = std::mem::take(&mut self.dialogs);
            let cx = self.context();
            let actions = super::super::dialogs::show_export(&mut state, &cx, ui);
            drop(cx);
            self.dialogs = state;
            actions
        };
        super::super::dialogs::apply(&mut self.desk(), actions);
    }
    fn show_preferences(&mut self, ui: &mut egui::Ui) {
        let mut state = std::mem::take(&mut self.dialogs);
        let actions = super::super::dialogs::show_preferences(&mut state, &self.context(), ui);
        self.dialogs = state;
        super::super::dialogs::apply(&mut self.desk(), actions);
    }
    /// Paint the model manager and route its requests.
    fn show_models(&mut self, ui: &mut egui::Ui) {
        let actions = {
            let mut state = std::mem::take(&mut self.dialogs);
            let cx = self.context();
            let actions = super::super::dialogs::show_models(&mut state, &cx, ui);
            drop(cx);
            self.dialogs = state;
            actions
        };
        super::super::dialogs::apply(&mut self.desk(), actions);
    }
    /// Render the player through its read-only boundary, then route its requests.
    pub(in crate::editor) fn show_player(&mut self, ui: &mut egui::Ui) {
        use super::super::panels::player::{self, Action};
        self.player.failed = self.error.is_some() || self.playback.error().is_some();
        self.player.snapshot_failed = self.player.snapshot.error.is_some();
        let placeholder =
            player::State::new(self.preferences.preview_fit, self.preferences.preview_zoom);
        let mut state = std::mem::replace(&mut self.player, placeholder);
        let cx = self.context();
        let actions = player::show(&mut state, &cx, ui);
        drop(cx);
        self.player = state;
        for action in actions {
            match action {
                Action::Remember {
                    id,
                    icon,
                    tooltip,
                    response,
                } => {
                    self.remember_with_icon(&id, icon, &response);
                    if let Some(widget) = self.frame.widgets.get_mut(&id) {
                        widget.tooltip = tooltip;
                    }
                }
                Action::Command(command) => self.action(&command),
                Action::Select(item) => self.select_single(item),
                Action::ClearSelection => {
                    self.selection.clear();
                    self.selection_changed();
                }
                Action::TransformPreview { item, values } => {
                    self.preview_transform(item, &values);
                }
                Action::TransformEnd { commit } => self.end_preview_transform(commit),
                Action::TextBegin { item } => self.begin_text_edit(item),
                Action::TextPreview { item, content } => self.preview_text(item, &content),
                Action::TextEnd { commit } => self.end_preview_transform(commit),
                Action::MaskPointSelect { target, toggle } => {
                    self.select_mask_point(target, toggle)
                }
                Action::MaskPointClear => self.clear_mask_points(),
                Action::MaskPathPreview { item, mask, path } => {
                    self.preview_mask_path(item, &mask, &path)
                }
                Action::MaskPathEnd { commit } => self.end_preview_transform(commit),
                Action::MaskPointAdd {
                    item,
                    mask,
                    segment,
                    t,
                } => self.add_mask_point(item, &mask, segment, t),
                Action::CommitTimecode => self.commit_timecode(),
                Action::Retry => {
                    self.player.timecode_error = None;
                    self.player.snapshot.error = None;
                    self.action("preview.retry");
                }
                Action::Copy => {
                    if let Err(error) = self.copy_snapshot(ui.ctx()) {
                        self.desk().fail(error.to_string());
                    }
                }
                Action::Zoom(factor) => {
                    if let Err(error) = self.preview_zoom_by(factor) {
                        self.desk().fail(error.to_string());
                    }
                }
                Action::PickKeyColor(color) => {
                    if let Some(pick) = self.player.key_color_pick.take()
                        && let Err(error) =
                            self.pick_key_color(pick.item, pick.effect.as_deref(), color)
                    {
                        self.desk().fail(error.to_string());
                    }
                }
            }
        }
    }
    pub(super) fn save_status(&self) -> Text {
        if self.persistence_error.is_some() {
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
        }
    }
    pub(in crate::editor) fn header(&mut self, ui: &mut egui::Ui) {
        let mut frame = std::mem::take(&mut self.frame);
        let cx = self.context();
        let actions = super::show(&mut frame, &cx, ui);
        drop(cx);
        self.frame = frame;
        for (id, icon, response) in std::mem::take(&mut self.frame.painted) {
            self.remember_with_icon(id, icon, &response);
        }
        for action in actions {
            match action {
                super::Action::Activate(id) => self.action(id),
                super::Action::Rename(name) => {
                    let command = Command::RenameProject {
                        name: name.clone().into(),
                    };
                    if self.desk().apply_result(command, false).is_err() {
                        self.frame.rename = Some(name);
                    }
                }
            }
        }
    }
    /// Record one painted control for automation reads.
    pub(in crate::editor) fn remember(&mut self, id: impl Into<String>, response: &egui::Response) {
        self.frame.widgets.remember(id, response);
    }
    /// Record one painted control with the icon its tooltip shows.
    pub(in crate::editor) fn remember_with_icon(
        &mut self,
        id: impl Into<String>,
        icon: Option<&'static str>,
        response: &egui::Response,
    ) {
        self.frame.widgets.remember_with_icon(id, icon, response);
    }
    /// One icon-only control that records its action and shortcut in a tooltip.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::editor) fn icon_only(
        &mut self,
        ui: &mut egui::Ui,
        id: &str,
        icon: Icon,
        label: &str,
        shortcut: Option<&str>,
        selected: Option<bool>,
        enabled: bool,
    ) -> bool {
        let enabled = enabled && actions::action(id).is_none_or(|_| self.action_enabled(id));
        let locale = self.preferences.locale;
        let tooltip = shortcut.map_or_else(
            || label.to_owned(),
            |key| {
                i18n::format(
                    locale,
                    Text::ActionShortcut,
                    &[("action", label), ("shortcut", key)],
                )
            },
        );
        let is_selected = selected.unwrap_or(false);
        let response = ui
            .add_enabled_ui(enabled, |ui| {
                crate::kit::icon_button(ui, id, icon, &tooltip, is_selected)
            })
            .inner;
        response.widget_info(|| {
            if let Some(selected) = selected {
                egui::WidgetInfo::selected(egui::Role::CheckBox, enabled, selected, &tooltip)
            } else {
                egui::WidgetInfo::labeled(egui::Role::Button, enabled, &tooltip)
            }
        });
        self.remember_with_icon(id, Some(icon.name()), &response);
        if let Some(state) = self.frame.widgets.get_mut(id) {
            state.tooltip = Some(tooltip);
        }
        enabled && response.clicked()
    }
}

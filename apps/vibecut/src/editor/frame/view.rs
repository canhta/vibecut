//! Composes the editor window.
use crate::editor::context::SaveStatus;
use crate::editor::{
    Command, Editor, Id, Rect, Vec2, drag_panel, egui, error_detail, icons, theme,
};

impl Editor {
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        self.sync_gap_policy();
        // Agents can change selection or the project between pointer events.
        let actions = self.properties.reconcile(
            self.session.project(),
            crate::editor::context::single(self.session.selection()),
        );
        self.apply_properties(actions);
        self.drain_requests();
        // The eyedropper belongs to one selected clip's panel.
        if self.player.key_color_pick.as_ref().is_some_and(|pick| {
            crate::editor::context::single(self.session.selection()) != Some(pick.item)
                || self.properties.details()
        }) {
            self.player.key_color_pick = None;
        }
        // The brush belongs to one selected clip's panel.
        if self.player.matte_brush.is_some_and(|brush| {
            crate::editor::context::single(self.session.selection()) != Some(brush.item)
                || self.properties.details()
        }) {
            self.player.matte_brush = None;
            self.player.brush_trail = None;
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
            self.assets.import_notice = Some(error_detail(&error));
        }
        Self::theme(ui.ctx(), self.preferences.dark);
        crate::kit::install_editor_density(ui.ctx());
        ui.style_mut().interaction.selectable_labels = false;
        self.frame.widgets.clear();
        self.frame.panels.clear();
        self.tick();
        // History and external commands can remove what the timeline is editing.
        self.forget_missing_transition();
        let missing_fade = self.timeline.forget_missing(&self.session);
        let interrupted_fade = (!ui.input(|input| input.focused) || self.player.fullscreen)
            && self.timeline.cancel_fade_drag();
        if missing_fade || interrupted_fade {
            self.discard_gesture();
        }
        self.frame.text_focused = ui.ctx().text_edit_focused();
        if !self.dialogs.export.is_open()
            && !self
                .assets
                .captions
                .as_ref()
                .is_some_and(|s| s.modal_open())
            && !self.properties.narration_modal()
            && self.player.crop.is_none()
            && !self.agent.previewing()
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
                self.fail(error.to_string());
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
            self.show_toast(ui.ctx());
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
            agent,
        } = super::layout::Layout::new(rect, self.preferences.panels, self.agent.is_open());
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
            // A previewed agent change is read only: the areas that edit stay off.
            if self.agent.previewing() && matches!(id, "media" | "inspector" | "timeline") {
                child.disable();
            }
            match id {
                "header" => self.header(&mut child),
                "media" => self.show_assets(&mut child),
                "preview" => self.show_player(&mut child),
                "inspector" => self.inspector(&mut child),
                _ => self.timeline_panel(&mut child),
            }
        }
        self.show_agent(ui, agent, timeline, preview);
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
        self.show_media_delete_dialog(ui.ctx());
        self.show_project_delete_dialog(ui.ctx());
        self.show_import_overlay(ui.ctx());
        self.show_drop_overlay(ui.ctx(), hovering);
        self.show_toast(ui.ctx());
        self.show_export(ui);
        self.with_player(|player, port| {
            crate::editor::panels::player::crop::show(player, port, ui.ctx());
        });
        if let Some(subtitles) = &mut self.assets.captions {
            let widgets = subtitles.dialogs(ui.ctx(), self.preferences.locale);
            for (id, response) in widgets {
                self.remember(id, &response);
            }
        }
        self.show_models(ui);
        self.show_voiceover(ui);
        self.show_scene_split(ui);
        self.show_link_media(ui);
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
        self.with_assets(|assets, port| assets.finish_effect_drag(port, ui.ctx()));
        self.assets.finish_media_drag(ui.ctx());
        self.with_assets(|assets, port| assets.finish_transition_drag(port, ui.ctx()));
        // Playback is the only frame loop this pass owns; a paused editor repaints on input and
        // on the jobs and animations that ask for it. The native loop paces worker polling.
        if self.playback.playing() {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(8));
        }
    }
    /// Draw the properties area and route what it asks for.
    fn inspector(&mut self, ui: &mut egui::Ui) {
        let mut properties = std::mem::take(&mut self.properties);
        let mut cx = self.context();
        if !self.clip_animation_available(&properties) {
            cx.disabled.extend(["animation.edit"]);
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
        self.apply_properties(actions);
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
        self.apply_dialogs(actions);
    }
    fn show_preferences(&mut self, ui: &mut egui::Ui) {
        let mut state = std::mem::take(&mut self.dialogs);
        let actions = super::super::dialogs::show_preferences(&mut state, &self.context(), ui);
        self.dialogs = state;
        self.apply_dialogs(actions);
    }
    /// Paint the voice-over recorder and route its requests.
    fn show_voiceover(&mut self, ui: &mut egui::Ui) {
        let actions = {
            let mut state = std::mem::take(&mut self.dialogs);
            let cx = self.context();
            let actions = super::super::dialogs::show_voiceover(&mut state, &cx, ui);
            drop(cx);
            self.dialogs = state;
            actions
        };
        self.apply_dialogs(actions);
    }
    /// Paint the Link media dialog and route its requests.
    fn show_link_media(&mut self, ui: &mut egui::Ui) {
        let actions = {
            let mut state = std::mem::take(&mut self.dialogs);
            let cx = self.context();
            let actions = super::super::dialogs::show_link_media(&mut state, &cx, ui);
            drop(cx);
            self.dialogs = state;
            actions
        };
        self.apply_dialogs(actions);
    }
    /// Paint the scene split review and route its requests.
    fn show_scene_split(&mut self, ui: &mut egui::Ui) {
        let actions = {
            let mut state = std::mem::take(&mut self.dialogs);
            let cx = self.context();
            let actions = super::super::dialogs::show_scene_split(&mut state, &cx, ui);
            drop(cx);
            self.dialogs = state;
            actions
        };
        self.apply_dialogs(actions);
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
        self.apply_dialogs(actions);
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
                    self.replace_selection(Default::default());
                }
                Action::TransformPreview { item, values } => {
                    self.preview_transform(item, &values);
                }
                Action::TransformEnd { commit } => self.end_preview_transform(commit),
                Action::MasterPreview { volume, muted } => {
                    self.preview_command(Command::TimelinePreview {
                        commands: vec![Command::SetMaster { volume, muted }],
                    });
                }
                Action::MasterEnd { commit } => self.end_preview_transform(commit),
                Action::MasterSet { volume, muted } => {
                    self.with_timeline(|_, port| {
                        crate::editor::timeline::command_on_selection(
                            port,
                            Command::SetMaster { volume, muted },
                        );
                    });
                }
                Action::TextBegin { item } => self.begin_text_edit(item),
                Action::TextPreview { item, content } => self.preview_text(item, &content),
                Action::TextEnd { commit } => self.end_preview_transform(commit),
                Action::MaskPointSelect { target, toggle } => {
                    self.select_mask_point(target, toggle)
                }
                Action::MaskPointClear => self.clear_mask_points(),
                Action::MaskPointMarquee {
                    item,
                    mask,
                    points,
                    toggle,
                } => self.select_mask_points(item, &mask, &points, toggle),
                Action::MaskPathPreview { item, mask, path } => {
                    self.preview_mask_path(item, &mask, &path)
                }
                Action::MaskBoxPreview { item, mask } => self.preview_mask_box(item, mask),
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
                Action::Zoom(factor) => {
                    if let Err(error) = self.preview_zoom_by(factor) {
                        self.fail(error.to_string());
                    }
                }
                Action::PickScreenColour(color) => self.properties.sample_colour(color),
                Action::PickKeyColor(color) => {
                    if let Some(pick) = self.player.key_color_pick.take()
                        && let Err(error) =
                            self.pick_key_color(pick.item, pick.effect.as_deref(), color)
                    {
                        self.fail(error.to_string());
                    }
                }
                Action::PaintMatteStroke { item, stroke } => {
                    if let Err(error) = self.paint_matte_stroke(item, stroke) {
                        self.fail(error.to_string());
                    }
                }
                Action::MoveGuide {
                    axis,
                    index,
                    position,
                } => {
                    if let Err(error) = self.dispatch(crate::editor::AppCommand::MoveGuide {
                        axis,
                        index,
                        position,
                    }) {
                        self.fail(error.to_string());
                    }
                }
                Action::SetPreference { key, value } => {
                    if let Err(error) = self.set_preference(key, value) {
                        self.fail(error.to_string());
                    }
                }
            }
        }
        if std::mem::take(&mut self.copy_snapshot_requested)
            && let Err(error) = self.copy_snapshot(ui.ctx())
        {
            self.fail(error.to_string());
        }
    }
    pub(super) fn save_status(&self) -> SaveStatus {
        if self.persistence_error.is_some() {
            SaveStatus::Failed
        } else {
            match &self.session {
                crate::services::lifecycle::EditorSession::Memory(_) => SaveStatus::Unsaved,
                crate::services::lifecycle::EditorSession::Persistent(autosave) => {
                    if autosave.saving() || autosave.journaled_revision() < autosave.revision() {
                        SaveStatus::Saving
                    } else {
                        SaveStatus::Saved {
                            clock: self.autosaved.as_ref().map(|(_, clock)| clock.clone()),
                        }
                    }
                }
            }
        }
    }
    /// Stamp the local time whenever the journal takes a new revision.
    fn note_autosave(&mut self) {
        let crate::services::lifecycle::EditorSession::Persistent(autosave) = &self.session else {
            return;
        };
        let journaled = autosave.journaled_revision();
        if self
            .autosaved
            .as_ref()
            .is_none_or(|(seen, _)| *seen != journaled)
        {
            let clock = chrono::Local::now().format("%H:%M:%S").to_string();
            self.autosaved = Some((journaled, clock));
        }
    }
    pub(in crate::editor) fn header(&mut self, ui: &mut egui::Ui) {
        self.note_autosave();
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
                    if self.apply_result(command).is_err() {
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
}

//! Renders the player surface.
use crate::editor::{
    Editor, Icon, Id, PREVIEW_FOOTER_HEIGHT, Rect, TIMECODE_WIDTH, Text, Tick, TimecodeEdit, Vec2,
    catalog, egui, preview, theme,
};

pub(in crate::editor) fn show(editor: &mut Editor, ui: &mut egui::Ui) {
    let duration = editor.session.project().duration();
    let empty = duration == Tick::ZERO;
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(editor.tr(Text::Preview))
                .size(16.0)
                .strong(),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if editor.icon_only(
                ui,
                "preview.fullscreen",
                Icon::CornersOut,
                editor.tr(if editor.preview_fullscreen {
                    Text::ExitFullscreen
                } else {
                    Text::Fullscreen
                }),
                None,
                Some(editor.preview_fullscreen),
                true,
            ) {
                editor.action("preview.fullscreen");
            }
            if editor.icon_only(
                ui,
                "preview.copy",
                Icon::File,
                editor.tr(Text::SnapshotCopy),
                None,
                None,
                !empty,
            ) && let Err(error) = editor.copy_snapshot(ui.ctx())
            {
                editor.error = Some(error.to_string());
            }
            if editor.icon_only(
                ui,
                "preview.snapshot",
                Icon::FloppyDisk,
                editor.tr(Text::SnapshotSave),
                None,
                None,
                !empty,
            ) {
                editor.action("preview.snapshot");
            }
        });
    });
    ui.add_space(theme::space::ROW_INSET);
    let mut footer_height = PREVIEW_FOOTER_HEIGHT;
    if !editor.playback.device_available() {
        footer_height += 24.0;
    }
    if editor.error.is_some() || editor.playback.error().is_some() {
        footer_height += 80.0;
    }
    let area = Rect::from_min_size(
        ui.cursor().min,
        Vec2::new(
            ui.available_width(),
            (ui.available_height() - footer_height).max(0.0),
        ),
    );
    editor.preview_viewport = area.size();
    editor.preview_canvas(ui, area, empty);
    ui.advance_cursor_after_rect(area);
    let ready = !empty && editor.playback.can_play();
    ui.horizontal(|ui| {
        if editor.icon_only(
            ui,
            "transport.back",
            Icon::SkipBack,
            editor.tr(Text::Back),
            editor.shortcut("transport.back").as_deref(),
            None,
            !empty,
        ) {
            editor.action("transport.back");
        }
        let (play_icon, play_text, play_action) = if editor.playback.playing() {
            (Icon::Pause, Text::Pause, "transport.pause")
        } else {
            (Icon::Play, Text::Play, "transport.play")
        };
        if editor.icon_only(
            ui,
            "transport.play",
            play_icon,
            editor.tr(play_text),
            editor.shortcut(play_action).as_deref(),
            None,
            ready,
        ) {
            editor.action("transport.play");
        }
        if editor.icon_only(
            ui,
            "transport.forward",
            Icon::SkipForward,
            editor.tr(Text::Forward),
            editor.shortcut("transport.forward").as_deref(),
            None,
            !empty,
        ) {
            editor.action("transport.forward");
        }
    });
    editor.preview_timecode_row(ui, empty, duration);
    editor.preview_zoom_row(ui, area.size());
    if let Some(code) = editor.snapshot.error {
        ui.label(
            egui::RichText::new(format!("{}: {code:?}", editor.tr(Text::SnapshotFailed)))
                .small()
                .color(theme::error_text(editor.preferences.dark)),
        );
    }
    if !editor.playback.device_available() {
        ui.label(editor.tr(Text::Offline));
    }
    if editor.error.is_some() || editor.playback.error().is_some() {
        ui.label(editor.tr(Text::Unavailable));
        if editor.icon_only(
            ui,
            "preview.retry",
            Icon::ArrowClockwise,
            editor.tr(Text::Retry),
            None,
            None,
            true,
        ) {
            editor.error = None;
            editor.action("preview.retry");
        }
    }
}

impl Editor {
    /// Editable HH:MM:SS:FF position and the project duration. Enter commits
    /// through `playback.seek`; Esc or invalid text restores the previous value.
    pub(in crate::editor) fn preview_timecode_row(
        &mut self,
        ui: &mut egui::Ui,
        empty: bool,
        duration: Tick,
    ) {
        let current = self.current_timecode();
        let total = self.format_timecode(duration);
        let mut editing = self.timecode_edit.take();
        let mut start = None;
        let mut commit = false;
        let mut cancel = false;
        ui.horizontal(|ui| {
            if let Some(edit) = editing.as_mut() {
                let response = ui.add_sized(
                    [TIMECODE_WIDTH, 26.0],
                    egui::TextEdit::singleline(&mut edit.text)
                        .id_salt("preview.timecode.field")
                        .font(egui::TextStyle::Monospace),
                );
                if edit.focus {
                    response.request_focus();
                    edit.focus = false;
                }
                self.remember("preview.timecode", &response);
                let escape = ui.input(|input| input.key_pressed(egui::Key::Escape));
                let enter = ui.input(|input| input.key_pressed(egui::Key::Enter));
                if escape {
                    cancel = true;
                } else if enter {
                    commit = true;
                } else if response.lost_focus() {
                    cancel = true;
                }
            } else {
                let response = ui.add_enabled(
                    !empty,
                    egui::Button::new(egui::RichText::new(&current).monospace())
                        .min_size(Vec2::new(TIMECODE_WIDTH, theme::size::FIELD)),
                );
                self.remember("preview.timecode", &response);
                if response.clicked() {
                    start = Some(current.clone());
                }
            }
            ui.label(
                egui::RichText::new(catalog::format(
                    self.preferences.locale,
                    Text::TimeTotal,
                    &[("duration", &total)],
                ))
                .monospace()
                .color(self.secondary()),
            );
            if let Some(error) = self.timecode_error {
                ui.label(
                    egui::RichText::new(self.tr(error))
                        .small()
                        .color(theme::error_text(self.preferences.dark)),
                );
            }
        });
        if let Some(text) = start {
            editing = Some(TimecodeEdit { text, focus: true });
            self.timecode_error = None;
        }
        if commit {
            self.timecode_edit = editing;
            self.commit_timecode();
        } else if cancel {
            self.timecode_edit = None;
        } else {
            self.timecode_edit = editing;
        }
    }
    /// Fit, the fixed presets, continuous ×1.25 steps and the current scale.
    pub(in crate::editor) fn preview_zoom_row(&mut self, ui: &mut egui::Ui, viewport: Vec2) {
        let canvas = self.preview_canvas_size();
        let mut action = None;
        ui.horizontal(|ui| {
            if self.preset_button(
                ui,
                "preview.zoom.fit",
                self.tr(Text::ZoomFit),
                self.preview_view.fit,
            ) {
                action = Some("preview.zoom.fit".to_owned());
            }
            for percent in preview::PRESETS {
                let target = *percent as f32 / 100.0;
                let selected =
                    !self.preview_view.fit && (self.preview_view.zoom - target).abs() < 1e-4;
                let id = format!("preview.zoom.{percent}");
                if self.preset_button(ui, &id, &format!("{percent}%"), selected) {
                    action = Some(id);
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if self.icon_only(
                    ui,
                    "preview.zoom.in",
                    Icon::MagnifyingGlassPlus,
                    self.tr(Text::ZoomIn),
                    None,
                    None,
                    true,
                ) {
                    action = Some("preview.zoom.in".to_owned());
                }
                if self.icon_only(
                    ui,
                    "preview.zoom.out",
                    Icon::MagnifyingGlassMinus,
                    self.tr(Text::ZoomOut),
                    None,
                    None,
                    true,
                ) {
                    action = Some("preview.zoom.out".to_owned());
                }
                let scale = self.preview_view.scale(viewport, canvas);
                ui.label(
                    egui::RichText::new(format!("{:.0}%", scale * 100.0))
                        .monospace()
                        .color(self.secondary()),
                );
            });
        });
        if let Some(action) = action {
            self.action(&action);
        }
    }
    pub(in crate::editor) fn preview_canvas(&mut self, ui: &mut egui::Ui, area: Rect, empty: bool) {
        ui.painter()
            .rect_filled(area, theme::radius::PANEL, theme::preview_backdrop());
        let response = ui.interact(
            area,
            Id::unique("preview.canvas"),
            egui::Sense::click_and_drag(),
        );
        self.remember("preview.canvas", &response);
        let canvas = self.preview_canvas_size();
        if response.hovered() && !empty {
            let (zoom, scroll) = ui
                .ctx()
                .input(|input| (input.zoom_delta(), input.smooth_scroll_delta()));
            if (zoom - 1.0).abs() > 1e-4 {
                if let Err(error) = self.preview_zoom_by(zoom) {
                    self.error = Some(error.to_string());
                }
            } else if scroll != Vec2::ZERO {
                self.preview_view.pan(scroll, area.size(), canvas);
                // Plain wheel/two-finger scroll never zooms and never falls
                // through to another owner while the preview can pan.
                if self.preview_view.can_pan() {
                    ui.ctx()
                        .input_mut(|input| input.smooth_scroll_delta = Vec2::ZERO);
                }
            }
        }
        // Middle-drag reads raw pointer state so it works even though the base
        // canvas also owns click/drag for selection-free presentation.
        let (middle_down, pointer_delta) = ui.ctx().input(|input| {
            (
                input.pointer.button_down(egui::PointerButton::Middle),
                input.pointer.delta(),
            )
        });
        if middle_down && (response.hovered() || response.dragged()) && pointer_delta != Vec2::ZERO
        {
            self.preview_view.pan(pointer_delta, area.size(), canvas);
        }
        let mut menu_action = None;
        response.context_menu(|ui| {
            if self.menu_button(ui, "preview.menu.snapshot", Text::SnapshotSave, !empty) {
                menu_action = Some("preview.snapshot");
                ui.close();
            }
            if self.menu_button(ui, "preview.menu.copy", Text::SnapshotCopy, !empty) {
                menu_action = Some("preview.menu.copy");
                ui.close();
            }
            if self.menu_button(ui, "preview.menu.fullscreen", Text::Fullscreen, true) {
                menu_action = Some("preview.fullscreen");
                ui.close();
            }
        });
        match menu_action {
            Some("preview.menu.copy") => {
                if let Err(error) = self.copy_snapshot(ui.ctx()) {
                    self.error = Some(error.to_string());
                }
            }
            Some(action) => self.action(action),
            None => {}
        }
        // The preview frame arrives asynchronously. The overlay must keep a
        // constant widget count whether it shows the label or the frame;
        // otherwise every control below the canvas derives a different egui id
        // from `ui.next_auto_id()` and a press/release pair spanning the
        // transition is dropped by egui's click tracking.
        let overlay = if empty {
            Some(Text::ProjectEmpty)
        } else if self.playback.frame().is_none() {
            Some(if self.playback.error().is_some() {
                Text::Unavailable
            } else {
                Text::Loading
            })
        } else {
            None
        };
        self.preview_overlay_label(ui, area, overlay);
        if empty {
            self.texture = None;
            self.texture_key = None;
            self.texture_owner = None;
            return;
        }
        if let Some(frame) = self.playback.frame() {
            let key = (frame.epoch, frame.at);
            if self.texture_key != Some(key) {
                if let Some(texture) = &mut self.texture {
                    texture.set(frame.image.clone(), egui::TextureOptions::LINEAR);
                } else {
                    self.texture = Some(ui.ctx().load_texture(
                        "preview.frame",
                        frame.image.clone(),
                        egui::TextureOptions::LINEAR,
                    ));
                }
                self.texture_key = Some(key);
                self.texture_owner = Some(frame.clone());
            }
            if let Some(texture) = &self.texture {
                let scale = self.preview_view.scale(area.size(), canvas);
                if self.preview_view.fit {
                    self.preview_view.offset = Vec2::ZERO;
                } else {
                    self.preview_view.offset =
                        preview::clamp_offset(self.preview_view.offset, area.size(), canvas, scale);
                }
                let size = Vec2::new(canvas[0] as f32 * scale, canvas[1] as f32 * scale);
                let rect = Rect::from_center_size(area.center() + self.preview_view.offset, size);
                let painter = ui.painter().with_clip_rect(area);
                painter.image(
                    texture.id(),
                    rect,
                    Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                    theme::image_tint(),
                );
            }
        } else {
            self.texture = None;
            self.texture_key = None;
            self.texture_owner = None;
        }
    }
    /// One stable scope for the canvas overlay. Its explicit id keeps the
    /// widget count below the canvas independent of the async frame state.
    pub(in crate::editor) fn preview_overlay_label(
        &mut self,
        ui: &mut egui::Ui,
        area: Rect,
        label: Option<Text>,
    ) {
        let text = label.map(|label| self.tr(label));
        ui.scope_builder(
            egui::UiBuilder::new()
                .scope_id(ui.make_persistent_id("preview.overlay"))
                .max_rect(area)
                .layout(egui::Layout::centered_and_justified(
                    egui::Direction::TopDown,
                )),
            |ui| {
                if let Some(text) = text {
                    ui.label(
                        egui::RichText::new(text)
                            .size(theme::type_size::CHROME)
                            .color(theme::on_scrim()),
                    );
                }
            },
        );
    }
}

//! Renders timeline tracks.

use crate::editor::timeline::{
    Align2, Arc, Editor, FontId, ICON_BUTTON, Icon, Id, ItemView, Rect, Text, TimelineLayout,
    TrackKind, TrackRow, Vec2, catalog, egui, icons, theme,
};

impl Editor {
    pub(in crate::editor::timeline) fn paint_tracks(
        &mut self,
        ui: &mut egui::Ui,
        layout: &TimelineLayout,
        rows: &[TrackRow],
        items: &[ItemView],
    ) {
        let mut header_ui = ui.new_child(
            egui::UiBuilder::new()
                .id_salt("track.headers")
                .max_rect(layout.labels),
        );
        header_ui.set_clip_rect(layout.labels);
        let body_painter = ui.painter().with_clip_rect(layout.body);
        let label_painter = ui.painter().with_clip_rect(layout.labels);
        label_painter.rect_filled(layout.labels, theme::radius::PANEL, self.panel_fill());
        body_painter.rect_filled(layout.body, theme::radius::PANEL, self.panel_fill());
        for (index, row) in rows.iter().enumerate() {
            let top = layout.body.top() + row.top - self.vertical_scroll;
            let label_rect = Rect::from_min_size(
                egui::pos2(layout.labels.left(), top),
                Vec2::new(layout.labels.width(), row.height),
            );
            let row_rect = Rect::from_min_size(
                egui::pos2(layout.body.left(), top),
                Vec2::new(layout.body.width(), row.height),
            );
            if !row_rect.intersects(layout.body) {
                continue;
            }
            let shade = if index % 2 == 0 {
                self.panel_fill()
            } else {
                self.control_fill()
            };
            label_painter.rect_filled(label_rect, theme::radius::PANEL, shade);
            body_painter.rect_filled(row_rect, theme::radius::PANEL, shade);
            let header = header_ui.interact(
                label_rect.intersect(layout.labels),
                Id::unique(("track.header", row.id.0)),
                egui::Sense::click_and_drag(),
            );
            header.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &row.name));
            let type_icon = track_icon(row.kind);
            self.remember_with_icon(
                format!("track.{}.header", row.id.0),
                Some(type_icon.name()),
                &header,
            );
            if row.locked {
                label_painter.rect_filled(
                    label_rect,
                    0.0,
                    theme::warning(self.preferences.dark).gamma_multiply(0.08),
                );
            }
            if self.track_drag == Some(row.id) {
                label_painter.rect_stroke(
                    label_rect,
                    0.0,
                    egui::Stroke::new(1.0, self.accent()),
                    egui::StrokeKind::Inside,
                );
            }
            let name_on_top = row.kind.can_hide() && row.kind.can_mute();
            let name_rect = Rect::from_min_max(
                egui::pos2(label_rect.left() + 4.0, label_rect.top() + 2.0),
                egui::pos2(
                    (label_rect.right() - if name_on_top { 4.0 } else { 58.0 })
                        .max(label_rect.left() + 4.0),
                    if name_on_top {
                        label_rect.top() + 20.0
                    } else {
                        label_rect.bottom() - 2.0
                    },
                ),
            );
            let renaming = self
                .track_rename
                .as_ref()
                .is_some_and(|(track, _)| *track == row.id);
            if renaming {
                let mut name = self
                    .track_rename
                    .take()
                    .map_or_else(String::new, |(_, name)| name);
                let response = header_ui
                    .scope_builder(
                        egui::UiBuilder::new().id_salt(("track.rename.editor", row.id.0)),
                        |ui| {
                            ui.spacing_mut().interact_size.y = name_rect.height();
                            ui.put(
                                name_rect,
                                egui::TextEdit::singleline(&mut name)
                                    .id_salt(("track.rename.input", row.id.0))
                                    .font(FontId::proportional(theme::type_size::BODY))
                                    .margin(Vec2::splat(1.0))
                                    .desired_width(name_rect.width()),
                            )
                        },
                    )
                    .inner;
                self.remember(format!("track.{}.rename", row.id.0), &response);
                if self.track_rename_focus {
                    response.request_focus();
                    self.track_rename_focus = false;
                }
                let cancel = header_ui.input(|input| input.key_pressed(egui::Key::Escape));
                let commit = header_ui.input(|input| input.key_pressed(egui::Key::Enter));
                if commit && !cancel {
                    self.track_command(
                        "track.rename",
                        serde_json::json!({"track":row.id,"name":name}),
                    );
                } else if !cancel {
                    self.track_rename = Some((row.id, name));
                }
            } else {
                label_painter.with_clip_rect(name_rect).text(
                    egui::pos2(name_rect.left(), name_rect.center().y),
                    Align2::LEFT_CENTER,
                    &row.name,
                    FontId::proportional(if row.height >= 40.0 { 12.0 } else { 10.0 }),
                    if row.locked {
                        theme::warning(self.preferences.dark)
                    } else {
                        self.text_color()
                    },
                );
            }
            let icon_rect = Rect::from_center_size(
                egui::pos2(label_rect.right() - 14.0, label_rect.center().y),
                Vec2::splat(theme::icon::DENSE),
            );
            icons::tinted(type_icon, 16.0, self.secondary()).paint_at(&header_ui, icon_rect);
            let mut x = label_rect.right() - 54.0;
            if row.kind.can_hide() {
                if self.track_toggle(
                    &mut header_ui,
                    layout.labels,
                    row,
                    x,
                    "hidden",
                    Text::HideTrack,
                    row.hidden,
                    if row.hidden {
                        Icon::EyeSlash
                    } else {
                        Icon::Eye
                    },
                ) {
                    self.track_command(
                        "track.set_hidden",
                        serde_json::json!({"track":row.id,"hidden":!row.hidden}),
                    );
                }
                x -= 28.0;
            }
            if row.kind.can_mute()
                && self.track_toggle(
                    &mut header_ui,
                    layout.labels,
                    row,
                    x,
                    "muted",
                    Text::MuteTrack,
                    row.muted,
                    if row.muted {
                        Icon::SpeakerSlash
                    } else {
                        Icon::SpeakerHigh
                    },
                )
            {
                self.track_command(
                    "track.set_muted",
                    serde_json::json!({"track":row.id,"muted":!row.muted}),
                );
            }
            if row.main {
                let enabled = self.session.project().data().scene().timeline.magnet;
                // A separate control fits below the main header's output toggles.
                let mut magnet_row = row.clone();
                magnet_row.top += 18.0;
                if self.track_toggle(
                    &mut header_ui,
                    layout.labels,
                    &magnet_row,
                    label_rect.left() + 4.0,
                    "magnet",
                    Text::Magnet,
                    enabled,
                    Icon::Magnet,
                ) {
                    self.track_command("track.set_magnet", serde_json::json!({"enabled":!enabled}));
                }
            }
            header.context_menu(|ui| {
                let rename = ui.add(egui::Button::image_and_text(
                    icons::tinted(Icon::PencilSimple, 16.0, self.text_color()),
                    self.tr(Text::RenameTrack),
                ));
                self.remember_with_icon(
                    format!("track.{}.rename_action", row.id.0),
                    Some(Icon::PencilSimple.name()),
                    &rename,
                );
                if rename.clicked() {
                    self.track_rename = Some((row.id, row.name.clone()));
                    self.track_rename_focus = true;
                    ui.close();
                }
                let lock_icon = if row.locked {
                    Icon::LockOpen
                } else {
                    Icon::Lock
                };
                let lock_text = if row.locked {
                    self.tr(Text::UnlockTrack)
                } else {
                    self.tr(Text::LockTrack)
                };
                let lock = ui.add(egui::Button::image_and_text(
                    icons::tinted(lock_icon, 16.0, self.text_color()),
                    lock_text,
                ));
                self.remember_with_icon(
                    format!("track.{}.lock", row.id.0),
                    Some(lock_icon.name()),
                    &lock,
                );
                if lock.clicked() {
                    self.track_command(
                        "track.set_locked",
                        serde_json::json!({"track":row.id,"locked":!row.locked}),
                    );
                    ui.close();
                }
                if !row.main {
                    let response = ui.add_enabled(
                        !row.locked,
                        egui::Button::image_and_text(
                            icons::tinted(Icon::Trash, 16.0, self.text_color()),
                            self.tr(Text::DeleteTrack),
                        ),
                    );
                    let response = response.on_disabled_hover_text(self.tr(Text::TrackLocked));
                    self.remember_with_icon(
                        format!("track.{}.delete", row.id.0),
                        Some(Icon::Trash.name()),
                        &response,
                    );
                    if response.clicked() {
                        self.track_command("track.remove", serde_json::json!({"track":row.id}));
                        ui.close();
                    }
                }
            });
            if header.drag_started() {
                self.track_drag = Some(row.id);
            }
            if header.drag_stopped() && self.track_drag == Some(row.id) {
                self.track_drag = None;
                let drop = ui.input(|input| input.pointer.interact_pos());
                if !row.main
                    && let Some(drop) = drop
                    && let Some(target) = rows.iter().find(|candidate| {
                        !candidate.main
                            && candidate.audio == row.audio
                            && drop.y >= layout.body.top() + candidate.top - self.vertical_scroll
                            && drop.y
                                <= layout.body.top() + candidate.top - self.vertical_scroll
                                    + candidate.height
                    })
                    && target.id != row.id
                {
                    self.track_command(
                        "track.reorder",
                        serde_json::json!({"track":row.id,"index":target.display_index}),
                    );
                }
            }
            if row.empty {
                let r = row_rect.shrink(3.0);
                let points = [
                    r.left_top(),
                    r.right_top(),
                    r.right_bottom(),
                    r.left_bottom(),
                    r.left_top(),
                ];
                body_painter.extend(egui::Shape::dashed_line(
                    &points,
                    egui::Stroke::new(1.0, self.border()),
                    5.0,
                    4.0,
                ));
            }
        }
        let background = ui.interact(
            layout.body,
            Id::unique("timeline.marquee"),
            egui::Sense::click_and_drag(),
        );
        self.remember("timeline.marquee", &background);
        let modifiers = ui.ctx().input(|input| input.modifiers);
        for item in items {
            self.paint_item(ui, layout, item, modifiers);
        }
        let item_rects: Vec<_> = items
            .iter()
            .map(|item| {
                (
                    item.id,
                    Rect::from_min_max(
                        egui::pos2(self.x_at(item.start, layout.body.left()), item.top + 2.0),
                        egui::pos2(
                            self.x_at(item.end, layout.body.left()),
                            item.top + item.height - 2.0,
                        ),
                    )
                    .intersect(layout.body),
                )
            })
            .filter(|(_, rect)| rect.is_positive())
            .collect();
        self.effect_drop_targets = item_rects.clone();
        if self.effect_drag.is_some() {
            let accent = self.accent();
            for (id, rect) in &item_rects {
                if self.effect_refusal(*id).is_none() {
                    body_painter.rect_stroke(
                        *rect,
                        2.0,
                        egui::Stroke::new(2.0, accent),
                        egui::StrokeKind::Inside,
                    );
                }
            }
        }
        if background.drag_started()
            && self.drag.is_none()
            && let Some(origin) = ui.input(|i| i.pointer.press_origin())
            && !item_rects.iter().any(|(_, r)| r.contains(origin))
        {
            self.marquee = Some((
                origin,
                self.selection.clone(),
                modifiers.command || modifiers.ctrl || modifiers.shift,
            ));
        }
        if let Some((origin, before, additive)) = &self.marquee
            && let Some(pointer) = ui.input(|i| i.pointer.interact_pos())
        {
            let rect = Rect::from_two_pos(*origin, pointer).intersect(layout.body);
            let mut selected =
                crate::selection::intersections(rect.min, rect.max, item_rects.iter().copied());
            if *additive {
                selected.extend(before);
            }
            self.selection = selected;
            ui.painter().with_clip_rect(layout.body).rect_filled(
                rect,
                0.0,
                self.accent().gamma_multiply(0.15),
            );
            ui.painter().with_clip_rect(layout.body).rect_stroke(
                rect,
                0.0,
                egui::Stroke::new(1.0, self.accent()),
                egui::StrokeKind::Inside,
            );
        }
        if background.drag_stopped() && self.marquee.take().is_some() {
            self.selection_changed();
        }
        if background.clicked() {
            self.selection.clear();
            self.selection_changed();
        }
        if let Some(at) = self.snap_indicator {
            let x = self.x_at(at, layout.body.left());
            if x >= layout.body.left() && x <= layout.body.right() {
                ui.painter().with_clip_rect(layout.body).line_segment(
                    [
                        egui::pos2(x, layout.body.top()),
                        egui::pos2(x, layout.body.bottom()),
                    ],
                    egui::Stroke::new(1.0, self.accent()),
                );
            }
        }
    }
    /// Add a secondary track compatible with the active section: audio beside an
    /// audio track, otherwise the active overlay kind (Video when nothing is
    /// selected). Above the active track, or the overlay section end otherwise.
    pub(in crate::editor) fn add_track_params(&self) -> serde_json::Value {
        let locale = self.preferences.locale;
        let (kind, placement, count) = {
            let data = self.session.project().data();
            let scene = data.scene();
            let active = self.selection.iter().find_map(|id| {
                scene
                    .timeline
                    .iter()
                    .find(|track| track.items.iter().any(|item| item.id == *id))
            });
            let kind = active.map_or(TrackKind::Video, |track| match track.kind {
                TrackKind::Audio => TrackKind::Audio,
                TrackKind::Text => TrackKind::Text,
                _ => TrackKind::Video,
            });
            let placement = match active {
                Some(track) if track.id != scene.timeline.main.id => {
                    serde_json::json!({"mode":"above","track":track.id})
                }
                _ => serde_json::json!({"mode":"above","track":scene.timeline.main.id}),
            };
            let count = scene
                .timeline
                .iter()
                .filter(|track| track.kind == kind)
                .count()
                + 1;
            (kind, placement, count)
        };
        let number = catalog::number(locale, count as u64);
        let name = match kind {
            TrackKind::Audio => catalog::format(locale, Text::AudioTrackName, &[("n", &number)]),
            TrackKind::Text => self.tr(Text::TextItem).to_owned(),
            _ => catalog::format(locale, Text::VideoTrackName, &[("n", &number)]),
        };
        serde_json::json!({"name":name,"kind":kind,"placement":placement})
    }
    pub(in crate::editor) fn track_command(&mut self, command: &str, params: serde_json::Value) {
        let before = self.session.project().clone();
        match self
            .session
            .set_selection(self.selection.clone())
            .and_then(|_| self.session.execute(command, params))
        {
            Ok(_) => {
                self.prune_selection();
                if !Arc::ptr_eq(&before, self.session.project()) {
                    self.sync_preview();
                }
            }
            Err(error) => self.error = Some(error.to_string()),
        }
    }
    #[allow(clippy::too_many_arguments)]
    pub(in crate::editor::timeline) fn track_toggle(
        &mut self,
        ui: &mut egui::Ui,
        clip: Rect,
        row: &TrackRow,
        x: f32,
        suffix: &str,
        label: Text,
        selected: bool,
        icon: Icon,
    ) -> bool {
        let rect = Rect::from_min_size(
            egui::pos2(
                x,
                clip.top() + row.top - self.vertical_scroll + (row.height - 24.0) / 2.0,
            ),
            Vec2::splat(ICON_BUTTON),
        )
        .intersect(clip);
        if !rect.is_positive() {
            return false;
        }
        let response = ui.interact(
            rect,
            Id::unique(("track", row.id.0, suffix)),
            egui::Sense::click(),
        );
        let tooltip = catalog::format(
            self.preferences.locale,
            Text::TrackToggle,
            &[("action", self.tr(label)), ("track", &row.name)],
        );
        response.widget_info(|| {
            egui::WidgetInfo::selected(egui::Role::CheckBox, true, selected, tooltip.clone())
        });
        self.remember_with_icon(
            format!("track.{}.{suffix}", row.id.0),
            Some(icon.name()),
            &response,
        );
        if let Some(state) = self
            .widgets
            .get_mut(&format!("track.{}.{suffix}", row.id.0))
        {
            state.tooltip = Some(tooltip.clone());
        }
        let color = if selected {
            self.accent()
        } else {
            self.secondary()
        };
        if response.hovered() || response.has_focus() {
            ui.painter().with_clip_rect(clip).rect_filled(
                rect,
                theme::radius::CONTROL,
                self.control_fill(),
            );
        }
        icons::tinted(icon, 16.0, color).paint_at(
            ui,
            Rect::from_center_size(rect.center(), Vec2::splat(theme::icon::DENSE)),
        );
        response.clone().on_hover_text(tooltip);
        response.clicked()
    }
}

pub(in crate::editor::timeline) fn track_height(kind: TrackKind) -> f32 {
    match kind {
        TrackKind::Video => 65.0,
        TrackKind::Audio => 50.0,
        TrackKind::Text
        | TrackKind::Sticker
        | TrackKind::Effect
        | TrackKind::Filter
        | TrackKind::Adjustment => 25.0,
    }
}
pub(in crate::editor::timeline) fn track_icon(kind: TrackKind) -> Icon {
    match kind {
        TrackKind::Video => Icon::FilmStrip,
        TrackKind::Audio => Icon::MusicNotes,
        TrackKind::Text => Icon::TextT,
        TrackKind::Sticker => Icon::Sticker,
        TrackKind::Effect => Icon::Sparkle,
        TrackKind::Filter => Icon::Funnel,
        TrackKind::Adjustment => Icon::SlidersHorizontal,
    }
}

//! Renders timeline tracks.

use crate::editor::timeline::{
    Action, BTreeSet, Context, FontId, ICON_BUTTON, Icon, Id, ItemId, ItemView, Locale, Rect,
    State, Text, TimelineLayout, TrackKind, TrackRow, Vec2, catalog, egui, icons, theme,
    view::{Ink, tr},
};

impl State {
    pub(in crate::editor::timeline) fn paint_tracks(
        &mut self,
        cx: &Context<'_>,
        ui: &mut egui::Ui,
        layout: &TimelineLayout,
        rows: &[TrackRow],
        items: &[ItemView],
    ) {
        let ink = Ink::of(cx);
        let mut header_ui = ui.new_child(
            egui::UiBuilder::new()
                .id_salt("track.headers")
                .max_rect(layout.labels),
        );
        header_ui.set_clip_rect(layout.labels);
        let body_painter = ui.painter().with_clip_rect(layout.body);
        let label_painter = ui.painter().with_clip_rect(layout.labels);
        label_painter.rect_filled(layout.labels, theme::radius::PANEL, ink.panel());
        body_painter.rect_filled(layout.body, theme::radius::PANEL, ink.panel());
        let shadow = Rect::from_min_max(
            egui::pos2(layout.body.left(), layout.body.top()),
            egui::pos2(layout.body.left() + SHADOW, layout.body.bottom()),
        );
        body_painter.add(egui::Shape::mesh(shadow_mesh(
            shadow,
            theme::window(ink.dark()),
        )));
        for row in rows {
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
            label_painter.rect_filled(label_rect, theme::radius::PANEL, ink.field());
            let header = header_ui.interact(
                label_rect.intersect(layout.labels),
                Id::unique(("track.header", row.id.0)),
                egui::Sense::click_and_drag(),
            );
            header.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &row.name));
            let type_icon = track_icon(row.kind);
            self.remember_icon(
                format!("track.{}.header", row.id.0),
                Some(type_icon.name()),
                &header,
            );
            if row.locked {
                label_painter.rect_filled(
                    label_rect,
                    0.0,
                    theme::warning(ink.dark()).gamma_multiply(0.08),
                );
            }
            if self.track_drag == Some(row.id) {
                label_painter.rect_stroke(
                    label_rect,
                    0.0,
                    egui::Stroke::new(1.0, ink.accent()),
                    egui::StrokeKind::Inside,
                );
            }
            let renaming = self
                .track_rename
                .as_ref()
                .is_some_and(|(track, _)| *track == row.id);
            if renaming {
                let name_rect = label_rect.shrink2(Vec2::new(4.0, 6.0));
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
                    self.push(Action::Command {
                        id: "track.rename",
                        params: serde_json::json!({"track":row.id,"name":name}),
                    });
                } else if !cancel {
                    self.track_rename = Some((row.id, name));
                }
            } else {
                // Five 24 px cells: kind glyph, lock, eye, mute, more. A track
                // that lacks a control leaves its cell empty so columns line up.
                let cell = |slot: usize| {
                    egui::pos2(
                        label_rect.left() + 1.0 + theme::size::HEADER_CELL * (slot as f32 + 0.5),
                        label_rect.center().y,
                    )
                };
                let kind_rect = Rect::from_center_size(cell(0), Vec2::splat(theme::icon::DENSE));
                icons::tinted(type_icon, 16.0, theme::disabled(ink.dark()))
                    .paint_at(&header_ui, kind_rect);
                if self.track_cell(
                    cx,
                    &mut header_ui,
                    layout.labels,
                    row,
                    cell(1),
                    "lock",
                    if row.locked {
                        Text::UnlockTrack
                    } else {
                        Text::LockTrack
                    },
                    row.locked,
                    Icon::Lock,
                ) {
                    self.push(Action::Command {
                        id: "track.set_locked",
                        params: serde_json::json!({"track":row.id,"locked":!row.locked}),
                    });
                }
                if row.kind.can_hide()
                    && self.track_cell(
                        cx,
                        &mut header_ui,
                        layout.labels,
                        row,
                        cell(2),
                        "hidden",
                        Text::HideTrack,
                        row.hidden,
                        if row.hidden {
                            Icon::EyeSlash
                        } else {
                            Icon::Eye
                        },
                    )
                {
                    self.push(Action::Command {
                        id: "track.set_hidden",
                        params: serde_json::json!({"track":row.id,"hidden":!row.hidden}),
                    });
                }
                if row.kind.can_mute()
                    && self.track_cell(
                        cx,
                        &mut header_ui,
                        layout.labels,
                        row,
                        cell(3),
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
                    self.push(Action::Command {
                        id: "track.set_muted",
                        params: serde_json::json!({"track":row.id,"muted":!row.muted}),
                    });
                }
                let more_rect = Rect::from_center_size(cell(4), Vec2::splat(theme::size::CONTROL))
                    .intersect(layout.labels);
                let more = header_ui.interact(
                    more_rect,
                    Id::unique(("track", row.id.0, "more")),
                    egui::Sense::click(),
                );
                let hint = catalog::format(
                    cx.locale,
                    Text::TrackToggle,
                    &[("action", tr(cx, Text::TrackMore)), ("track", &row.name)],
                );
                more.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &hint));
                self.remember_icon(
                    format!("track.{}.more", row.id.0),
                    Some(Icon::DotsThree.name()),
                    &more,
                );
                if more.hovered() || more.has_focus() {
                    label_painter.rect_filled(more_rect, theme::radius::CONTROL, ink.panel());
                }
                icons::tinted(Icon::DotsThree, 16.0, ink.text()).paint_at(
                    &header_ui,
                    Rect::from_center_size(more_rect.center(), Vec2::splat(theme::icon::DENSE)),
                );
                let more = more.on_hover_text(hint);
                egui::Popup::menu(&more).show(|ui| self.track_menu(cx, ui, row));
            }
            header.context_menu(|ui| self.track_menu(cx, ui, row));
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
                    self.push(Action::Command {
                        id: "track.reorder",
                        params: serde_json::json!({"track":row.id,"index":target.display_index}),
                    });
                }
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
            self.paint_item(cx, ui, layout, item, modifiers);
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
        self.item_rects.clone_from(&item_rects);
        if let Some(accepting) = &cx.effect_targets {
            let accent = ink.accent();
            for (id, rect) in &item_rects {
                if accepting.contains(id) {
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
                cx.selection.clone(),
                modifiers.command || modifiers.ctrl || modifiers.shift,
            ));
        }
        if let Some((origin, before, additive)) = &self.marquee
            && let Some(pointer) = ui.input(|i| i.pointer.interact_pos())
        {
            let rect = Rect::from_two_pos(*origin, pointer).intersect(layout.body);
            let mut selected: BTreeSet<ItemId> =
                crate::selection::intersections(rect.min, rect.max, item_rects.iter().copied());
            if *additive {
                selected.extend(before);
            }
            self.outbox.push(Action::Marquee(selected));
            ui.painter().with_clip_rect(layout.body).rect_filled(
                rect,
                0.0,
                ink.accent().gamma_multiply(0.15),
            );
            ui.painter().with_clip_rect(layout.body).rect_stroke(
                rect,
                0.0,
                egui::Stroke::new(1.0, ink.accent()),
                egui::StrokeKind::Inside,
            );
        }
        if background.drag_stopped() && self.marquee.take().is_some() {
            self.push(Action::SettleSelection);
        }
        if background.clicked() {
            self.push(Action::ClearSelection);
        }
        if let Some(at) = self.snap_indicator {
            let x = self.x_at(at, layout.body.left());
            if x >= layout.body.left() && x <= layout.body.right() {
                ui.painter().with_clip_rect(layout.body).line_segment(
                    [
                        egui::pos2(x, layout.body.top()),
                        egui::pos2(x, layout.body.bottom()),
                    ],
                    egui::Stroke::new(1.0, ink.accent()),
                );
            }
        }
    }
    /// The context menu shared by a header's right click and its more button.
    fn track_menu(&mut self, cx: &Context<'_>, ui: &mut egui::Ui, row: &TrackRow) {
        let ink = Ink::of(cx);
        let rename = ui.add(egui::Button::image_and_text(
            icons::tinted(Icon::PencilSimple, 16.0, ink.text()),
            tr(cx, Text::RenameTrack),
        ));
        self.remember_icon(
            format!("track.{}.rename_action", row.id.0),
            Some(Icon::PencilSimple.name()),
            &rename,
        );
        if rename.clicked() {
            self.track_rename = Some((row.id, row.name.clone()));
            self.track_rename_focus = true;
            ui.close();
        }
        if !row.main {
            let response = ui.add_enabled(
                !row.locked,
                egui::Button::image_and_text(
                    icons::tinted(Icon::Trash, 16.0, ink.text()),
                    tr(cx, Text::DeleteTrack),
                ),
            );
            let response = response.on_disabled_hover_text(tr(cx, Text::TrackLocked));
            self.remember_icon(
                format!("track.{}.delete", row.id.0),
                Some(Icon::Trash.name()),
                &response,
            );
            if response.clicked() {
                self.push(Action::Command {
                    id: "track.remove",
                    params: serde_json::json!({"track":row.id}),
                });
                ui.close();
            }
        }
    }
    /// A 24 px header toggle centred on `center`.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::editor::timeline) fn track_cell(
        &mut self,
        cx: &Context<'_>,
        ui: &mut egui::Ui,
        clip: Rect,
        row: &TrackRow,
        center: egui::Pos2,
        suffix: &str,
        label: Text,
        selected: bool,
        icon: Icon,
    ) -> bool {
        let ink = Ink::of(cx);
        let rect = Rect::from_center_size(center, Vec2::splat(ICON_BUTTON)).intersect(clip);
        if !rect.is_positive() {
            return false;
        }
        let response = ui.interact(
            rect,
            Id::unique(("track", row.id.0, suffix)),
            egui::Sense::click(),
        );
        let tooltip = catalog::format(
            cx.locale,
            Text::TrackToggle,
            &[("action", tr(cx, label)), ("track", &row.name)],
        );
        response.widget_info(|| {
            egui::WidgetInfo::selected(egui::Role::CheckBox, true, selected, tooltip.clone())
        });
        self.remember_with(
            format!("track.{}.{suffix}", row.id.0),
            Some(icon.name()),
            Some(tooltip.clone()),
            &response,
        );
        // Header controls read in the full text ink; a locked, hidden or muted
        // track dims its own control.
        let color = if suffix == "lock" {
            if selected {
                theme::warning(ink.dark())
            } else {
                ink.text()
            }
        } else if selected {
            ink.secondary()
        } else {
            ink.text()
        };
        if response.hovered() || response.has_focus() {
            ui.painter().with_clip_rect(clip).rect_filled(
                rect,
                theme::radius::CONTROL,
                ink.panel(),
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

/// Params for adding a secondary track compatible with the active section:
/// audio beside an audio track, otherwise the active overlay kind (Video when
/// nothing is selected). Above the active track, or the overlay section end otherwise.
pub(in crate::editor) fn add_track_params(
    session: &engine::Session,
    selection: &BTreeSet<ItemId>,
    locale: Locale,
) -> serde_json::Value {
    let (kind, placement, count) = {
        let data = session.project().data();
        let scene = data.scene();
        let active = selection.iter().find_map(|id| {
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
        TrackKind::Text => catalog::text(locale, Text::TextItem).to_owned(),
        _ => catalog::format(locale, Text::VideoTrackName, &[("n", &number)]),
    };
    serde_json::json!({"name":name,"kind":kind,"placement":placement})
}

/// Inner shadow at the left edge of the lanes (221643 x 132-137: dark to panel over 5 px).
const SHADOW: f32 = 5.0;
fn shadow_mesh(rect: Rect, dark: egui::Color32) -> egui::Mesh {
    let mut mesh = egui::Mesh::default();
    let clear = egui::Color32::TRANSPARENT;
    mesh.colored_vertex(rect.left_top(), dark);
    mesh.colored_vertex(rect.right_top(), clear);
    mesh.colored_vertex(rect.right_bottom(), clear);
    mesh.colored_vertex(rect.left_bottom(), dark);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    mesh
}

pub(in crate::editor::timeline) fn track_height(kind: TrackKind) -> f32 {
    match kind {
        TrackKind::Video => theme::size::VIDEO_TRACK,
        TrackKind::Audio => 40.0,
        TrackKind::Text
        | TrackKind::Sticker
        | TrackKind::Effect
        | TrackKind::Filter
        | TrackKind::Adjustment => 32.0,
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

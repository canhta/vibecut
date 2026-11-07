//! Renders timeline clips.

use crate::editor::timeline::{
    Action, Align2, Color32, Context, DragMode, FontId, Icon, Id, ItemParams, ItemView,
    LABEL_CHARS, Locale, Rect, State, Text, TimelineLayout, TrackKind, Vec2, audio, display_label,
    egui, icons, theme,
    view::{Ink, tr},
};
use crate::i18n;

impl State {
    pub(in crate::editor::timeline) fn paint_item(
        &mut self,
        cx: &Context<'_>,
        ui: &egui::Ui,
        layout: &TimelineLayout,
        item: &ItemView,
        modifiers: egui::Modifiers,
    ) {
        let x0 = self.x_at(item.start, layout.body.left());
        let x1 = self.x_at(item.end, layout.body.left());
        let full = Rect::from_min_max(
            egui::pos2(x0, item.top + CLIP_INSET),
            egui::pos2(x1, item.top + item.height - CLIP_INSET),
        );
        let visible = full.intersect(layout.body);
        if !visible.is_positive() {
            return;
        }
        let selected = cx.selection.contains(&item.id);
        let response = ui.interact(
            visible,
            Id::unique(("timeline.item", item.id.0)),
            if item.locked {
                egui::Sense::click()
            } else {
                egui::Sense::click_and_drag()
            },
        );
        self.remember(format!("timeline.item.{}", item.id.0), &response);
        let ink = Ink::of(cx);
        let dark = ink.dark();
        let (fill, border, text) = item_palette(item.kind, dark, selected, item.overlay);
        // Hidden clips stay editable in the timeline at reduced contrast.
        let dim = if item.hidden { 0.45 } else { 1.0 };
        let (fill, border, text) = (
            fill.gamma_multiply(dim),
            border.gamma_multiply(dim),
            text.gamma_multiply(dim),
        );
        let painter = ui.painter().with_clip_rect(visible);
        painter.rect_filled(visible, theme::radius::PANEL, fill);
        let inner = full.shrink(1.0);
        match item.kind {
            TrackKind::Video if item.overlay.is_none() => {
                // Top band, filmstrip, waveform: 17 + 39 + 16 px inside the border.
                let strip_top = inner.top() + theme::size::CLIP_BAND;
                let strip = Rect::from_min_size(
                    egui::pos2(inner.left(), strip_top),
                    Vec2::new(
                        inner.width(),
                        theme::size::CLIP_FILMSTRIP.min(inner.height()),
                    ),
                );
                self.paint_filmstrip(cx, ui, &painter, strip, item, dark);
                let wave = Rect::from_min_max(
                    egui::pos2(inner.left(), strip.bottom()),
                    inner.right_bottom(),
                );
                paint_waveform(&painter, wave, item, theme::waveform(dark), 0.9);
            }
            TrackKind::Audio => {
                paint_waveform(&painter, inner, item, theme::waveform(dark), 0.7);
            }
            _ => {}
        }
        painter.rect_stroke(
            full,
            theme::radius::PANEL,
            egui::Stroke::new(1.0, border),
            egui::StrokeKind::Inside,
        );
        if !item.label.is_empty() {
            painter.text(
                egui::pos2(visible.left() + 22.0, visible.center().y),
                Align2::LEFT_CENTER,
                &item.label,
                FontId::proportional(theme::type_size::BODY),
                text,
            );
        }
        self.paint_lane_toggle(cx, ui, item, visible, ink);
        if item.pinned {
            // The main track's first item cannot move before the origin: a
            // 3 px accent edge marks the pinned start.
            painter.rect_filled(
                Rect::from_min_max(
                    visible.left_top(),
                    egui::pos2(visible.left() + 3.0, visible.bottom()),
                ),
                0.0,
                ink.accent(),
            );
            let label = tr(cx, Text::PinnedToStart);
            let hit = Rect::from_min_max(
                visible.left_top(),
                egui::pos2(
                    visible.left() + 10.0,
                    visible.top() + 24.0_f32.min(visible.height()),
                ),
            );
            let response = ui.interact(
                hit,
                Id::unique(("timeline.item.pinned", item.id.0)),
                egui::Sense::hover(),
            );
            response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, label));
            self.remember_with(
                format!("timeline.item.{}.pinned", item.id.0),
                None,
                Some(label.to_owned()),
                &response,
            );
            response.on_hover_text(label);
        }
        if item.kind == TrackKind::Audio {
            self.paint_volume_line(cx, ui, item, inner, ink, layout);
        }
        if !item.locked && visible.width() > 16.0 {
            let width = 8.0_f32.min(visible.width() / 3.0);
            let left = Rect::from_min_max(
                visible.left_top(),
                egui::pos2(visible.left() + width, visible.bottom()),
            );
            let right = Rect::from_min_max(
                egui::pos2(visible.right() - width, visible.top()),
                visible.right_bottom(),
            );
            for (id, rect, mode) in [
                ("trim_start", left, DragMode::TrimStart),
                ("trim_end", right, DragMode::TrimEnd),
            ] {
                let response = ui.interact(
                    rect,
                    Id::unique(("timeline.item.handle", item.id.0, id)),
                    egui::Sense::click_and_drag(),
                );
                self.remember(format!("timeline.item.{}.{id}", item.id.0), &response);
                if selected || response.hovered() {
                    ui.painter().with_clip_rect(rect).line_segment(
                        [rect.center_top(), rect.center_bottom()],
                        egui::Stroke::new(2.0, ink.accent()),
                    );
                }
                if response.clicked() {
                    self.push(Action::Select {
                        item: item.id,
                        modifiers,
                    });
                }
                if response.drag_started() && self.drag.is_none() {
                    self.begin_drag(cx, item.id, mode, ui);
                }
            }
        }
        if response.clicked() {
            self.push(Action::Select {
                item: item.id,
                modifiers,
            });
        }
        if !item.locked && self.drag.is_none() && response.drag_started() {
            let press = ui.input(|input| input.pointer.press_origin());
            if item.kind == TrackKind::Audio
                && press
                    .is_some_and(|origin| self.near_volume_line(cx, item, inner, origin, layout))
            {
                self.begin_volume_drag(cx, ui, layout, item, inner);
            } else {
                self.begin_drag(cx, item.id, DragMode::Move, ui);
            }
        }
        if response.secondary_clicked() && !selected {
            self.push(Action::Select {
                item: item.id,
                modifiers: egui::Modifiers::NONE,
            });
        }
        response.context_menu(|ui| self.item_menu(cx, ui, item));
    }

    /// The clip context menu: split variants, duplicate, delete, hide/mute and
    /// source-audio extraction, each dispatching its registered action id.
    fn item_menu(&mut self, cx: &Context<'_>, ui: &mut egui::Ui, item: &ItemView) {
        let editable = !item.locked;
        let at = cx.playhead;
        let splittable = editable && item.start < at && at < item.end;
        let split = format!("timeline.item.{}.split", item.id.0);
        if self.menu_action(cx, ui, split, Text::Split, Icon::Scissors, splittable) {
            self.push(Action::Activate("timeline.split".to_owned()));
            ui.close();
        }
        let split_left = format!("timeline.item.{}.split_left", item.id.0);
        if self.menu_action(
            cx,
            ui,
            split_left,
            Text::SplitLeft,
            Icon::ArrowLeft,
            splittable,
        ) {
            self.push(Action::Activate("timeline.split_left".to_owned()));
            ui.close();
        }
        let split_right = format!("timeline.item.{}.split_right", item.id.0);
        if self.menu_action(
            cx,
            ui,
            split_right,
            Text::SplitRight,
            Icon::ArrowsLeftRight,
            splittable,
        ) {
            self.push(Action::Activate("timeline.split_right".to_owned()));
            ui.close();
        }
        let duplicate = format!("timeline.item.{}.duplicate", item.id.0);
        if self.menu_action(cx, ui, duplicate, Text::Duplicate, Icon::Copy, editable) {
            self.push(Action::Activate("timeline.duplicate".to_owned()));
            ui.close();
        }
        let delete = format!("timeline.item.{}.delete", item.id.0);
        if self.menu_action(cx, ui, delete, Text::Delete, Icon::Trash, editable) {
            self.push(Action::Activate("timeline.delete".to_owned()));
            ui.close();
        }
        if item.kind != TrackKind::Audio {
            let (label, icon) = if item.hidden {
                (Text::ShowItem, Icon::Eye)
            } else {
                (Text::HideItem, Icon::EyeSlash)
            };
            let hide = format!("timeline.item.{}.hide", item.id.0);
            if self.menu_action(cx, ui, hide, label, icon, editable) {
                self.push(Action::Activate("timeline.set_hidden".to_owned()));
                ui.close();
            }
        }
        if matches!(item.kind, TrackKind::Video | TrackKind::Audio) {
            let muted = item.muted;
            let (label, icon) = if muted {
                (Text::UnmuteItem, Icon::SpeakerHigh)
            } else {
                (Text::MuteItem, Icon::SpeakerSlash)
            };
            let mute = format!("timeline.item.{}.mute", item.id.0);
            if self.menu_action(cx, ui, mute, label, icon, editable) {
                self.push(Action::Activate("timeline.set_muted".to_owned()));
                ui.close();
            }
        }
        if item.kind == TrackKind::Video {
            let separated = item.source_audio_enabled == Some(false);
            let (label, action) = if separated {
                (Text::RecoverAudio, "timeline.recover_audio")
            } else {
                (Text::ExtractAudio, "timeline.extract_audio")
            };
            let id = format!("timeline.item.{}.audio", item.id.0);
            if self.menu_action(cx, ui, id, label, Icon::MusicNotes, editable) {
                self.push(Action::Activate(action.to_owned()));
                ui.close();
            }
        }
        if item.source_asset.is_some() {
            let reveal = format!("timeline.item.{}.reveal", item.id.0);
            if self.menu_action(cx, ui, reveal, Text::RevealMedia, Icon::FolderOpen, true) {
                if let Some(asset) = item.source_asset {
                    self.push(Action::Activate(format!("media.reveal.{}", asset.0)));
                }
                ui.close();
            }
        }
    }

    fn menu_action(
        &mut self,
        cx: &Context<'_>,
        ui: &mut egui::Ui,
        id: String,
        label: Text,
        icon: Icon,
        enabled: bool,
    ) -> bool {
        let ink = Ink::of(cx);
        let response = ui.add_enabled(
            enabled,
            egui::Button::image_and_text(icons::tinted(icon, 16.0, ink.text()), tr(cx, label)),
        );
        self.remember_icon(id, Some(icon.name()), &response);
        enabled && response.clicked()
    }
}

/// Gap between a lane's edge and its clip: a 76 px video lane holds the 74 px clip.
const CLIP_INSET: f32 = 1.0;

impl State {
    /// A 16 px chevron on the clip toggles its keyframe lanes.
    fn paint_lane_toggle(
        &mut self,
        cx: &Context<'_>,
        ui: &egui::Ui,
        item: &ItemView,
        visible: Rect,
        ink: Ink,
    ) {
        if visible.width() < 28.0 || visible.height() < 24.0 {
            return;
        }
        let rect = Rect::from_min_size(
            egui::pos2(visible.left() + 3.0, visible.top() + 3.0),
            Vec2::splat(16.0),
        );
        let response = ui.interact(
            rect,
            Id::unique(("timeline.item.lanes", item.id.0)),
            egui::Sense::click(),
        );
        let open = self.lane_open(item.id);
        let label = tr(
            cx,
            if open {
                Text::HideKeyframes
            } else {
                Text::ShowKeyframes
            },
        );
        self.remember_icon(
            format!("timeline.item.{}.lanes", item.id.0),
            Some(if open {
                Icon::CaretDown.name()
            } else {
                Icon::CaretRight.name()
            }),
            &response,
        );
        response
            .widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, true, open, label));
        let response = response.on_hover_text(label);
        if response.hovered() || response.has_focus() {
            ui.painter().with_clip_rect(visible).rect_filled(
                rect,
                theme::radius::CONTROL,
                ink.panel(),
            );
        }
        icons::tinted(
            if open {
                Icon::CaretDown
            } else {
                Icon::CaretRight
            },
            12.0,
            ink.text(),
        )
        .paint_at(ui, Rect::from_center_size(rect.center(), Vec2::splat(12.0)));
        if response.clicked() {
            self.push(Action::SetLaneOpen {
                item: item.id,
                open: !open,
            });
        }
    }

    fn paint_volume_line(
        &mut self,
        cx: &Context<'_>,
        ui: &egui::Ui,
        item: &ItemView,
        inner: Rect,
        ink: Ink,
        layout: &TimelineLayout,
    ) {
        let Some(base) = item.volume else {
            return;
        };
        let painter = ui.painter().with_clip_rect(inner);
        let channel = volume_channel(cx, item.id);
        let visible = inner.intersect(layout.body);
        if !visible.is_positive() {
            return;
        }
        let mut line_top = f32::INFINITY;
        let mut line_bottom = f32::NEG_INFINITY;
        let line = theme::accent(ink.dark()).gamma_multiply(0.9);
        let stroke = egui::Stroke::new(1.5, line);
        let step = 8.0;
        let first = ((visible.left() - inner.left()) / step).floor() * step;
        let mut previous: Option<egui::Pos2> = None;
        let mut x = first;
        let mut guard = 0;
        while x <= visible.right() - inner.left() + step && guard < 4096 {
            let px = inner.left() + x;
            let at = self.tick_at(px, layout.body.left());
            let db = channel.map_or(base, |channel| {
                item_db_at(channel, at.saturating_sub(item.start), base)
            });
            let point = egui::pos2(px, inner.top() + audio::line_fraction(db) * inner.height());
            line_top = line_top.min(point.y);
            line_bottom = line_bottom.max(point.y);
            if let Some(previous) = previous {
                painter.line_segment([previous, point], stroke);
            }
            previous = Some(point);
            x += step;
            guard += 1;
        }
        if let Some(channel) = channel {
            for key in channel.keys() {
                let Some(at) = item.start.checked_add(key.time) else {
                    continue;
                };
                let px = self.x_at(at, layout.body.left());
                if px < visible.left() - 3.0 || px > visible.right() + 3.0 {
                    continue;
                }
                let db = match &key.value {
                    project::ParamValue::Scalar(value) => *value as f32,
                    _ => base,
                };
                let y = inner.top() + audio::line_fraction(db) * inner.height();
                painter.circle_filled(egui::pos2(px, y), 2.5, ink.panel());
                painter.circle_stroke(egui::pos2(px, y), 2.5, stroke);
            }
        }
        // A 12 px band around the line is the drag surface; the rest of the clip
        // still moves, and a plain click falls through to selection.
        let band = Rect::from_min_max(
            egui::pos2(visible.left(), (line_top - 6.0).max(visible.top())),
            egui::pos2(visible.right(), (line_bottom + 6.0).min(visible.bottom())),
        )
        .intersect(visible);
        let response = ui.interact(
            band,
            Id::unique(("timeline.volume", item.id.0)),
            egui::Sense::drag(),
        );
        self.remember(format!("timeline.volume.{}", item.id.0), &response);
        if response.drag_started() && self.volume_drag.is_none() {
            self.begin_volume_drag(cx, ui, layout, item, inner);
        }
        // Hover and drag show the exact dB under the pointer.
        let pointer = ui.ctx().input(|input| {
            input
                .pointer
                .interact_pos()
                .or_else(|| input.pointer.hover_pos())
        });
        let Some(pointer) = pointer else {
            return;
        };
        if !inner.x_range().contains(pointer.x) {
            return;
        }
        let at = self.tick_at(pointer.x, layout.body.left());
        let db = channel.map_or(base, |channel| {
            item_db_at(channel, at.saturating_sub(item.start), base)
        });
        let y = inner.top() + audio::line_fraction(db) * inner.height();
        let dragging = self.volume_drag.is_some_and(|drag| drag.item == item.id);
        if !dragging && (pointer.y - y).abs() > 6.0 {
            return;
        }
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
        let text = format!("{} dB", i18n::decimal(cx.locale, f64::from(db), 1));
        let anchor = egui::pos2(
            pointer.x.min(inner.right() - 48.0),
            (pointer.y - 18.0).max(inner.top()),
        );
        let galley = painter.layout_no_wrap(
            text,
            FontId::proportional(theme::type_size::BADGE),
            ink.text(),
        );
        let rect = Rect::from_min_size(
            egui::pos2(anchor.x - 4.0, anchor.y - 2.0),
            Vec2::new(galley.size().x + 8.0, galley.size().y + 4.0),
        );
        painter.rect_filled(rect, theme::radius::CONTROL, ink.panel());
        painter.rect_stroke(
            rect,
            theme::radius::CONTROL,
            egui::Stroke::new(1.0, ink.border()),
            egui::StrokeKind::Inside,
        );
        painter.galley(egui::pos2(anchor.x, anchor.y), galley, ink.text());
    }

    /// Whether a press point is close enough to the volume line to grab it.
    fn near_volume_line(
        &self,
        cx: &Context<'_>,
        item: &ItemView,
        inner: Rect,
        point: egui::Pos2,
        layout: &TimelineLayout,
    ) -> bool {
        if !inner.x_range().contains(point.x) {
            return false;
        }
        let Some(base) = item.volume else {
            return false;
        };
        let channel = volume_channel(cx, item.id);
        let at = self.tick_at(point.x, layout.body.left());
        let db = channel.map_or(base, |channel| {
            item_db_at(channel, at.saturating_sub(item.start), base)
        });
        let y = inner.top() + audio::line_fraction(db) * inner.height();
        (point.y - y).abs() <= 6.0
    }
}

fn volume_channel<'a>(
    cx: &'a Context<'_>,
    item: crate::editor::timeline::ItemId,
) -> Option<&'a project::Channel> {
    cx.staged_project()
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| track.items.iter())
        .find(|candidate| candidate.id == item)
        .and_then(|candidate| {
            candidate
                .animations
                .get(&project::AnimationPath::new("volume").ok()?)
        })
        .and_then(|param| match param {
            project::Param::Value(channel) => Some(channel),
            project::Param::Color(_) => None,
        })
}
fn item_db_at(channel: &project::Channel, time: crate::editor::timeline::Tick, base: f32) -> f32 {
    match channel.value_at(time) {
        project::ParamValue::Scalar(value) => (value as f32).clamp(audio::DB_MIN, audio::DB_MAX),
        _ => base,
    }
}

impl State {
    /// Frame tiles at the asset aspect: the decoded thumbnail when there is
    /// one, otherwise quiet placeholder frames so the strip still reads as footage.
    fn paint_filmstrip(
        &mut self,
        cx: &Context<'_>,
        ui: &egui::Ui,
        painter: &egui::Painter,
        strip: Rect,
        item: &ItemView,
        dark: bool,
    ) {
        let texture = item
            .source_asset
            .and_then(|asset| cx.thumbnails.texture(ui.ctx(), asset));
        let aspect = texture.as_ref().map_or(16.0 / 9.0, |texture| {
            let [width, height] = texture.size();
            width.max(1) as f32 / height.max(1) as f32
        });
        let tile = (strip.height() * aspect).max(8.0);
        let hover = theme::hover(dark);
        let visible = strip.intersect(painter.clip_rect());
        if !visible.is_positive() {
            return;
        }
        let first = ((visible.left() - strip.left()) / tile).floor();
        let left = visible.left() - (visible.left() - strip.left()).rem_euclid(tile);
        let count = ((visible.right() - left) / tile).ceil() as usize;
        for index in 0..count {
            let x = left + index as f32 * tile;
            let rect =
                Rect::from_min_size(egui::pos2(x, strip.top()), Vec2::new(tile, strip.height()));
            match &texture {
                Some(texture) => {
                    painter.image(
                        texture.id(),
                        rect,
                        Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        Color32::WHITE,
                    );
                }
                None => {
                    let shade = if (first + index as f32).rem_euclid(2.0) < 1.0 {
                        hover
                    } else {
                        theme::field(dark)
                    };
                    painter.rect_filled(rect.shrink(0.5), 0.0, shade);
                }
            }
        }
    }
}

/// Deterministic amplitude bars until the engine publishes waveform peaks.
fn paint_waveform(
    painter: &egui::Painter,
    area: Rect,
    item: &ItemView,
    color: Color32,
    reach: f32,
) {
    let visible = area.intersect(painter.clip_rect());
    if !visible.is_positive() {
        return;
    }
    let center = area.center().y;
    let bars = (area.width() / 3.0).floor().max(1.0);
    let first = ((visible.left() - area.left()) / 3.0).floor();
    let left = visible.left() - (visible.left() - area.left()).rem_euclid(3.0);
    let count = ((visible.right() - left) / 3.0).ceil() as usize;
    for index in 0..count {
        let t = (first + index as f32) / bars;
        // Synthetic stand-in amplitude until the engine publishes waveform
        // peaks; the display floor and log curve match the source mapping.
        let amplitude = ((t * 47.0 + item.id.0 as f32).sin() * 0.5 + 0.5) * 0.8 + 0.1;
        let half = (area.height() * 0.5 * reach * audio::bar_fraction(amplitude)).max(1.0);
        let x = left + index as f32 * 3.0 + 1.5;
        painter.line_segment(
            [egui::pos2(x, center - half), egui::pos2(x, center + half)],
            egui::Stroke::new(1.0, color),
        );
    }
}

pub(in crate::editor::timeline) fn item_label(
    content: &ItemParams,
    locale: Locale,
    dark: bool,
) -> (String, Option<[u8; 4]>) {
    match content {
        ItemParams::Video(_) | ItemParams::Audio(_) => (String::new(), None),
        ItemParams::Text(params) => {
            let value = params.content.as_str();
            if value.is_empty() {
                (i18n::text(locale, Text::TextItem).to_owned(), None)
            } else {
                (display_label(value, LABEL_CHARS), None)
            }
        }
        ItemParams::Caption(_) => (i18n::text(locale, Text::Caption).to_owned(), None),
        ItemParams::Overlay(params) => {
            let [r, g, b, _] = params.color.0;
            let color = if dark { [r, g, b, 90] } else { [r, g, b, 60] };
            (String::new(), Some(color))
        }
    }
}
pub(in crate::editor::timeline) fn item_palette(
    kind: TrackKind,
    dark: bool,
    selected: bool,
    overlay: Option<[u8; 4]>,
) -> (Color32, Color32, Color32) {
    let text = theme::text(dark);
    let (fill, border) = match kind {
        TrackKind::Video => (theme::video_fill(dark), theme::video_border(dark)),
        TrackKind::Audio => (theme::video_fill(dark), theme::video_border(dark)),
        _ => (theme::hover(dark), theme::border(dark)),
    };
    let fill = overlay.map_or(fill, |[r, g, b, a]| {
        Color32::from_rgba_unmultiplied(r, g, b, a)
    });
    if selected {
        (fill, theme::accent(dark), text)
    } else {
        (fill, border, text)
    }
}

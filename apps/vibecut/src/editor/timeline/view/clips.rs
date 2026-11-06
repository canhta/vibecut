//! Renders timeline clips.

use crate::editor::timeline::{
    Align2, Color32, DragMode, Editor, FontId, Id, ItemParams, ItemView, LABEL_CHARS, Locale, Rect,
    Text, TimelineLayout, TrackKind, Vec2, catalog, display_label, egui, theme,
};

impl Editor {
    pub(in crate::editor::timeline) fn paint_item(
        &mut self,
        ui: &egui::Ui,
        layout: &TimelineLayout,
        item: &ItemView,
        modifiers: egui::Modifiers,
    ) {
        let x0 = self.x_at(item.start, layout.body.left());
        let x1 = self.x_at(item.end, layout.body.left());
        let full = Rect::from_min_max(
            egui::pos2(x0, item.top + 2.0),
            egui::pos2(x1, item.top + item.height - 2.0),
        );
        let visible = full.intersect(layout.body);
        if !visible.is_positive() {
            return;
        }
        let selected = self.selection.contains(&item.id);
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
        let (fill, border, text) =
            item_palette(item.kind, self.preferences.dark, selected, item.overlay);
        let painter = ui.painter().with_clip_rect(visible);
        painter.rect_filled(visible, theme::radius::PANEL, fill);
        painter.rect_stroke(
            visible,
            0.0,
            egui::Stroke::new(if selected { 2.0 } else { 1.0 }, border),
            egui::StrokeKind::Inside,
        );
        match item.kind {
            TrackKind::Video if item.overlay.is_none() => {
                let mut x = visible.left() + 4.0;
                let mut index = 0;
                while x + 10.0 < visible.right() && index < 256 {
                    let shade = if index % 2 == 0 { 12 } else { 6 };
                    painter.rect_filled(
                        Rect::from_min_size(
                            egui::pos2(x, visible.top() + 3.0),
                            Vec2::new(9.0, visible.height() - 6.0),
                        ),
                        0.0,
                        Color32::from_rgba_unmultiplied(text.r(), text.g(), text.b(), shade),
                    );
                    x += 13.0;
                    index += 1;
                }
            }
            TrackKind::Audio => {
                let center = visible.center().y;
                let bars = ((visible.width() / 4.0).floor() as usize).min(256);
                for index in 0..bars {
                    let t = index as f32 / bars.max(1) as f32;
                    let scale = ((t * 47.0 + item.id.0 as f32).sin() * 0.5 + 0.5) * 0.8 + 0.1;
                    let half = (visible.height() * 0.35 * scale).max(1.0);
                    let x = visible.left() + index as f32 * 4.0 + 2.0;
                    painter.line_segment(
                        [egui::pos2(x, center - half), egui::pos2(x, center + half)],
                        egui::Stroke::new(1.0, text),
                    );
                }
            }
            _ => {}
        }
        painter.text(
            egui::pos2(visible.left() + 6.0, visible.center().y),
            Align2::LEFT_CENTER,
            &item.label,
            FontId::proportional(theme::type_size::BODY),
            text,
        );
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
                ui.painter().with_clip_rect(rect).line_segment(
                    [rect.center_top(), rect.center_bottom()],
                    egui::Stroke::new(
                        2.0,
                        if selected || response.hovered() {
                            self.accent()
                        } else {
                            border
                        },
                    ),
                );
                if response.clicked() {
                    self.select_item(item.id, modifiers);
                }
                if response.drag_started() && self.drag.is_none() {
                    self.begin_drag(item.id, mode, ui);
                }
            }
        }
        if response.clicked() {
            self.select_item(item.id, modifiers);
        }
        if !item.locked && self.drag.is_none() && response.drag_started() {
            self.begin_drag(item.id, DragMode::Move, ui);
        }
        if let Some(asset) = item.source_asset {
            response.context_menu(|ui| {
                let reveal = ui.button(self.tr(Text::RevealMedia));
                self.remember(format!("timeline.item.{}.reveal", item.id.0), &reveal);
                if reveal.clicked() {
                    self.action(&format!("media.reveal.{}", asset.0));
                    ui.close();
                }
            });
        }
    }
}

pub(in crate::editor::timeline) fn item_label(
    content: &ItemParams,
    locale: Locale,
    dark: bool,
) -> (String, Option<[u8; 4]>) {
    match content {
        ItemParams::Video(_) => (catalog::text(locale, Text::Video).to_owned(), None),
        ItemParams::Audio(_) => (catalog::text(locale, Text::Audio).to_owned(), None),
        ItemParams::Text(params) => {
            let value = params.content.as_str();
            if value.is_empty() {
                (catalog::text(locale, Text::TextItem).to_owned(), None)
            } else {
                (display_label(value, LABEL_CHARS), None)
            }
        }
        ItemParams::Caption(_) => (catalog::text(locale, Text::Caption).to_owned(), None),
        ItemParams::Overlay(params) => {
            let [r, g, b, _] = params.color.0;
            let color = if dark { [r, g, b, 90] } else { [r, g, b, 60] };
            (catalog::text(locale, Text::Overlay).to_owned(), Some(color))
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
        TrackKind::Audio => (theme::field(dark), theme::border(dark)),
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

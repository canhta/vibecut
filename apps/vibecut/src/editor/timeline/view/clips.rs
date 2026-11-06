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
            egui::pos2(x0, item.top + CLIP_INSET),
            egui::pos2(x1, item.top + item.height - CLIP_INSET),
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
        let dark = self.preferences.dark;
        let (fill, border, text) = item_palette(item.kind, dark, selected, item.overlay);
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
                self.paint_filmstrip(ui, &painter, strip, item, dark);
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
                egui::pos2(visible.left() + 6.0, visible.center().y),
                Align2::LEFT_CENTER,
                &item.label,
                FontId::proportional(theme::type_size::BODY),
                text,
            );
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
                        egui::Stroke::new(2.0, self.accent()),
                    );
                }
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

/// Gap between a lane's edge and its clip: a 76 px video lane holds the 74 px clip.
const CLIP_INSET: f32 = 1.0;

impl Editor {
    /// Frame tiles at the asset aspect: the decoded thumbnail when there is
    /// one, otherwise quiet placeholder frames so the strip still reads as footage.
    fn paint_filmstrip(
        &mut self,
        ui: &egui::Ui,
        painter: &egui::Painter,
        strip: Rect,
        item: &ItemView,
        dark: bool,
    ) {
        let texture = item
            .source_asset
            .and_then(|asset| self.thumbnail_texture(ui.ctx(), asset));
        let aspect = texture.as_ref().map_or(16.0 / 9.0, |texture| {
            let [width, height] = texture.size();
            width.max(1) as f32 / height.max(1) as f32
        });
        let tile = (strip.height() * aspect).max(8.0);
        let hover = theme::hover(dark);
        let mut index = 0;
        let mut x = strip.left();
        while x < strip.right() && index < 512 {
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
                    let shade = if index % 2 == 0 {
                        hover
                    } else {
                        theme::field(dark)
                    };
                    painter.rect_filled(rect.shrink(0.5), 0.0, shade);
                }
            }
            x += tile;
            index += 1;
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
    let center = area.center().y;
    let bars = ((area.width() / 3.0).floor() as usize).min(1024);
    for index in 0..bars {
        let t = index as f32 / bars.max(1) as f32;
        let scale = ((t * 47.0 + item.id.0 as f32).sin() * 0.5 + 0.5) * 0.8 + 0.1;
        let half = (area.height() * 0.5 * reach * scale).max(1.0);
        let x = area.left() + index as f32 * 3.0 + 1.5;
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
                (catalog::text(locale, Text::TextItem).to_owned(), None)
            } else {
                (display_label(value, LABEL_CHARS), None)
            }
        }
        ItemParams::Caption(_) => (catalog::text(locale, Text::Caption).to_owned(), None),
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

//! Renders applied transitions as selectable markers over their cuts.
use crate::editor::timeline::{
    Action, Align2, Context, FontId, Icon, Id, Rect, State, TimelineLayout, TrackRow, Vec2, egui,
    icons, theme,
    view::{Ink, tr},
};

/// A transition shorter than this on screen stays clickable at any zoom.
const MIN_WIDTH: f32 = 20.0;
const HEIGHT: f32 = 18.0;
const GLYPH: f32 = 12.0;
/// Room the name needs beside the glyph before it is drawn.
const LABEL_WIDTH: f32 = 64.0;

impl State {
    pub(in crate::editor::timeline) fn paint_transitions(
        &mut self,
        cx: &Context<'_>,
        ui: &egui::Ui,
        layout: &TimelineLayout,
        rows: &[TrackRow],
    ) {
        let ink = Ink::of(cx);
        let dark = ink.dark();
        let project = cx.staged_project();
        for transition in &project.data().scene().timeline.transitions {
            let Some(row) = rows.iter().find(|row| row.id == transition.track) else {
                continue;
            };
            let Ok(end) = transition.end() else { continue };
            let top = layout.body.top() + row.top - self.vertical_scroll;
            if top + row.base_height < layout.body.top() || top > layout.body.bottom() {
                continue;
            }
            let left = layout.body.left();
            let x0 = self.x_at(transition.start, left);
            let x1 = self.x_at(end, left);
            let width = (x1 - x0).max(MIN_WIDTH);
            let center = (x0 + x1) / 2.0;
            let rect = Rect::from_center_size(
                egui::pos2(center, top + row.base_height / 2.0),
                Vec2::new(width, HEIGHT),
            )
            .intersect(layout.body);
            if !rect.is_positive() {
                continue;
            }
            let selected = cx.transition == Some(transition.id);
            let response = ui.interact(
                rect,
                Id::unique(("timeline.transition", transition.id.0)),
                egui::Sense::click(),
            );
            let name = tr(cx, crate::i18n::transition_label(&transition.kind));
            response.widget_info(|| {
                egui::WidgetInfo::selected(egui::Role::Button, true, selected, name)
            });
            self.remember_with(
                format!("timeline.transition.{}", transition.id.0),
                Some(Icon::ArrowsLeftRight.name()),
                Some(name.to_owned()),
                &response,
            );
            let (fill, ink_color) = if selected {
                (ink.accent(), theme::window(dark))
            } else {
                (ink.accent().gamma_multiply(0.35), ink.text())
            };
            let stroke = if selected {
                egui::Stroke::new(1.5, theme::text(dark))
            } else {
                egui::Stroke::new(1.0, ink.accent())
            };
            let painter = ui.painter().with_clip_rect(rect);
            painter.rect_filled(rect, theme::radius::CONTROL, fill);
            painter.rect_stroke(
                rect,
                theme::radius::CONTROL,
                stroke,
                egui::StrokeKind::Inside,
            );
            let wide = rect.width() > LABEL_WIDTH;
            let glyph_center = if wide {
                egui::pos2(rect.left() + GLYPH, rect.center().y)
            } else {
                rect.center()
            };
            icons::tinted(Icon::ArrowsLeftRight, GLYPH, ink_color)
                .paint_at(ui, Rect::from_center_size(glyph_center, Vec2::splat(GLYPH)));
            if wide {
                painter.text(
                    egui::pos2(glyph_center.x + GLYPH, rect.center().y),
                    Align2::LEFT_CENTER,
                    name,
                    FontId::proportional(theme::type_size::BODY),
                    ink_color,
                );
            }
            if response.clicked() {
                self.push(Action::SelectTransition(transition.id));
            }
        }
    }
}

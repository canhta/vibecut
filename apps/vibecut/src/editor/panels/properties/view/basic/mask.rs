//! Draws the Basic tab's Mask row and routes its sub-panel.
use super::super::Panel;
use crate::editor::properties::{
    Action, ElementKind, ItemId, Rect, Text, egui, mask_picker, read, theme,
};
use crate::i18n;
use crate::kit;
use project::MaskShape;

impl Panel<'_, '_> {
    /// The Basic tab's Mask row: current shape, or None, with a right chevron.
    /// Video and image items only; a locked track still opens read-only.
    pub(in super::super) fn mask_row(
        &mut self,
        ui: &mut egui::Ui,
        item: ItemId,
        kind: ElementKind,
    ) {
        if !matches!(kind, ElementKind::Video | ElementKind::Image) {
            return;
        }
        let mask = read::item(self.cx.session.project(), item).and_then(mask_picker::current);
        let value = match mask {
            None => self.tr(Text::DecorationNone),
            Some(mask) => shape_label(&mask.shape, self.cx.locale),
        };
        let (row, _) = ui.allocate_exact_size(
            egui::Vec2::new(ui.available_width(), theme::size::PROPERTY_ROW),
            egui::Sense::hover(),
        );
        let response = ui
            .interact(
                row,
                ui.scope_id().with("inspector.mask.row"),
                egui::Sense::click(),
            )
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        if response.hovered() {
            ui.painter()
                .rect_filled(row, theme::radius::CONTROL, theme::hover(self.cx.dark));
        }
        let color = theme::text(self.cx.dark);
        ui.painter().text(
            egui::pos2(row.left(), row.center().y),
            egui::Align2::LEFT_CENTER,
            self.tr(Text::Mask),
            theme::label_font(),
            color,
        );
        ui.painter().text(
            egui::pos2(row.left() + super::super::VALUE_COLUMN, row.center().y),
            egui::Align2::LEFT_CENTER,
            value,
            theme::label_font(),
            theme::secondary(self.cx.dark),
        );
        let caret = Rect::from_center_size(
            egui::pos2(row.right() - 12.0, row.center().y),
            egui::Vec2::splat(12.0),
        );
        ui.put(caret, kit::tinted(kit::Icon::CaretRight, 12.0, color));
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::Role::Button, true, self.tr(Text::Mask))
        });
        self.remember("inspector.mask.row", &response);
        if response.clicked() {
            self.state.mask.open(item);
        }
    }

    /// Draw the open sub-panel and collect its actions.
    pub(in super::super) fn mask_dialog(&mut self, ctx: &egui::Context) {
        if !self.state.mask.is_open() {
            return;
        }
        let Some(item) = self.state.mask.item() else {
            self.state.mask.close();
            return;
        };
        let mask = read::item(self.cx.session.project(), item)
            .and_then(mask_picker::current)
            .cloned();
        let locked = read::item_locked(self.cx.session.project(), item);
        let project = self.cx.session.project();
        let timing = read::item(project, item).and_then(|found| {
            let (start, end) = read::range_of(project, item)?;
            let at = self.cx.playhead;
            Some(mask_picker::Timing {
                animations: &found.animations,
                time: time::Tick(at.0 - start.0),
                inside: start <= at && at < end,
            })
        });
        let anchor = self.state.area.map_or_else(
            || egui::pos2(ctx.content_rect().left() + 24.0, 120.0),
            |panel| {
                egui::pos2(
                    panel.left() - theme::space::PANEL_GUTTER,
                    panel.top() + theme::size::TAB_STRIP,
                )
            },
        );
        let event = mask_picker::show(
            &mut self.state.mask,
            ctx,
            self.cx.locale,
            self.cx.dark,
            anchor,
            item,
            mask.as_ref(),
            locked,
            timing,
        );
        for (id, response) in event.widgets {
            self.remember(id, &response);
        }
        if event.editing {
            self.state.focus = Some("inspector.mask".to_owned());
        } else if self.state.focus.as_deref() == Some("inspector.mask") {
            self.state.focus = None;
        }
        for action in event.actions {
            self.actions.push(Action::Mask(action));
        }
    }
}

/// The row's value: a capture label, or the engine name for shapes agents may
/// still add through commands.
fn shape_label(shape: &MaskShape, locale: crate::i18n::Locale) -> &'static str {
    let key = match mask_picker::Choice::of(shape) {
        Some(choice) => choice.label(),
        None => match shape {
            MaskShape::Diamond => Text::MaskDiamond,
            MaskShape::Text { .. } => Text::MaskText,
            MaskShape::Path { .. } => Text::MaskPath,
            _ => Text::Mask,
        },
    };
    i18n::text(locale, key)
}

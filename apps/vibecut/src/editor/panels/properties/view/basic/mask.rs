//! Draws the Basic tab's Mask row and routes its sub-panel.
use super::super::Panel;
use crate::editor::properties::{
    Action, ElementKind, ItemId, Rect, Text, egui, mask_picker, read, theme,
};
use crate::i18n;
use crate::kit;
use project::{MaskScope, MaskShape};

/// Add mask button size, measured in frame 105253 (logical px).
const ADD_MASK_WIDTH: f32 = 103.0;
const ADD_MASK_HEIGHT: f32 = 27.0;
/// The reference lays its mask tiles six to a row (frame 105253).
const INLINE_TILES_PER_ROW: usize = 6;

impl Panel<'_, '_> {
    /// The Basic tab's Mask row: the clip mask's current shape, or None, with a right chevron.
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
        let scope = MaskScope::Clip;
        let mask = read::item(self.cx.session.project(), item)
            .and_then(|found| mask_picker::current(found, scope));
        let row_id = "inspector.mask.row";
        let value = match mask {
            None => self.tr(Text::DecorationNone),
            Some(mask) => shape_label(&mask.shape, self.cx.locale),
        };
        if mask.is_none()
            && self.empty_cta(
                ui,
                "inspector.mask.empty",
                kit::Icon::CircleHalf,
                Text::MaskNone,
                ("inspector.mask.add", kit::Icon::Plus, Text::MaskAdd),
            )
        {
            self.state.mask.open(item, scope);
        }
        let (row, _) = ui.allocate_exact_size(
            egui::Vec2::new(ui.available_width(), theme::size::PROPERTY_ROW),
            egui::Sense::hover(),
        );
        let response = ui
            .interact(row, ui.scope_id().with(row_id), egui::Sense::click())
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
        self.remember(row_id, &response);
        if response.clicked() {
            self.state.mask.open(item, scope);
        }
    }

    /// The inline Mask page of an effect or adjustment layer (clip scope) and of the Adjust
    /// tab's Mask sub-tab (adjust scope), as frames 105253 and 102732 show it: a `Mask`
    /// checkbox row, an `Add mask` button and the shape tiles, grey until a mask exists.
    pub(in super::super) fn mask_page(
        &mut self,
        ui: &mut egui::Ui,
        item: ItemId,
        kind: ElementKind,
        scope: MaskScope,
    ) {
        let served = match scope {
            MaskScope::Clip => kind == ElementKind::Effect,
            MaskScope::Adjust => matches!(kind, ElementKind::Video | ElementKind::Image),
        };
        if !served {
            return;
        }
        self.state.mask.set_scope(scope);
        let project = self.cx.session.project();
        let mask = read::item(project, item)
            .and_then(|found| mask_picker::current(found, scope))
            .cloned();
        let locked = read::item_locked(project, item);
        let dark = self.cx.dark;
        // The checkbox switches the mask on and off; it has nothing to switch before Add mask.
        let mut on = mask.as_ref().is_some_and(|mask| mask.enabled);
        let (row, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), theme::size::PROPERTY_ROW),
            egui::Sense::hover(),
        );
        let mut cells = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(row)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        cells.spacing_mut().icon_spacing = theme::space::XS;
        let checkbox = cells.add_enabled(
            mask.is_some() && !locked,
            egui::Checkbox::new(
                &mut on,
                egui::RichText::new(self.tr(Text::Mask))
                    .size(theme::type_size::BODY)
                    .color(theme::text(dark)),
            ),
        );
        self.remember("inspector.mask.enabled", &checkbox);
        if checkbox.changed()
            && let Some(mut changed) = mask.clone()
        {
            changed.enabled = on;
            self.actions.push(Action::Mask(mask_picker::Action::Update {
                item,
                mask: changed,
            }));
        }
        // Add mask: 103 x 27 logical in frame 105253 (112 x 29 of 2000 px, scaled by 0.92).
        let can_add = mask.is_none() && !locked;
        let (button, add) = ui.allocate_exact_size(
            egui::vec2(ADD_MASK_WIDTH, ADD_MASK_HEIGHT),
            if can_add {
                egui::Sense::click()
            } else {
                egui::Sense::hover()
            },
        );
        let fill = if can_add && add.hovered() {
            theme::hover(dark)
        } else {
            theme::field(dark)
        };
        let ink = if can_add {
            theme::text(dark)
        } else {
            theme::secondary(dark)
        };
        ui.painter()
            .rect_filled(button, theme::radius::CONTROL, fill);
        ui.put(
            Rect::from_center_size(
                egui::pos2(button.left() + 14.0, button.center().y),
                egui::Vec2::splat(12.0),
            ),
            kit::tinted(kit::Icon::Plus, 12.0, ink),
        );
        ui.painter().text(
            egui::pos2(button.left() + 28.0, button.center().y),
            egui::Align2::LEFT_CENTER,
            self.tr(Text::MaskAdd),
            theme::label_font(),
            ink,
        );
        add.widget_info(|| {
            egui::WidgetInfo::labeled(egui::Role::Button, can_add, self.tr(Text::MaskAdd))
        });
        self.remember("inspector.mask.add", &add);
        if add.clicked() && can_add {
            self.actions.push(Action::Mask(mask_picker::Action::Choose {
                item,
                shape: mask_picker::Choice::Rectangle.shape(),
            }));
        }
        ui.add_space(theme::space::MD);
        let timing = read::item(project, item).and_then(|found| {
            let (start, end) = read::range_of(project, item)?;
            let at = self.cx.playhead;
            Some(mask_picker::Timing {
                animations: &found.animations,
                start,
                time: time::Tick(at.0 - start.0),
                inside: start <= at && at < end,
            })
        });
        let page = mask_picker::Page {
            locale: self.cx.locale,
            dark,
            item,
            mask: mask.as_ref(),
            locked,
            timing,
            per_row: INLINE_TILES_PER_ROW,
            inline: true,
        };
        let event = mask_picker::inline(&mut self.state.mask, ui, &page);
        // No mask yet: the page says so where the mask's fields go, under the grey tiles, so
        // the tiles keep their place; Add mask is its call-to-action.
        if mask.is_none() {
            ui.add_space(theme::space::MD);
            self.empty_face(
                ui,
                "inspector.mask.empty",
                kit::Icon::CircleHalf,
                Text::MaskNone,
            );
        }
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

    /// Draw the open sub-panel and collect its actions.
    pub(in super::super) fn mask_dialog(&mut self, ctx: &egui::Context) {
        if !self.state.mask.is_open() {
            return;
        }
        let Some(item) = self.state.mask.item() else {
            self.state.mask.close();
            return;
        };
        let scope = self.state.mask.scope();
        let mask = read::item(self.cx.session.project(), item)
            .and_then(|found| mask_picker::current(found, scope))
            .cloned();
        let locked = read::item_locked(self.cx.session.project(), item);
        let project = self.cx.session.project();
        let timing = read::item(project, item).and_then(|found| {
            let (start, end) = read::range_of(project, item)?;
            let at = self.cx.playhead;
            Some(mask_picker::Timing {
                animations: &found.animations,
                start,
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

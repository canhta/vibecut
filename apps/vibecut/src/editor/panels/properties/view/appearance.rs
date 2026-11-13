//! Renders appearance controls.
use super::Panel;
use crate::editor::properties::{
    Color32, ColorTarget, Id, ItemId, OPACITY_SCRUB, Rect, Text, Vec2, egui, read, theme,
};
use crate::i18n;

const VALUE_COLUMN: f32 = 120.0;
pub(super) const FIELD_COLUMN: f32 = 150.0;

impl Panel<'_, '_> {
    /// The right-hand region of a 39 px row, starting at the value column.
    pub(super) fn row_value_ui(&self, ui: &mut egui::Ui, height: f32) -> (Rect, egui::Ui) {
        let (row, _) = ui.allocate_exact_size(
            Vec2::new(ui.available_width(), height),
            egui::Sense::hover(),
        );
        let value = Rect::from_min_max(egui::pos2(row.left() + VALUE_COLUMN, row.top()), row.max);
        let child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(value)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        (row, child)
    }

    pub(super) fn row_label(&self, ui: &egui::Ui, row: Rect, text: &str) {
        let color = theme::text(self.cx.dark);
        let galley = ui.painter().layout_no_wrap(
            text.to_owned(),
            egui::FontId::proportional(theme::type_size::BODY),
            color,
        );
        ui.painter().galley(
            egui::pos2(row.left(), row.center().y - galley.size().y / 2.0),
            galley,
            color,
        );
    }

    /// The Blend section body: blend mode, then opacity. Opacity stays read-only
    /// outside its keyed range, as before the section moved into Basic.
    pub(super) fn blend_section(&mut self, ui: &mut egui::Ui, item: ItemId) {
        self.blend_control(ui, item);
        self.opacity_row(ui, item);
    }

    /// The Opacity slider with its keyframe diamond; keyed outside its range it is disabled.
    pub(super) fn opacity_row(&mut self, ui: &mut egui::Ui, item: ItemId) {
        let editable = !read::channel_has_keys(self.cx.session.project(), item, "opacity")
            || read::range_of(self.cx.session.project(), item)
                .is_some_and(|(start, end)| start <= self.cx.playhead && self.cx.playhead < end);
        self.slider_row_in(ui, item, OPACITY_SCRUB, editable);
    }

    pub(super) fn blend_control(&mut self, ui: &mut egui::Ui, item: ItemId) {
        let Some(current) = read::item(self.cx.session.project(), item).map(|item| item.blend_mode)
        else {
            return;
        };
        let (row, mut value_ui) = self.row_value_ui(ui, theme::size::PROPERTY_ROW);
        self.row_label(ui, row, self.tr(Text::BlendMode));
        let label = self.tr(i18n::blend_label(current));
        let response = value_ui
            .scope(|ui| {
                ui.spacing_mut().interact_size.y = theme::size::FIELD;
                egui::ComboBox::from_id_salt(Id::unique("inspector.blendMode"))
                    .selected_text(label)
                    .width(ui.available_width().min(FIELD_COLUMN))
                    .show_ui(ui, |ui| {
                        for mode in project::BlendMode::ALL {
                            let response = ui.selectable_label(
                                current == mode,
                                self.tr(i18n::blend_label(mode)),
                            );
                            self.remember(format!("inspector.blendMode.{}", mode.id()), &response);
                            if response.clicked() {
                                self.set_item_field(item, "blendMode", mode.id());
                                ui.close();
                            }
                        }
                    })
                    .response
            })
            .inner;
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::Role::ComboBox, true, self.tr(Text::BlendMode))
        });
        self.remember("inspector.blendMode", &response);
    }

    /// A colour row whose selected texts differ: the label and a dash, with no swatch to open.
    fn mixed_color_row(&mut self, ui: &mut egui::Ui, id: &str, label: Text) {
        let (row, mut value_ui) = self.row_value_ui(ui, theme::size::PROPERTY_ROW);
        self.row_label(ui, row, self.tr(label));
        let dark = self.cx.dark;
        let (rect, response) = value_ui.allocate_exact_size(
            Vec2::new(
                value_ui.available_width().min(FIELD_COLUMN),
                theme::size::FIELD,
            ),
            egui::Sense::hover(),
        );
        value_ui
            .painter()
            .rect_filled(rect, theme::radius::FIELD, theme::field(dark));
        value_ui.painter().text(
            egui::pos2(rect.left() + theme::space::MD, rect.center().y),
            egui::Align2::LEFT_CENTER,
            self.tr(Text::EmptyValue),
            theme::label_font(),
            theme::text(dark),
        );
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::Role::Label, true, self.tr(Text::EmptyValue))
        });
        self.remember(format!("{id}.mixed"), &response);
    }

    /// The Fit mode row of a video or image item: how its cropped source meets
    /// the canvas. The reference exposes no such row, so it is [new].
    pub(super) fn fit_control(&mut self, ui: &mut egui::Ui, item: ItemId) {
        let Some(current) =
            read::item(self.cx.session.project(), item).and_then(|item| match &item.content {
                project::ItemParams::Video(params) => Some(params.fit),
                _ => None,
            })
        else {
            return;
        };
        let (row, mut value_ui) = self.row_value_ui(ui, theme::size::PROPERTY_ROW);
        self.row_label(ui, row, self.tr(Text::FitMode));
        let label = self.tr(i18n::fit_label(current));
        let response = value_ui
            .scope(|ui| {
                ui.spacing_mut().interact_size.y = theme::size::FIELD;
                egui::ComboBox::from_id_salt(Id::unique("inspector.fit"))
                    .selected_text(label)
                    .width(ui.available_width().min(FIELD_COLUMN))
                    .show_ui(ui, |ui| {
                        for mode in project::FitMode::ALL {
                            let response = ui
                                .selectable_label(current == mode, self.tr(i18n::fit_label(mode)));
                            self.remember(format!("inspector.fit.{}", mode.id()), &response);
                            if response.clicked() {
                                self.set_item_field(item, "fit", mode.id());
                                ui.close();
                            }
                        }
                    })
                    .response
            })
            .inner;
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::Role::ComboBox, true, self.tr(Text::FitMode))
        });
        self.remember("inspector.fit", &response);
    }

    /// A colour row: the label left, a swatch with its hex right. Clicking opens
    /// the colour popover hanging under the field's right edge.
    pub(super) fn color_row(
        &mut self,
        ui: &mut egui::Ui,
        id: &str,
        label: Text,
        target: ColorTarget,
    ) {
        let color = match target.color(self.cx.session.project()) {
            Ok(color) => color,
            Err(_) if target.mixed(self.cx.session.project()) => {
                self.mixed_color_row(ui, id, label);
                return;
            }
            Err(_) => return,
        };
        let [r, g, b, a] = color.0;
        let (row, mut value_ui) = self.row_value_ui(ui, theme::size::PROPERTY_ROW);
        self.row_label(ui, row, self.tr(label));
        let dark = self.cx.dark;
        let (rect, response) = value_ui.allocate_exact_size(
            Vec2::new(
                value_ui.available_width().min(FIELD_COLUMN),
                theme::size::FIELD,
            ),
            egui::Sense::click(),
        );
        let fill = if response.hovered() {
            theme::hover(dark)
        } else {
            theme::field(dark)
        };
        value_ui
            .painter()
            .rect_filled(rect, theme::radius::FIELD, fill);
        let swatch = Rect::from_center_size(
            egui::pos2(rect.left() + theme::space::MD + 8.0, rect.center().y),
            Vec2::splat(16.0),
        );
        value_ui
            .painter()
            .rect_filled(swatch, 2.0, Color32::from_rgba_unmultiplied(r, g, b, a));
        value_ui.painter().rect_stroke(
            swatch,
            2.0,
            egui::Stroke::new(theme::space::HAIRLINE, theme::border(dark)),
            egui::StrokeKind::Inside,
        );
        value_ui.painter().text(
            egui::pos2(swatch.right() + theme::space::MD, rect.center().y),
            egui::Align2::LEFT_CENTER,
            format!("#{r:02X}{g:02X}{b:02X}"),
            theme::label_font(),
            theme::text(dark),
        );
        response
            .widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, self.tr(label)));
        let response = response.on_hover_text(self.tr(label));
        self.remember_with_icon(id, None, &response);
        if response.clicked() {
            let anchor = egui::pos2(rect.right(), rect.bottom() + theme::space::XS);
            match self
                .state
                .open_picker(self.cx.session.project(), target, Some(anchor))
            {
                Ok(actions) => self.actions.extend(actions),
                Err(error) => self.fail(error),
            }
        }
    }
}

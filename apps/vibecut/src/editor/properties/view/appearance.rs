//! Renders appearance controls.
use crate::editor::properties::{
    Color32, ColorTarget, Editor, Id, ItemId, ItemParams, Rect, Text, Vec2, catalog, egui, theme,
};

use crate::kit;

fn value_ui_width(ui: &egui::Ui) -> f32 {
    ui.available_width().min(FIELD_COLUMN)
}

const VALUE_COLUMN: f32 = 120.0;
const FIELD_COLUMN: f32 = 150.0;

impl Editor {
    /// The right-hand region of a 39 px row, starting at the value column.
    pub(in crate::editor) fn row_value_ui(
        &self,
        ui: &mut egui::Ui,
        height: f32,
    ) -> (Rect, egui::Ui) {
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

    pub(in crate::editor) fn row_label(&self, ui: &egui::Ui, row: Rect, text: &str) {
        let color = theme::text(self.preferences.dark);
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

    pub(in crate::editor) fn blend_control(&mut self, ui: &mut egui::Ui, item: ItemId) {
        let Some(current) = self
            .session
            .project()
            .data()
            .scene()
            .timeline
            .iter()
            .flat_map(|track| &track.items)
            .find(|candidate| candidate.id == item)
            .map(|item| item.blend_mode)
        else {
            return;
        };
        let (row, mut value_ui) = self.row_value_ui(ui, theme::size::PROPERTY_ROW);
        self.row_label(ui, row, self.tr(Text::BlendMode));
        let label = self.tr(catalog::blend_label(current));
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
                                self.tr(catalog::blend_label(mode)),
                            );
                            self.remember(format!("inspector.blendMode.{}", mode.id()), &response);
                            if response.clicked() {
                                if let Err(error) =
                                    self.set_item_field(item, "blendMode", mode.id())
                                {
                                    self.error = Some(error.to_string());
                                }
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

    /// A colour row: the label left, a swatch with its hex right. Clicking opens
    /// the picker as a popover hanging off the panel's left edge.
    pub(in crate::editor) fn color_row(
        &mut self,
        ui: &mut egui::Ui,
        id: &str,
        label: Text,
        target: ColorTarget,
    ) {
        let Ok(color) = target.color(self.session.project()) else {
            return;
        };
        let [r, g, b, a] = color.0;
        let (row, mut value_ui) = self.row_value_ui(ui, theme::size::PROPERTY_ROW);
        self.row_label(ui, row, self.tr(label));
        let dark = self.preferences.dark;
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
            let panel_left = self
                .panels
                .get("inspector")
                .map_or(rect.left(), |panel| panel.left());
            match self.open_color_picker(target) {
                Ok(()) => {
                    if let Some(picker) = self.color_picker.as_mut() {
                        picker.anchor = Some(egui::pos2(
                            panel_left - theme::space::PANEL_GUTTER,
                            row.top(),
                        ));
                    }
                }
                Err(error) => self.error = Some(error.to_string()),
            }
        }
    }
    pub(in crate::editor) fn text_color_field(&mut self, ui: &mut egui::Ui, item: ItemId) {
        let Some(value) = self
            .session
            .project()
            .data()
            .scene()
            .timeline
            .iter()
            .flat_map(|track| &track.items)
            .find(|candidate| candidate.id == item)
            .and_then(|item| match &item.content {
                ItemParams::Text(params) => {
                    let [r, g, b, a] = params.color.0;
                    Some(format!("#{r:02X}{g:02X}{b:02X}{a:02X}"))
                }
                _ => None,
            })
        else {
            return;
        };
        let (row, mut value_ui) = self.row_value_ui(ui, theme::size::PROPERTY_ROW);
        self.row_label(ui, row, self.tr(Text::TextColor));
        let draft = self
            .property_drafts
            .get("text_color")
            .cloned()
            .unwrap_or(value);
        let mut next = draft;
        let field_width = value_ui_width(&value_ui);
        let response = kit::value_field(
            &mut value_ui,
            "inspector.text_color",
            &mut next,
            field_width,
            None,
            self.tr(Text::TextColor),
        );
        self.remember("inspector.text_color", &response);
        if response.has_focus() {
            self.property_focus = Some("text_color".to_owned());
        }
        if response.changed() {
            self.property_drafts
                .insert("text_color".to_owned(), next.clone());
        }
        let editing = response.has_focus() || self.property_focus.as_deref() == Some("text_color");
        if editing && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.property_drafts.remove("text_color");
            self.property_focus = None;
            response.surrender_focus();
            return;
        }
        let enter = response.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if enter || response.lost_focus() {
            self.property_focus = None;
            if let Some(draft) = self.property_drafts.remove("text_color")
                && let Err(error) = self.set_item_field(item, "text_color", &draft)
            {
                self.error = Some(error.to_string());
            }
        }
    }
}

//! Renders the properties area.

use super::{ColorTarget, Editor, Text, catalog, egui, theme};

pub(in crate::editor) fn show(editor: &mut Editor, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        if editor.text_button(ui, "inspector.project", Text::Project) {
            editor.action("inspector.project");
        }
        if editor.text_button(ui, "inspector.details", Text::Details) {
            editor.action("inspector.details");
        }
    });
    ui.separator();
    ui.add_space(theme::space::MD);
    if editor.details {
        editor.property_focus = None;
        editor.section(
            ui,
            "inspector.section.canvas",
            "inspector.canvas",
            Text::Canvas,
            true,
            |editor, ui| {
                let locale = editor.preferences.locale;
                let data = editor.session.project().data();
                let width = catalog::number(locale, u64::from(data.canvas.width));
                let height = catalog::number(locale, u64::from(data.canvas.height));
                ui.label(catalog::format(
                    locale,
                    Text::CanvasSize,
                    &[("width", &width), ("height", &height)],
                ));
                let numerator = catalog::signed(locale, data.canvas.fps.num());
                let denominator = catalog::signed(locale, data.canvas.fps.den());
                ui.label(catalog::format(
                    locale,
                    Text::FrameRate,
                    &[("num", &numerator), ("den", &denominator)],
                ));
                let rate = catalog::number(locale, u64::from(data.settings.audio_sample_rate));
                ui.label(catalog::format(
                    locale,
                    Text::SampleRate,
                    &[("rate", &rate)],
                ));
            },
        );
        editor.section(
            ui,
            "inspector.section.metadata",
            "inspector.metadata",
            Text::Metadata,
            false,
            |editor, ui| {
                let locale = editor.preferences.locale;
                let created = catalog::date(locale, editor.session.metadata().created_unix_ms);
                let modified = catalog::date(locale, editor.session.metadata().modified_unix_ms);
                ui.label(
                    egui::RichText::new(catalog::format(
                        locale,
                        Text::CreatedDate,
                        &[("date", &created)],
                    ))
                    .small()
                    .color(editor.secondary()),
                );
                ui.label(
                    egui::RichText::new(catalog::format(
                        locale,
                        Text::ModifiedDate,
                        &[("date", &modified)],
                    ))
                    .small()
                    .color(editor.secondary()),
                );
            },
        );
    } else if editor.selection.is_empty() {
        editor.property_focus = None;
        ui.label(editor.tr(Text::EmptyInspector));
        editor.color_button(
            ui,
            "inspector.canvas_background",
            Text::CanvasBackground,
            ColorTarget::Canvas,
        );
    } else if editor.selection.len() == 1 {
        editor.properties(ui);
    } else {
        editor.property_focus = None;
        egui::ScrollArea::vertical()
            .id_salt("inspector.shared")
            .show(ui, |ui| {
                editor.section(
                    ui,
                    "inspector.section.selection",
                    "inspector.selection",
                    Text::Selection,
                    true,
                    |editor, ui| editor.shared_inspector(ui),
                );
            });
    }
}

mod dialog;
pub(in crate::editor) use dialog::show_dialog;

use super::{
    AUDIO_GAIN_SCRUB, ElementKind, Error, ItemId, OPACITY_SCRUB, PropertiesTab, SelectionScope,
    TEXT_SIZE_SCRUB, Vec2, shared_fields,
};

impl Editor {
    /// One selected element: its type's tab set, remembered per element type.
    pub(in crate::editor) fn properties(&mut self, ui: &mut egui::Ui) {
        let Some(item) = self.selected_item_id() else {
            return;
        };
        let Some((kind, has_audio)) = self.element_kind(item) else {
            return;
        };
        if self.property_selection != self.selection {
            self.property_drafts.clear();
            self.property_focus = None;
            self.property_selection = self.selection.clone();
        }
        let tabs: Vec<PropertiesTab> = kind
            .tabs()
            .iter()
            .copied()
            .filter(|tab| {
                !(*tab == PropertiesTab::Audio && kind == ElementKind::Video && !has_audio)
            })
            .collect();
        let stored = self
            .preferences
            .properties_tabs
            .get(kind.id())
            .and_then(|id| PropertiesTab::from_id(id))
            .filter(|tab| tabs.contains(tab));
        let active = stored.unwrap_or_else(|| kind.default_tab());
        match self.selection_scopes.active(&self.selection) {
            SelectionScope::MaskPoints => {
                ui.label(self.tr(Text::MaskPoints));
            }
            SelectionScope::Keyframes => {
                ui.label(self.tr(Text::Keyframes));
            }
            _ => {}
        }
        let mut clicked = None;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            for tab in &tabs {
                let id = format!("inspector.tab.{}", tab.id());
                let color = if *tab == active {
                    self.accent()
                } else {
                    self.secondary()
                };
                let label = egui::RichText::new(self.tr(tab.label()))
                    .size(theme::type_size::CHROME)
                    .color(color);
                let response = ui
                    .push_id(id.clone(), |ui| {
                        ui.add(
                            egui::Button::new(label)
                                .frame(false)
                                .wrap_mode(egui::TextWrapMode::Extend)
                                .min_size(Vec2::new(0.0, 32.0)),
                        )
                    })
                    .inner;
                self.remember(&id, &response);
                if *tab == active {
                    ui.painter().line_segment(
                        [response.rect.left_bottom(), response.rect.right_bottom()],
                        egui::Stroke::new(2.0, self.accent()),
                    );
                }
                if response.clicked() {
                    clicked = Some(*tab);
                }
            }
        });
        if let Some(tab) = clicked {
            self.property_focus = None;
            let mut next = self.preferences.properties_tabs.clone();
            next.insert(kind.id().to_owned(), tab.id().to_owned());
            let value = serde_json::to_value(next).map_err(|_| Error::Invalid("tabs"));
            match value.and_then(|value| self.preferences.set("properties_tabs", value)) {
                Ok(()) => {}
                Err(error) => self.error = Some(error.to_string()),
            }
        }
        ui.separator();
        ui.add_space(theme::space::MD);
        egui::ScrollArea::vertical()
            .id_salt("inspector.properties")
            .show(ui, |ui| self.properties_tab(ui, item, kind, active));
    }
    pub(in crate::editor) fn properties_tab(
        &mut self,
        ui: &mut egui::Ui,
        item: ItemId,
        kind: ElementKind,
        tab: PropertiesTab,
    ) {
        match tab {
            PropertiesTab::Text if self.item_value(item, TEXT_SIZE_SCRUB.field).is_some() => {
                for field in shared_fields(self.session.project(), &self.selection) {
                    match field.id {
                        "text_size" => self.scrub_field(ui, item, TEXT_SIZE_SCRUB),
                        "text_color" => self.text_color_field(ui, item),
                        "text_content" | "text_font" | "text_background" | "text_position_x"
                        | "text_position_y" => {
                            self.shared_field_controls(ui, std::iter::once(field));
                        }
                        _ => {}
                    }
                }
                self.color_button(
                    ui,
                    "inspector.text_color.picker",
                    Text::ColorPicker,
                    ColorTarget::Text(item),
                );
                self.color_button(
                    ui,
                    "inspector.background_color",
                    Text::BackgroundColor,
                    ColorTarget::Background(item),
                );
            }
            PropertiesTab::Transform => {
                for field in project::TransformField::ALL {
                    self.scrub_field(ui, item, transform_scrub(field));
                }
            }
            PropertiesTab::Graphic => {
                self.color_button(
                    ui,
                    "inspector.graphic_fill",
                    Text::GraphicFill,
                    ColorTarget::GraphicFill(item),
                );
                self.color_button(
                    ui,
                    "inspector.graphic_stroke",
                    Text::GraphicStroke,
                    ColorTarget::GraphicStroke(item),
                );
            }
            PropertiesTab::Masks => {
                let masks = self
                    .session
                    .project()
                    .data()
                    .scene()
                    .timeline
                    .iter()
                    .flat_map(|t| &t.items)
                    .find(|i| i.id == item)
                    .map(|i| i.masks.iter().map(|m| m.id.clone()).collect::<Vec<_>>())
                    .unwrap_or_default();
                if masks.is_empty() {
                    self.properties_stub(ui);
                }
                for mask in masks {
                    self.color_button(
                        ui,
                        &format!("inspector.mask.{mask}.stroke"),
                        Text::GraphicStroke,
                        ColorTarget::MaskStroke { item, mask },
                    );
                }
            }
            PropertiesTab::Audio if kind == ElementKind::Audio => {
                self.scrub_field(ui, item, AUDIO_GAIN_SCRUB);
                if let Some(field) = shared_fields(self.session.project(), &self.selection)
                    .into_iter()
                    .find(|field| field.id == "audio_muted")
                {
                    self.audio_mute_control(ui, &field);
                }
            }
            PropertiesTab::Speed if matches!(kind, ElementKind::Video | ElementKind::Audio) => {
                self.speed_controls(ui, item);
            }
            PropertiesTab::Blending => {
                self.blend_control(ui, item);
                self.scrub_field(ui, item, OPACITY_SCRUB);
            }
            PropertiesTab::Effects
                if matches!(
                    kind,
                    ElementKind::Video
                        | ElementKind::Image
                        | ElementKind::Sticker
                        | ElementKind::Graphic
                ) =>
            {
                self.effects_stack(ui, item);
            }
            _ => self.properties_stub(ui),
        }
    }
    pub(in crate::editor) fn properties_stub(&mut self, ui: &mut egui::Ui) {
        ui.label(
            egui::RichText::new(self.tr(Text::PanelStub))
                .small()
                .color(self.secondary()),
        );
    }
}

mod speed;

mod effects;

mod appearance;

mod number_edit;

mod selection;

use super::transform_scrub;

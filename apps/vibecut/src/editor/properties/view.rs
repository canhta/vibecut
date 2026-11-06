//! Renders the properties area: a 42 px tab strip, then collapsible sections of rows.

use super::{ColorTarget, Editor, Rect, Text, Vec2, catalog, egui, theme};
use crate::kit;

mod dialog;
pub(in crate::editor) use dialog::show_dialog;

use super::{
    AUDIO_GAIN_SCRUB, ElementKind, Error, ItemId, OPACITY_SCRUB, PropertiesTab, SelectionScope,
    TEXT_SIZE_SCRUB, shared_fields,
};

fn is_basic(tab: PropertiesTab) -> bool {
    matches!(
        tab,
        PropertiesTab::Transform | PropertiesTab::Blending | PropertiesTab::Masks
    )
}

/// Content inset from the panel edge, measured.
const INSET: f32 = 12.0;

pub(in crate::editor) fn show(editor: &mut Editor, ui: &mut egui::Ui) {
    // The area draws its own strip, so it fills the slot the frame measured.
    let area = editor
        .panels
        .get("inspector")
        .copied()
        .unwrap_or_else(|| ui.max_rect());
    let mut ui = ui.new_child(
        egui::UiBuilder::new()
            .id_salt("inspector.area")
            .max_rect(area),
    );
    ui.set_clip_rect(area);
    ui.spacing_mut().item_spacing = Vec2::ZERO;
    if !editor.details && editor.selection.len() == 1 {
        editor.properties(&mut ui, area);
        return;
    }
    editor.property_focus = None;
    // Without a single element selected the strip switches the project views.
    let ids = [
        "inspector.project".to_owned(),
        "inspector.details".to_owned(),
    ];
    let labels = [editor.tr(Text::Project), editor.tr(Text::Details)];
    let selected = usize::from(editor.details);
    let tabs = kit::text_tabs(&mut ui, &ids, &labels, selected);
    for (id, response) in ids.iter().zip(&tabs) {
        editor.remember(id.clone(), response);
        if response.clicked() {
            editor.action(id);
        }
    }
    let mut body = body_ui(&mut ui, area);
    if editor.details {
        project_details(editor, &mut body);
    } else if editor.selection.is_empty() {
        editor.panel_section(
            &mut body,
            "inspector.section.canvas",
            "inspector.canvas",
            Text::Canvas,
            true,
            false,
            |editor, ui| {
                editor.color_row(
                    ui,
                    "inspector.canvas_background",
                    Text::CanvasBackground,
                    ColorTarget::Canvas,
                );
            },
        );
    } else {
        egui::ScrollArea::vertical()
            .id_salt("inspector.shared")
            .show(&mut body, |ui| {
                editor.panel_section(
                    ui,
                    "inspector.section.selection",
                    "inspector.selection",
                    Text::Selection,
                    true,
                    false,
                    |editor, ui| editor.shared_inspector(ui),
                );
            });
    }
}

fn body_ui(ui: &mut egui::Ui, area: Rect) -> egui::Ui {
    let rect = Rect::from_min_max(
        egui::pos2(area.left() + INSET, area.top() + theme::size::TAB_STRIP),
        egui::pos2(area.right() - INSET, area.bottom()),
    );
    let mut body = ui.new_child(egui::UiBuilder::new().max_rect(rect));
    body.set_clip_rect(rect.intersect(ui.clip_rect()));
    body.spacing_mut().item_spacing = Vec2::ZERO;
    body
}

fn project_details(editor: &mut Editor, ui: &mut egui::Ui) {
    editor.panel_section(
        ui,
        "inspector.section.canvas",
        "inspector.canvas",
        Text::Canvas,
        true,
        false,
        |editor, ui| {
            let locale = editor.preferences.locale;
            let data = editor.session.project().data();
            let width = catalog::number(locale, u64::from(data.canvas.width));
            let height = catalog::number(locale, u64::from(data.canvas.height));
            let numerator = catalog::signed(locale, data.canvas.fps.num());
            let denominator = catalog::signed(locale, data.canvas.fps.den());
            let rate = catalog::number(locale, u64::from(data.settings.audio_sample_rate));
            let lines = [
                catalog::format(
                    locale,
                    Text::CanvasSize,
                    &[("width", &width), ("height", &height)],
                ),
                catalog::format(
                    locale,
                    Text::FrameRate,
                    &[("num", &numerator), ("den", &denominator)],
                ),
                catalog::format(locale, Text::SampleRate, &[("rate", &rate)]),
            ];
            for line in lines {
                detail_row(editor, ui, &line);
            }
        },
    );
    editor.panel_section(
        ui,
        "inspector.section.metadata",
        "inspector.metadata",
        Text::Metadata,
        false,
        false,
        |editor, ui| {
            let locale = editor.preferences.locale;
            let created = catalog::date(locale, editor.session.metadata().created_unix_ms);
            let modified = catalog::date(locale, editor.session.metadata().modified_unix_ms);
            for line in [
                catalog::format(locale, Text::CreatedDate, &[("date", &created)]),
                catalog::format(locale, Text::ModifiedDate, &[("date", &modified)]),
            ] {
                detail_row(editor, ui, &line);
            }
        },
    );
}

fn detail_row(editor: &Editor, ui: &mut egui::Ui, text: &str) {
    ui.allocate_ui_with_layout(
        Vec2::new(ui.available_width(), theme::size::PROPERTY_ROW),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            ui.label(
                egui::RichText::new(text)
                    .size(theme::type_size::BODY)
                    .color(editor.secondary()),
            );
        },
    );
}

impl Editor {
    /// A collapsible section: header with caret and optional reset, rows below.
    /// Returns whether reset was clicked; the open state lives in preferences.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::editor) fn panel_section(
        &mut self,
        ui: &mut egui::Ui,
        id: &'static str,
        key: &'static str,
        label: Text,
        default_open: bool,
        reset: bool,
        body: impl FnOnce(&mut Self, &mut egui::Ui),
    ) -> bool {
        let stored = self.preferences.section_open(key, default_open);
        let mut open = stored;
        let label = self.tr(label);
        let reset_label = self.tr(Text::ResetValue);
        let header =
            kit::section_header_with(ui, id, label, &mut open, reset.then_some(reset_label), None);
        self.remember(id, &header.toggle);
        let clicked = header.reset.as_ref().is_some_and(|response| {
            self.remember(format!("{id}.reset"), response);
            response.clicked()
        });
        if open != stored
            && let Err(error) = self.set_section_open(key, open)
        {
            self.error = Some(error.to_string());
        }
        if open {
            body(self, ui);
        }
        clicked
    }

    /// One selected element: its type's tab set, remembered per element type.
    pub(in crate::editor) fn properties(&mut self, ui: &mut egui::Ui, area: Rect) {
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
        // Transform, blending and masks share one top tab with segmented sub-tabs.
        let basic: Vec<PropertiesTab> = tabs.iter().copied().filter(|tab| is_basic(*tab)).collect();
        let group_label = match kind {
            ElementKind::Video => Text::Video,
            ElementKind::Image => Text::Image,
            _ => Text::Basic,
        };
        let mut top: Vec<Option<PropertiesTab>> = Vec::new();
        for tab in &tabs {
            if !is_basic(*tab) {
                top.push(Some(*tab));
            } else if !top.contains(&None) {
                top.push(None);
            }
        }
        let ids: Vec<String> = top
            .iter()
            .map(|entry| match entry {
                Some(tab) => format!("inspector.tab.{}", tab.id()),
                None => "inspector.group.basic".to_owned(),
            })
            .collect();
        let labels: Vec<&str> = top
            .iter()
            .map(|entry| match entry {
                Some(tab) => self.tr(tab.label()),
                None => self.tr(group_label),
            })
            .collect();
        let selected = top
            .iter()
            .position(|entry| match entry {
                Some(tab) => *tab == active,
                None => is_basic(active),
            })
            .unwrap_or(0);
        let responses = kit::text_tabs(ui, &ids, &labels, selected);
        let mut clicked = None;
        for ((id, response), entry) in ids.iter().zip(&responses).zip(&top) {
            self.remember(id.clone(), response);
            if response.clicked() {
                clicked = match entry {
                    Some(tab) => Some(*tab),
                    None if is_basic(active) => None,
                    None => basic.first().copied(),
                };
            }
        }
        let mut body = body_ui(ui, area);
        let mut sub_clicked = None;
        if is_basic(active) && basic.len() > 1 {
            body.add_space(13.0);
            let sub_ids: Vec<String> = basic
                .iter()
                .map(|tab| format!("inspector.tab.{}", tab.id()))
                .collect();
            let sub_labels: Vec<&str> = basic.iter().map(|tab| self.tr(tab.label())).collect();
            let at = basic.iter().position(|tab| *tab == active).unwrap_or(0);
            let cells = kit::segmented(&mut body, &sub_ids, &sub_labels, at);
            for ((id, response), tab) in sub_ids.iter().zip(&cells).zip(&basic) {
                self.remember(id.clone(), response);
                if response.clicked() {
                    sub_clicked = Some(*tab);
                }
            }
            body.add_space(12.0);
        }
        if let Some(tab) = clicked.or(sub_clicked) {
            self.property_focus = None;
            let mut next = self.preferences.properties_tabs.clone();
            next.insert(kind.id().to_owned(), tab.id().to_owned());
            let value = serde_json::to_value(next).map_err(|_| Error::Invalid("tabs"));
            match value.and_then(|value| self.preferences.set("properties_tabs", value)) {
                Ok(()) => {}
                Err(error) => self.error = Some(error.to_string()),
            }
        }
        if matches!(
            self.selection_scopes.active(&self.selection),
            SelectionScope::MaskPoints | SelectionScope::Keyframes
        ) {
            let scope = match self.selection_scopes.active(&self.selection) {
                SelectionScope::MaskPoints => Text::MaskPoints,
                _ => Text::Keyframes,
            };
            body.label(
                egui::RichText::new(self.tr(scope))
                    .size(theme::type_size::BODY)
                    .color(self.secondary()),
            );
        }
        egui::ScrollArea::vertical()
            .id_salt("inspector.properties")
            .auto_shrink([false, false])
            .show(&mut body, |ui| self.properties_tab(ui, item, kind, active));
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
                self.panel_section(
                    ui,
                    "inspector.section.text",
                    "inspector.text",
                    Text::TextItem,
                    true,
                    false,
                    |editor, ui| {
                        for field in shared_fields(editor.session.project(), &editor.selection) {
                            match field.id {
                                "text_size" => editor.scrub_field(ui, item, TEXT_SIZE_SCRUB),
                                "text_color" => editor.text_color_field(ui, item),
                                "text_content" | "text_font" | "text_background"
                                | "text_position_x" | "text_position_y" => {
                                    editor.shared_field_controls(ui, std::iter::once(field));
                                }
                                _ => {}
                            }
                        }
                        editor.color_row(
                            ui,
                            "inspector.text_color.picker",
                            Text::ColorPicker,
                            ColorTarget::Text(item),
                        );
                        editor.color_row(
                            ui,
                            "inspector.background_color",
                            Text::BackgroundColor,
                            ColorTarget::Background(item),
                        );
                    },
                );
            }
            PropertiesTab::Transform => {
                let reset = self.panel_section(
                    ui,
                    "inspector.section.transform",
                    "inspector.transform",
                    Text::Transform,
                    true,
                    true,
                    |editor, ui| {
                        use project::TransformField as F;
                        let mut scale = transform_scrub(F::ScaleX);
                        if editor.uniform_scale {
                            scale.label = Text::Scale;
                        }
                        editor.slider_row(ui, item, scale);
                        let (row, mut value_ui) =
                            editor.row_value_ui(ui, theme::size::PROPERTY_ROW);
                        editor.row_label(ui, row, editor.tr(Text::UniformScale));
                        let mut uniform = editor.uniform_scale;
                        let width = value_ui.available_width();
                        value_ui.add_space((width - theme::size::TOGGLE_WIDTH).max(0.0));
                        let response = kit::toggle(
                            &mut value_ui,
                            "inspector.uniform_scale",
                            &mut uniform,
                            editor.tr(Text::UniformScale),
                        );
                        editor.remember("inspector.uniform_scale", &response);
                        editor.uniform_scale = uniform;
                        if !uniform {
                            editor.slider_row(ui, item, transform_scrub(F::ScaleY));
                        }
                        editor.number_row(
                            ui,
                            item,
                            Text::Position,
                            &[
                                (transform_scrub(F::PositionX), Some("X")),
                                (transform_scrub(F::PositionY), Some("Y")),
                            ],
                        );
                        editor.scrub_field(ui, item, transform_scrub(F::Rotate));
                    },
                );
                if reset {
                    self.property_drafts.clear();
                    self.property_focus = None;
                    for field in project::TransformField::ALL {
                        if let Err(error) = self.set_item_field(
                            item,
                            field.path(),
                            &field.default_value().to_string(),
                        ) {
                            self.error = Some(error.to_string());
                        }
                    }
                }
            }
            PropertiesTab::Graphic => {
                self.panel_section(
                    ui,
                    "inspector.section.graphic",
                    "inspector.graphic",
                    Text::Graphic,
                    true,
                    false,
                    |editor, ui| {
                        editor.color_row(
                            ui,
                            "inspector.graphic_fill",
                            Text::GraphicFill,
                            ColorTarget::GraphicFill(item),
                        );
                        editor.color_row(
                            ui,
                            "inspector.graphic_stroke",
                            Text::GraphicStroke,
                            ColorTarget::GraphicStroke(item),
                        );
                    },
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
                if !masks.is_empty() {
                    self.panel_section(
                        ui,
                        "inspector.section.masks",
                        "inspector.masks",
                        Text::Masks,
                        true,
                        false,
                        |editor, ui| {
                            for mask in masks {
                                editor.color_row(
                                    ui,
                                    &format!("inspector.mask.{mask}.stroke"),
                                    Text::GraphicStroke,
                                    ColorTarget::MaskStroke { item, mask },
                                );
                            }
                        },
                    );
                }
            }
            PropertiesTab::Audio if kind == ElementKind::Audio => {
                self.panel_section(
                    ui,
                    "inspector.section.audio",
                    "inspector.audio",
                    Text::Audio,
                    true,
                    false,
                    |editor, ui| {
                        editor.scrub_field(ui, item, AUDIO_GAIN_SCRUB);
                        if let Some(field) =
                            shared_fields(editor.session.project(), &editor.selection)
                                .into_iter()
                                .find(|field| field.id == "audio_muted")
                        {
                            editor.audio_mute_control(ui, &field);
                        }
                    },
                );
            }
            PropertiesTab::Speed if matches!(kind, ElementKind::Video | ElementKind::Audio) => {
                self.panel_section(
                    ui,
                    "inspector.section.speed",
                    "inspector.speed",
                    Text::Speed,
                    true,
                    false,
                    |editor, ui| editor.speed_controls(ui, item),
                );
            }
            PropertiesTab::Blending => {
                self.panel_section(
                    ui,
                    "inspector.section.blending",
                    "inspector.blending",
                    Text::Blending,
                    true,
                    false,
                    |editor, ui| {
                        editor.blend_control(ui, item);
                        editor.slider_row(ui, item, OPACITY_SCRUB);
                    },
                );
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
            _ => {}
        }
    }
}

mod speed;

mod effects;

mod appearance;

mod number_edit;

mod selection;

use super::transform_scrub;

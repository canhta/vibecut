//! Text style rows of the Text tab: weight, slant, line, alignment, spacing and the
//! background box geometry. Every edit goes through the registered item field.
use super::Panel;
use super::number_edit::Layout;
use crate::editor::properties::{
    ColorTarget, Id, ItemId, ScrubSpec, TEXT_SIZE_SCRUB, Text, TextPart, egui, read, shared_fields,
    theme,
};
use crate::kit;

const fn spec(field: &'static str, label: Text, min: f64, max: f64, decimals: usize) -> ScrubSpec {
    ScrubSpec {
        field,
        label,
        min,
        max,
        step: if decimals == 0 { 1.0 } else { 0.1 },
        pixels_per_step: 2.0,
        decimals,
        suffix: "",
        scale: 1.0,
    }
}

const LETTER_SPACING: ScrubSpec = spec(
    "text_letter_spacing",
    Text::TextLetterSpacing,
    -(project::MAX_LETTER_SPACING as f64),
    project::MAX_LETTER_SPACING as f64,
    1,
);
const LINE_HEIGHT: ScrubSpec = spec(
    "text_line_height",
    Text::TextLineHeight,
    0.0,
    project::MAX_LINE_HEIGHT as f64,
    0,
);
const PADDING_X: ScrubSpec = spec(
    "text_background_padding_x",
    Text::BackgroundPaddingX,
    0.0,
    project::TextBackground::MAX_EXTENT as f64,
    0,
);
const PADDING_Y: ScrubSpec = spec(
    "text_background_padding_y",
    Text::BackgroundPaddingY,
    0.0,
    project::TextBackground::MAX_EXTENT as f64,
    0,
);
const OFFSET_X: ScrubSpec = spec(
    "text_background_offset_x",
    Text::BackgroundOffsetX,
    -(project::TextBackground::MAX_OFFSET as f64),
    project::TextBackground::MAX_OFFSET as f64,
    0,
);
const OFFSET_Y: ScrubSpec = spec(
    "text_background_offset_y",
    Text::BackgroundOffsetY,
    -(project::TextBackground::MAX_OFFSET as f64),
    project::TextBackground::MAX_OFFSET as f64,
    0,
);
const RADIUS: ScrubSpec = spec(
    "text_background_corner_radius",
    Text::BackgroundRadius,
    0.0,
    project::TextBackground::MAX_EXTENT as f64,
    0,
);

impl Panel<'_, '_> {
    /// Segmented choice row; returns the clicked option index.
    fn choice_row(
        &mut self,
        ui: &mut egui::Ui,
        id: &str,
        label: Text,
        options: &[(&str, Text)],
        selected: usize,
    ) -> Option<usize> {
        let (row, mut value_ui) = self.row_value_ui(ui, theme::size::PROPERTY_ROW);
        self.row_label(ui, row, self.tr(label));
        let ids: Vec<String> = options
            .iter()
            .map(|(option, _)| format!("{id}.{option}"))
            .collect();
        let labels: Vec<&str> = options.iter().map(|(_, text)| self.tr(*text)).collect();
        let cells = kit::segmented(&mut value_ui, &ids, &labels, selected);
        let mut clicked = None;
        for (index, (cell_id, response)) in ids.iter().zip(&cells).enumerate() {
            let response = if id.starts_with("text.animation") {
                response
                    .clone()
                    .on_hover_text(self.tr(Text::TextAnimationTip))
            } else {
                response.clone()
            };
            self.remember(cell_id.clone(), &response);
            if response.clicked() && index != selected {
                clicked = Some(index);
            }
        }
        clicked
    }

    /// Format: one row of size, bold, italic, underline and alignment, then the line
    /// decoration, letter spacing and line height.
    pub(super) fn text_format_controls(&mut self, ui: &mut egui::Ui, item: ItemId) {
        let Some(project::ItemParams::Text(params)) =
            read::item(self.cx.session.project(), item).map(|found| found.content.clone())
        else {
            return;
        };
        use project::TextDecoration;
        self.format_row(ui, item, &params);
        let lines = [
            TextDecoration::None,
            TextDecoration::Underline,
            TextDecoration::LineThrough,
        ];
        let at = lines
            .iter()
            .position(|d| *d == params.decoration)
            .unwrap_or(0);
        if let Some(choice) = self.choice_row(
            ui,
            "inspector.text_decoration",
            Text::TextDecoration,
            &[
                ("none", Text::DecorationNone),
                ("underline", Text::DecorationUnderline),
                ("line_through", Text::DecorationStrike),
            ],
            at,
        ) && let Some(word) = ["none", "underline", "line_through"].get(choice)
        {
            self.set_item_field(item, "text_decoration", word);
        }
        self.scrub_field(ui, item, LETTER_SPACING);
        self.scrub_field(ui, item, LINE_HEIGHT);
    }

    /// The Format row: a size field with its drag grip, then bold, italic and underline
    /// toggles and the three alignment cells, all on one property row.
    fn format_row(&mut self, ui: &mut egui::Ui, item: ItemId, params: &project::TextParams) {
        use project::{TextAlign, TextDecoration};
        let (row, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), theme::size::PROPERTY_ROW),
            egui::Sense::hover(),
        );
        let mut cells = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(row)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        cells.spacing_mut().item_spacing.x = theme::space::XXS;
        let size =
            super::super::read::item_value(self.cx.session.project(), item, TEXT_SIZE_SCRUB.field)
                .unwrap_or(f64::from(params.font_size));
        let (grip, _) =
            cells.allocate_exact_size(egui::vec2(12.0, theme::size::FIELD), egui::Sense::hover());
        self.scrub_handle(&mut cells, item, TEXT_SIZE_SCRUB, size, grip);
        self.numeric_edit(
            &mut cells,
            item,
            TEXT_SIZE_SCRUB,
            size,
            Layout {
                prefix: None,
                width: 54.0,
                gap: 4.0,
            },
        );
        let bold = params.weight >= 600;
        if self.glyph_toggle(
            &mut cells,
            "inspector.text_weight",
            Text::TextBoldGlyph,
            Text::WeightBold,
            bold,
        ) {
            self.set_item_field(item, "text_weight", if bold { "400" } else { "700" });
        }
        let italic = params.italic;
        if self.glyph_toggle(
            &mut cells,
            "inspector.text_italic",
            Text::TextItalicGlyph,
            Text::SlantItalic,
            italic,
        ) {
            self.set_item_field(item, "text_italic", if italic { "false" } else { "true" });
        }
        let underline = params.decoration == TextDecoration::Underline;
        if self.glyph_toggle(
            &mut cells,
            "inspector.text_underline",
            Text::TextUnderlineGlyph,
            Text::DecorationUnderline,
            underline,
        ) {
            self.set_item_field(
                item,
                "text_decoration",
                if underline { "none" } else { "underline" },
            );
        }
        for (word, label, icon, align) in [
            (
                "left",
                Text::AlignLeft,
                kit::Icon::AlignLeft,
                TextAlign::Left,
            ),
            (
                "center",
                Text::AlignCenter,
                kit::Icon::AlignCenterHorizontal,
                TextAlign::Center,
            ),
            (
                "right",
                Text::AlignRight,
                kit::Icon::AlignRight,
                TextAlign::Right,
            ),
        ] {
            let response = kit::icon_button(
                &mut cells,
                &format!("inspector.text_align.{word}"),
                icon,
                self.tr(label),
                params.align == align,
            );
            if response.clicked() {
                self.set_item_field(item, "text_align", word);
            }
        }
    }

    /// A two-state glyph button (B, I, U); returns whether it was clicked.
    fn glyph_toggle(
        &mut self,
        ui: &mut egui::Ui,
        id: &str,
        glyph: Text,
        label: Text,
        on: bool,
    ) -> bool {
        let button = egui::Button::new(egui::RichText::new(self.tr(glyph)).strong()).selected(on);
        let response = ui
            .add_sized([theme::size::FIELD, theme::size::FIELD], button)
            .on_hover_text(self.tr(label));
        self.remember(id, &response);
        response.clicked()
    }

    /// The text items among the selection, in timeline order.
    fn selected_texts(&self) -> Vec<ItemId> {
        self.cx
            .session
            .project()
            .data()
            .scene()
            .timeline
            .iter()
            .flat_map(|track| &track.items)
            .filter(|item| {
                self.cx.selection().contains(&item.id)
                    && matches!(item.content, project::ItemParams::Text(_))
            })
            .map(|item| item.id)
            .collect()
    }

    /// The Style rows every text selection shares: the fill swatch, the background box, the
    /// outline and the shadow (each a toggle with a swatch) and the glow switch. A row writes
    /// every selected text in one edit, and a mixed value shows no swatch.
    pub(super) fn text_style_rows(&mut self, ui: &mut egui::Ui) {
        let items = self.selected_texts();
        if items.is_empty() {
            return;
        }
        let fields = shared_fields(self.cx.session.project(), self.cx.selection());
        let field = |id: &str| fields.iter().find(|field| field.id == id);
        let swatch = |part| ColorTarget::Texts {
            items: items.clone(),
            part,
        };
        self.color_row(
            ui,
            "inspector.text_color.picker",
            Text::TextFill,
            swatch(TextPart::Fill),
        );
        if let Some(background) = field("text_background") {
            self.text_background_toggle(ui, background);
            if background.value.as_deref().is_some_and(|v| v != "none") {
                self.color_row(
                    ui,
                    "inspector.background_color",
                    Text::BackgroundColor,
                    swatch(TextPart::Background),
                );
            }
        }
        if let Some(stroke) = field("text_stroke") {
            self.text_effect_toggle(
                ui,
                stroke,
                Text::TextStroke,
                "inspector.text_stroke",
                "#000000FF",
            );
            if stroke.value.as_deref().is_some_and(|v| v != "none") {
                self.color_row(
                    ui,
                    "inspector.text_stroke.picker",
                    Text::ColorPicker,
                    swatch(TextPart::Stroke),
                );
            }
        }
        if let Some(shadow) = field("text_shadow") {
            self.text_effect_toggle(
                ui,
                shadow,
                Text::TextShadow,
                "inspector.text_shadow",
                "#000000A0",
            );
            if shadow.value.as_deref().is_some_and(|v| v != "none") {
                self.color_row(
                    ui,
                    "inspector.text_shadow.picker",
                    Text::ColorPicker,
                    swatch(TextPart::Shadow),
                );
            }
        }
        if let Some(glow) = field("text_glow") {
            let mut on = glow.value.as_deref() == Some("true");
            let (row, mut value_ui) = self.row_value_ui(ui, theme::size::PROPERTY_ROW);
            self.row_label(ui, row, self.tr(Text::TextGlow));
            let width = value_ui.available_width();
            value_ui.add_space((width - theme::size::TOGGLE_WIDTH).max(0.0));
            let response = kit::toggle(
                &mut value_ui,
                "inspector.text_glow",
                &mut on,
                self.tr(Text::TextGlow),
            );
            self.remember("inspector.text_glow", &response);
            if response.clicked() {
                self.set_shared_field("text_glow", if on { "true" } else { "false" });
            }
        }
    }

    /// A checkbox row for an outline or shadow: on sets `on_value`, off clears it.
    fn text_effect_toggle(
        &mut self,
        ui: &mut egui::Ui,
        field: &crate::editor::properties::SharedField,
        label: Text,
        id: &str,
        on_value: &str,
    ) {
        let mut checked = field.value.as_deref().is_some_and(|value| value != "none");
        let (row, mut value_ui) = self.row_value_ui(ui, theme::size::PROPERTY_ROW);
        self.row_label(ui, row, self.tr(label));
        let response = value_ui
            .add(egui::Checkbox::without_text(&mut checked).indeterminate(field.value.is_none()));
        self.remember(id, &response);
        if response.changed() {
            self.set_shared_field(field.id, if checked { on_value } else { "none" });
        }
    }

    /// Style: the saved style bank, then the background box geometry.
    pub(super) fn text_style_controls(&mut self, ui: &mut egui::Ui, item: ItemId) {
        let Some(project::ItemParams::Text(params)) =
            read::item(self.cx.session.project(), item).map(|found| found.content.clone())
        else {
            return;
        };
        let enabled = !self.cx.disabled.contains("text.animation.fade_in");
        self.text_preset_controls(ui, item, &params, enabled);
        // The box rows exist only while the box does, like the colour fields they sit beside.
        if params.background.is_some() {
            self.number_row(
                ui,
                item,
                Text::BackgroundPadding,
                &[(PADDING_X, Some("X")), (PADDING_Y, Some("Y"))],
            );
            self.number_row(
                ui,
                item,
                Text::BackgroundOffset,
                &[(OFFSET_X, Some("X")), (OFFSET_Y, Some("Y"))],
            );
            self.scrub_field(ui, item, RADIUS);
        }
    }

    /// One effect row and one direction row apply a combined in/out animation.
    pub(super) fn text_animation_controls(
        &mut self,
        ui: &mut egui::Ui,
        item: ItemId,
        enabled: bool,
    ) {
        use engine::TextAnimationPreset::{
            FadeIn, FadeInOut, FadeOut, ScaleIn, ScaleInOut, ScaleOut, SlideIn, SlideInOut,
            SlideOut,
        };
        const ALL: [engine::TextAnimationPreset; 9] = [
            FadeIn, FadeOut, FadeInOut, SlideIn, SlideOut, SlideInOut, ScaleIn, ScaleOut,
            ScaleInOut,
        ];
        let project = self.cx.session.project();
        let matched = read::item(project, item).and_then(|target| {
            ALL.iter()
                .position(|preset| preset.matches(target, project.data()))
        });
        let (family, direction) =
            matched.map_or((usize::MAX, usize::MAX), |index| (index / 3, index % 3));
        let fallback = matched.map_or((0, 0), |index| (index / 3, index % 3));
        let mut chosen = None;
        ui.add_enabled_ui(enabled, |ui| {
            if let Some(index) = self.choice_row(
                ui,
                "text.animation.family",
                Text::TextAnimation,
                &[
                    ("fade", Text::TextAnimationFade),
                    ("slide", Text::TextAnimationSlide),
                    ("scale", Text::TextAnimationScale),
                ],
                family,
            ) {
                chosen = Some(index * 3 + fallback.1);
            }
            let family_options = matched.map_or(0, |index| index / 3);
            let options = match family_options {
                1 => [
                    ("slide_in", Text::TextAnimationIn),
                    ("slide_out", Text::TextAnimationOut),
                    ("slide_in_out", Text::TextAnimationBoth),
                ],
                2 => [
                    ("scale_in", Text::TextAnimationIn),
                    ("scale_out", Text::TextAnimationOut),
                    ("scale_in_out", Text::TextAnimationBoth),
                ],
                _ => [
                    ("fade_in", Text::TextAnimationIn),
                    ("fade_out", Text::TextAnimationOut),
                    ("fade_in_out", Text::TextAnimationBoth),
                ],
            };
            if let Some(index) = self.choice_row(
                ui,
                "text.animation",
                Text::TextAnimationDirection,
                &options,
                direction,
            ) {
                chosen = Some(fallback.0 * 3 + index);
            }
        });
        if let Some(preset) = chosen.and_then(|index| ALL.get(index)).copied() {
            self.actions
                .push(crate::editor::properties::Action::ApplyTextAnimation {
                    item,
                    preset,
                    snapshot: self.cx.session.project().clone(),
                });
        }
    }

    /// The saved style bank: choose one to copy onto this item, or save its look.
    fn text_preset_controls(
        &mut self,
        ui: &mut egui::Ui,
        item: ItemId,
        params: &project::TextParams,
        enabled: bool,
    ) {
        let snapshot = self.cx.session.project().clone();
        let bank = snapshot.data().text_presets.clone();
        let current = bank
            .iter()
            .find(|preset| preset.style == project::TextStyle::from_params(params))
            .map(|preset| preset.id);
        let current_name = current
            .and_then(|id| bank.iter().find(|preset| preset.id == id))
            .map_or_else(
                || self.tr(Text::TextPresetNone),
                |preset| preset.name.as_str(),
            );
        let mut chosen = None;
        let (row, mut value_ui) = self.row_value_ui(ui, theme::size::PROPERTY_ROW);
        self.row_label(ui, row, self.tr(Text::TextPreset));
        value_ui.spacing_mut().interact_size.y = theme::size::FIELD;
        let combo = value_ui
            .add_enabled_ui(enabled, |ui| {
                egui::ComboBox::from_id_salt(Id::unique(format!(
                    "inspector.text_preset.{}.{}",
                    item.0,
                    std::sync::Arc::as_ptr(&snapshot) as usize
                )))
                .selected_text(current_name)
                .width(96.0)
                .show_ui(ui, |ui| {
                    for preset in &bank {
                        let response =
                            ui.selectable_label(current == Some(preset.id), preset.name.as_str());
                        self.remember(
                            format!("inspector.text_preset.saved.{}", preset.id.0),
                            &response,
                        );
                        if response.clicked() {
                            chosen = Some(preset.id);
                            ui.close();
                        }
                    }
                })
                .response
            })
            .inner;
        // The row label is painted beside the combo; name the combo itself too.
        combo.widget_info(|| {
            egui::WidgetInfo::labeled(egui::Role::ComboBox, enabled, self.tr(Text::TextPreset))
        });
        self.remember("inspector.text_preset", &combo);
        let response = value_ui
            .add_enabled_ui(enabled, |ui| {
                ui.add(egui::Button::new(self.tr(Text::TextPresetSave)))
            })
            .inner;
        self.remember("inspector.text_preset.save", &response);
        if let Some(preset) = chosen {
            self.actions
                .push(crate::editor::properties::Action::ApplyTextPreset {
                    item,
                    preset,
                    snapshot: snapshot.clone(),
                });
        }
        if response.clicked() {
            self.actions
                .push(crate::editor::properties::Action::SaveTextPreset { item, snapshot });
        }
    }
}

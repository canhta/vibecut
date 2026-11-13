//! Text style rows of the Text tab: weight, slant, line, alignment, spacing and the
//! background box geometry. Every edit goes through the registered item field.
use super::Panel;
use crate::editor::properties::{Id, ItemId, ScrubSpec, Text, egui, read, theme};
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

    /// Format: weight, slant, decoration, alignment, letter spacing and line height.
    pub(super) fn text_format_controls(&mut self, ui: &mut egui::Ui, item: ItemId) {
        let Some(project::ItemParams::Text(params)) =
            read::item(self.cx.session.project(), item).map(|found| found.content.clone())
        else {
            return;
        };
        use project::{TextAlign, TextDecoration};
        let bold = params.weight >= 600;
        if let Some(choice) = self.choice_row(
            ui,
            "inspector.text_weight",
            Text::TextWeight,
            &[("normal", Text::WeightNormal), ("bold", Text::WeightBold)],
            usize::from(bold),
        ) {
            let weight = if choice == 1 { "700" } else { "400" };
            self.set_item_field(item, "text_weight", weight);
        }
        if let Some(choice) = self.choice_row(
            ui,
            "inspector.text_italic",
            Text::TextSlant,
            &[("normal", Text::SlantNormal), ("italic", Text::SlantItalic)],
            usize::from(params.italic),
        ) {
            self.set_item_field(
                item,
                "text_italic",
                if choice == 1 { "true" } else { "false" },
            );
        }
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
        let aligns = [TextAlign::Left, TextAlign::Center, TextAlign::Right];
        let at = aligns.iter().position(|a| *a == params.align).unwrap_or(1);
        if let Some(choice) = self.choice_row(
            ui,
            "inspector.text_align",
            Text::TextAlign,
            &[
                ("left", Text::AlignLeft),
                ("center", Text::AlignCenter),
                ("right", Text::AlignRight),
            ],
            at,
        ) && let Some(word) = ["left", "center", "right"].get(choice)
        {
            self.set_item_field(item, "text_align", word);
        }
        self.scrub_field(ui, item, LETTER_SPACING);
        self.scrub_field(ui, item, LINE_HEIGHT);
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

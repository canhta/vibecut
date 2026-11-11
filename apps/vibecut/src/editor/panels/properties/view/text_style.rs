//! Text style rows of the Text tab: weight, slant, line, alignment, spacing and the
//! background box geometry. Every edit goes through the registered item field.
use super::Panel;
use crate::editor::properties::{ItemId, ScrubSpec, Text, egui, read, theme};
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
            let response = if id == "text.animation" {
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

    pub(super) fn text_style_controls(&mut self, ui: &mut egui::Ui, item: ItemId) {
        let Some(project::ItemParams::Text(params)) =
            read::item(self.cx.session.project(), item).map(|found| found.content.clone())
        else {
            return;
        };
        use engine::TextAnimationPreset::{FadeIn, FadeInOut, FadeOut};
        let presets = [FadeIn, FadeOut, FadeInOut];
        let project = self.cx.session.project();
        let selected = read::item(project, item)
            .and_then(|target| {
                presets
                    .iter()
                    .position(|preset| preset.matches(target, project.data().canvas.fps))
            })
            .unwrap_or(usize::MAX);
        let enabled = !self.cx.disabled.contains("text.animation.fade_in");
        let choice = ui
            .add_enabled_ui(enabled, |ui| {
                self.choice_row(
                    ui,
                    "text.animation",
                    Text::TextAnimation,
                    &[
                        ("fade_in", Text::TextAnimationIn),
                        ("fade_out", Text::TextAnimationOut),
                        ("fade_in_out", Text::TextAnimationBoth),
                    ],
                    selected,
                )
            })
            .inner;
        if let Some(preset) = choice.and_then(|i| presets.get(i)).copied() {
            self.actions
                .push(crate::editor::properties::Action::ApplyTextAnimation {
                    item,
                    preset,
                    snapshot: self.cx.session.project().clone(),
                });
        }
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
}

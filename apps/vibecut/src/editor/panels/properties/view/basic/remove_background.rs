//! Draws the Remove BG sub-tab: the Auto removal checkbox with its feather and analysis state, then
//! the Chroma key switch and its keyed controls.
use super::super::Panel;
use crate::editor::properties::{
    Action, ColorTarget, Icon, ItemId, ScrubSpec, Text, egui, read, theme,
};
use crate::kit;
use engine::footage::CorrectionKind;

/// Distance between the centres of neighbouring tool icons, measured in the reference.
const TOOL_PITCH: f32 = 48.0;

impl Panel<'_, '_> {
    /// A property row led by its checkbox, the label to its right (reference 100332: the box
    /// at the row's left edge, the label 4 px after it). The caller reads `on` and the click.
    fn checkbox_row(&self, ui: &mut egui::Ui, label: Text, on: &mut bool) -> egui::Response {
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
        cells.add(egui::Checkbox::new(
            on,
            egui::RichText::new(self.tr(label))
                .size(theme::type_size::BODY)
                .color(theme::text(self.cx.dark)),
        ))
    }

    /// The Auto removal switch row, then, while it is on, the edge feather slider and the
    /// analysis state. Turning the switch on keeps the model's own edge and starts the person
    /// matte analysis; a warm cache finishes at once.
    fn auto_removal_rows(&mut self, ui: &mut egui::Ui, item: ItemId) {
        let Some(project::ItemParams::Video(params)) =
            read::item(self.cx.session.project(), item).map(|item| &item.content)
        else {
            return;
        };
        let mut on = params.background_removal.is_some();
        let response = self.checkbox_row(ui, Text::AutoRemoval, &mut on);
        self.remember("inspector.auto_removal", &response);
        if response.clicked() {
            if on {
                self.set_item_field(item, "background_removal", "on");
                self.actions.push(Action::AnalyzeFootage {
                    item,
                    kind: CorrectionKind::PersonMatte,
                });
            } else {
                self.set_item_field(item, "background_removal", "off");
            }
        }
        if !on {
            return;
        }
        self.slider_row(
            ui,
            item,
            ScrubSpec::of("background_removal_feather", Text::EdgeFeather, 1., 2., 0),
        );
        self.correction_status(
            ui,
            item,
            CorrectionKind::PersonMatte,
            "inspector.auto_removal",
        );
        self.matte_brush_rows(ui, item);
        ui.add_space(theme::space::MD);
    }

    /// The Custom removal section: a label, the brush and eraser tools, then Size. The tools
    /// are toggles: the armed one paints with the player drags (Brush adds the person,
    /// Eraser removes it) and clicking it again, Escape or a selection change leaves it. The
    /// two Pro tools of the reference are model-backed selection tools and are not drawn.
    /// Strokes render as they are painted, so there is no Apply step; Size is view state.
    fn matte_brush_rows(&mut self, ui: &mut egui::Ui, item: ItemId) {
        let armed = self
            .cx
            .matte_brush
            .filter(|brush| brush.item == item)
            .map(|brush| brush.mode);
        let mut custom = armed.is_some();
        let response = self.checkbox_row(ui, Text::CustomRemoval, &mut custom);
        self.remember("inspector.custom_removal", &response);
        if response.clicked() {
            self.actions.push(Action::SetMatteBrush(custom.then_some(
                crate::editor::MatteBrush {
                    item,
                    mode: project::MatteStrokeMode::Add,
                },
            )));
        }
        let tools = [
            (
                project::MatteStrokeMode::Add,
                "inspector.matte_brush.add",
                Icon::PaintBrush,
                Text::MatteBrushAdd,
            ),
            (
                project::MatteStrokeMode::Remove,
                "inspector.matte_brush.remove",
                Icon::Eraser,
                Text::MatteBrushRemove,
            ),
        ];
        ui.horizontal(|ui| {
            // The reference tool pitch is 48 px between icon centres.
            ui.spacing_mut().item_spacing.x = TOOL_PITCH - theme::size::CONTROL;
            for (mode, id, icon, label) in tools {
                let selected = armed == Some(mode);
                if self.icon_only(ui, id, icon, self.tr(label), Some(selected), true) {
                    self.actions.push(Action::SetMatteBrush(
                        (!selected).then_some(crate::editor::MatteBrush { item, mode }),
                    ));
                }
            }
        });
        self.matte_size_row(ui);
    }

    pub(in super::super) fn remove_background_controls(&mut self, ui: &mut egui::Ui, item: ItemId) {
        self.auto_removal_rows(ui, item);
        let project = self.cx.session.project();
        let existing =
            read::chroma_key(project, item).map(|effect| (effect.id.clone(), effect.enabled));
        let staged = self.state.chroma_staged == Some(item);
        let on = existing.as_ref().is_some_and(|(_, enabled)| *enabled) || staged;
        let mut value = on;
        let (row, mut value_ui) = self.row_value_ui(ui, theme::size::PROPERTY_ROW);
        self.row_label(ui, row, self.tr(Text::ChromaKey));
        let width = value_ui.available_width();
        value_ui.add_space((width - theme::size::TOGGLE_WIDTH).max(0.0));
        let response = kit::toggle(
            &mut value_ui,
            "inspector.chroma_key.toggle",
            &mut value,
            self.tr(Text::ChromaKey),
        );
        self.remember("inspector.chroma_key.toggle", &response);
        if response.clicked() {
            self.actions
                .push(Action::ToggleChromaKey { item, enabled: !on });
        }
        let Some((effect, enabled)) = existing else {
            if staged {
                ui.add_space(theme::space::MD);
                self.eyedropper_row(ui, item, None, false);
            }
            return;
        };
        if !enabled {
            return;
        }
        ui.add_space(theme::space::MD);
        self.color_row(
            ui,
            "inspector.chroma_key.color",
            Text::KeyColor,
            ColorTarget::EffectColor {
                item,
                effect: effect.clone(),
                key: "key_color".into(),
                time: time::Tick::ZERO,
            },
        );
        let picked = read::chroma_key_color(project, item, &effect);
        let default = project::CHROMA_KEY_COLOR_DEFAULT;
        self.eyedropper_row(
            ui,
            item,
            Some(effect.as_str()),
            picked.is_some_and(|color| color != default),
        );
        ui.add_space(theme::space::MD);
        let Some(definition) = project::effect_definitions()
            .iter()
            .find(|definition| definition.id == "chroma_key")
        else {
            return;
        };
        let at = self.effect_time(item);
        for parameter in definition.params {
            if parameter.id == "key_color" {
                continue;
            }
            let Some(current) = read::effect_param_value_at(
                self.cx.session.project(),
                item,
                &effect,
                parameter.id,
                at,
            ) else {
                continue;
            };
            self.effect_scrub_field(ui, item, &effect, "chroma_key", parameter, current);
        }
    }

    /// The reference's eyedropper and reset row; it is the only control while
    /// the switch is on but no colour has been picked yet.
    fn eyedropper_row(
        &mut self,
        ui: &mut egui::Ui,
        item: ItemId,
        effect: Option<&str>,
        can_reset: bool,
    ) {
        let armed = self
            .cx
            .key_color_pick
            .as_ref()
            .is_some_and(|pick| pick.item == item && pick.effect.as_deref() == effect);
        let (_, mut value_ui) = self.row_value_ui(ui, theme::size::PROPERTY_ROW);
        if self.icon_only(
            &mut value_ui,
            "inspector.chroma_key.eyedropper",
            Icon::Eye,
            self.tr(Text::PickKeyColor),
            Some(armed),
            true,
        ) {
            self.actions.push(Action::SetKeyColorPick((!armed).then(|| {
                crate::editor::KeyPick {
                    item,
                    effect: effect.map(str::to_owned),
                }
            })));
        }
        value_ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if self.icon_only(
                ui,
                "inspector.chroma_key.reset",
                Icon::ArrowCounterClockwise,
                self.tr(Text::ResetValue),
                None,
                can_reset,
            ) && let Some(effect) = effect
            {
                self.actions.push(Action::ResetKeyColor {
                    item,
                    effect: effect.to_owned(),
                });
            }
        });
    }
}

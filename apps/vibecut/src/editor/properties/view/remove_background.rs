//! Draws the Remove BG sub-tab: the Chroma key toggle and its keyed controls.
use super::Panel;
use crate::editor::properties::{Action, ColorTarget, Icon, ItemId, Text, egui, read, theme};
use crate::kit;

impl Panel<'_, '_> {
    pub(super) fn remove_background_controls(&mut self, ui: &mut egui::Ui, item: ItemId) {
        let project = self.cx.session.project();
        let existing =
            read::chroma_key(project, item).map(|effect| (effect.id.clone(), effect.enabled));
        let mut on = existing.as_ref().is_some_and(|(_, enabled)| *enabled);
        let (row, mut value_ui) = self.row_value_ui(ui, theme::size::PROPERTY_ROW);
        self.row_label(ui, row, self.tr(Text::ChromaKey));
        let width = value_ui.available_width();
        value_ui.add_space((width - theme::size::TOGGLE_WIDTH).max(0.0));
        let response = kit::toggle(
            &mut value_ui,
            "inspector.chroma_key.toggle",
            &mut on,
            self.tr(Text::ChromaKey),
        );
        self.remember("inspector.chroma_key.toggle", &response);
        if response.clicked() {
            self.actions.push(Action::ToggleChromaKey {
                item,
                enabled: !existing.as_ref().is_some_and(|(_, enabled)| *enabled),
            });
        }
        let Some((effect, enabled)) = existing else {
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
            ColorTarget::EffectKeyColor {
                item,
                effect: effect.clone(),
            },
        );
        let armed = self
            .cx
            .key_color_pick
            .as_ref()
            .is_some_and(|pick| pick.item == item && pick.effect == effect);
        let default = project::chroma_key_color(project::CHROMA_KEY_COLOR_DEFAULT).ok();
        let picked = read::chroma_key_color(project, item, &effect);
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
                    effect: effect.clone(),
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
                picked != default,
            ) {
                self.actions.push(Action::ResetKeyColor {
                    item,
                    effect: effect.clone(),
                });
            }
        });
        ui.add_space(theme::space::MD);
        let Some(definition) = project::effect_definitions()
            .iter()
            .find(|definition| definition.kind == "chroma_key")
        else {
            return;
        };
        let at = self.effect_time(item);
        for parameter in definition.params {
            if parameter.key == "key_color" {
                continue;
            }
            let Some(current) = read::effect_param_value_at(
                self.cx.session.project(),
                item,
                &effect,
                parameter.key,
                at,
            ) else {
                continue;
            };
            self.effect_scrub_field(ui, item, &effect, parameter, current);
        }
    }
}

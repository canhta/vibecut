//! Draws the Remove BG sub-tab: the Chroma key switch and its keyed controls.
use super::Panel;
use crate::editor::properties::{Action, ColorTarget, Icon, ItemId, Text, egui, read, theme};
use crate::kit;

impl Panel<'_, '_> {
    pub(super) fn remove_background_controls(&mut self, ui: &mut egui::Ui, item: ItemId) {
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
            self.effect_scrub_field(ui, item, &effect, parameter, current);
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

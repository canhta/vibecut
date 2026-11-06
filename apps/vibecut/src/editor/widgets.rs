//! Records editor controls for automation.

use super::{
    BUTTON_ICON, Color32, Editor, Icon, Text, Vec2, WidgetState, actions, catalog, egui, icons,
    theme,
};

impl Editor {
    pub(in crate::editor) fn tr(&self, key: Text) -> &'static str {
        catalog::text(self.preferences.locale, key)
    }
    pub(in crate::editor) fn remember(&mut self, id: impl Into<String>, response: &egui::Response) {
        self.remember_with_icon(id, None, response);
    }
    pub(in crate::editor) fn remember_with_icon(
        &mut self,
        id: impl Into<String>,
        icon: Option<&'static str>,
        response: &egui::Response,
    ) {
        let id = id.into();
        response
            .ctx
            .accesskit_node_builder(response.id, |node| node.set_author_id(&id));
        self.widgets.insert(
            id.clone(),
            WidgetState {
                id,
                enabled: response.enabled(),
                focused: response.has_focus(),
                rect: response.rect,
                icon,
                tooltip: None,
            },
        );
    }
    pub(in crate::editor) fn button(
        &mut self,
        ui: &mut egui::Ui,
        id: &'static str,
        icon: Icon,
        key: Text,
        enabled: bool,
    ) -> bool {
        let enabled = enabled && actions::action(id).is_none_or(|_| self.action_enabled(id));
        let color = if enabled {
            self.text_color()
        } else {
            theme::disabled(self.preferences.dark)
        };
        let image = icons::tinted(icon, BUTTON_ICON, color);
        let label = egui::RichText::new(self.tr(key)).color(color);
        let response = ui
            .push_id(id, |ui| {
                ui.add_enabled(
                    enabled,
                    egui::Button::image_and_text(image, label)
                        .min_size(Vec2::new(theme::size::CONTROL, theme::size::FIELD)),
                )
            })
            .inner;
        self.remember_with_icon(id, Some(icon.name()), &response);
        response.clicked()
    }
    /// Text-only is reserved for the properties tabs by the iconography contract.
    pub(in crate::editor) fn text_button(
        &mut self,
        ui: &mut egui::Ui,
        id: &'static str,
        key: Text,
    ) -> bool {
        let response = ui.push_id(id, |ui| ui.button(self.tr(key))).inner;
        self.remember(id, &response);
        response.clicked()
    }
    pub(in crate::editor) fn preset_button(
        &mut self,
        ui: &mut egui::Ui,
        id: &str,
        label: &str,
        selected: bool,
    ) -> bool {
        let response = ui
            .push_id(id, |ui| ui.selectable_label(selected, label))
            .inner;
        self.remember(id, &response);
        response.clicked()
    }

    pub(in crate::editor) fn menu_button(
        &mut self,
        ui: &mut egui::Ui,
        id: &str,
        key: Text,
        enabled: bool,
    ) -> bool {
        let response = ui.add_enabled(enabled, egui::Button::new(self.tr(key)));
        self.remember(id, &response);
        response.clicked()
    }
    /// A property section whose open state lives in the preference store, with
    /// the same stable automation id across locale and layout changes.
    pub(in crate::editor) fn section<R>(
        &mut self,
        ui: &mut egui::Ui,
        id: &'static str,
        key: &'static str,
        label: Text,
        default_open: bool,
        body: impl FnOnce(&mut Self, &mut egui::Ui) -> R,
    ) -> Option<R> {
        let state_id = ui.make_persistent_id(("preferences.section", key));
        let stored = self.preferences.section_open(key, default_open);
        let mut state = egui::collapsing_header::CollapsingState::load_with_default_open(
            ui.ctx(),
            state_id,
            default_open,
        );
        if state.is_open() != stored {
            state.set_open(stored);
            state.store(ui.ctx());
        }
        let label = self.tr(label);
        let mut clicked = false;
        let mut header = state.show_header(ui, |ui| {
            let response = ui.add(egui::Label::new(label).sense(egui::Sense::click()));
            clicked = response.clicked();
        });
        if clicked {
            header.toggle();
        }
        let open = header.is_open();
        let (toggle, _, body_response) = header.body(|ui| body(self, ui));
        if open != stored
            && let Err(error) = self.set_section_open(key, open)
        {
            self.error = Some(error.to_string());
        }
        toggle
            .ctx
            .accesskit_node_builder(toggle.id, |node| node.set_author_id(id));
        self.remember(id, &toggle);
        body_response.map(|response| response.inner)
    }
    #[allow(clippy::too_many_arguments)]
    pub(in crate::editor) fn icon_only(
        &mut self,
        ui: &mut egui::Ui,
        id: &str,
        icon: Icon,
        label: &str,
        shortcut: Option<&str>,
        selected: Option<bool>,
        enabled: bool,
    ) -> bool {
        let enabled = enabled && actions::action(id).is_none_or(|_| self.action_enabled(id));
        let locale = self.preferences.locale;
        let tooltip = shortcut.map_or_else(
            || label.to_owned(),
            |key| {
                catalog::format(
                    locale,
                    Text::ActionShortcut,
                    &[("action", label), ("shortcut", key)],
                )
            },
        );
        let is_selected = selected.unwrap_or(false);
        let response = ui
            .add_enabled_ui(enabled, |ui| {
                crate::kit::icon_button(ui, id, icon, &tooltip, is_selected)
            })
            .inner;
        response.widget_info(|| {
            if let Some(selected) = selected {
                egui::WidgetInfo::selected(egui::Role::CheckBox, enabled, selected, &tooltip)
            } else {
                egui::WidgetInfo::labeled(egui::Role::Button, enabled, &tooltip)
            }
        });
        self.remember_with_icon(id, Some(icon.name()), &response);
        if let Some(state) = self.widgets.get_mut(id) {
            state.tooltip = Some(tooltip);
        }
        enabled && response.clicked()
    }
    pub(in crate::editor) fn accent(&self) -> Color32 {
        theme::accent(self.preferences.dark)
    }
    pub(in crate::editor) fn panel_fill(&self) -> Color32 {
        theme::panel(self.preferences.dark)
    }
    pub(in crate::editor) fn control_fill(&self) -> Color32 {
        theme::field(self.preferences.dark)
    }
    pub(in crate::editor) fn border(&self) -> Color32 {
        theme::border(self.preferences.dark)
    }
    pub(in crate::editor) fn text_color(&self) -> Color32 {
        theme::text(self.preferences.dark)
    }
    pub(in crate::editor) fn secondary(&self) -> Color32 {
        theme::secondary(self.preferences.dark)
    }
}

//! Records editor controls for automation.

use super::{BTreeMap, Rect, egui};

#[derive(Clone, Debug)]
pub struct WidgetState {
    pub id: String,
    pub enabled: bool,
    pub focused: bool,
    pub rect: Rect,
    /// Embedded Phosphor name for a button that carries an icon.
    pub icon: Option<&'static str>,
    /// Action and shortcut shown for icon-only controls.
    pub tooltip: Option<String>,
}

/// The controls one editor frame painted, keyed by their stable author id.
#[derive(Default)]
pub(crate) struct Widgets {
    states: BTreeMap<String, WidgetState>,
}

impl Widgets {
    pub(in crate::editor) fn clear(&mut self) {
        self.states.clear();
    }

    pub(in crate::editor) fn values(&self) -> impl Iterator<Item = &WidgetState> {
        self.states.values()
    }

    pub(in crate::editor) fn get_mut(&mut self, id: &str) -> Option<&mut WidgetState> {
        self.states.get_mut(id)
    }

    pub(in crate::editor) fn contains_key(&self, id: &str) -> bool {
        self.states.contains_key(id)
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
        self.states.insert(
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
}

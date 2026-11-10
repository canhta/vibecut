//! Renders the colour window.
use super::Panel;
use crate::editor::properties::{Action, egui};

impl Panel<'_, '_> {
    /// Draw the open picker and turn its events into previews and commits.
    pub(super) fn picker_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut draft) = self.state.picker.take() else {
            return;
        };
        if let crate::editor::properties::ColorTarget::EffectColor {
            item,
            effect,
            key,
            time,
        } = &draft.target
        {
            if key != "key_color"
                && (super::super::read::item_locked(self.cx.session.project(), *item)
                    || super::super::read::item(self.cx.session.project(), *item)
                        .is_none_or(|i| self.cx.playhead.saturating_sub(i.start) != *time)
                    || !self
                        .state
                        .widgets
                        .iter()
                        .any(|w| w.id == format!("effects.{effect}.param.{key}")))
            {
                self.state.color_base = None;
                self.actions.push(Action::SyncPreview);
                return;
            }
        }
        let picker = &mut draft.picker;
        let event = picker.ui(ctx, self.cx.locale, &self.cx.preferences.recent_colors);
        for (id, response) in event.widgets {
            self.remember(id, &response);
        }
        if matches!(&draft.target, crate::editor::properties::ColorTarget::EffectColor { key, .. } if key != "key_color")
        {
            picker.color.0[3] = picker.original.0[3];
        }
        if event.cancel {
            self.state.color_base = None;
            self.actions.push(Action::SyncPreview);
            return;
        }
        if (!event.preview && !event.commit) || picker.color == picker.original {
            if event.preview {
                self.actions.push(Action::SyncPreview);
            }
            self.state.picker = Some(draft);
            return;
        }
        match draft
            .target
            .command(self.cx.session.project(), picker.color)
        {
            Ok(command) => {
                if event.commit && picker.color != picker.original {
                    let mut recent = self.cx.preferences.recent_colors.clone();
                    recent.retain(|c| *c != picker.color);
                    recent.insert(0, picker.color);
                    recent.truncate(16);
                    self.actions.push(Action::ColorCommit { command, recent });
                } else if event.preview {
                    self.actions.push(Action::ColorPreview { command });
                }
            }
            Err(error) => {
                self.fail(error);
                self.actions.push(Action::SyncPreview);
            }
        }
        self.state.picker = Some(draft);
    }
}

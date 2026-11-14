//! Renders the colour window and routes the font popover.
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
            && key != "key_color"
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
        let picker = &mut draft.picker;
        let event = picker.ui(
            ctx,
            self.cx.locale,
            &self.cx.session.project().data().recent_colors,
        );
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
                    self.actions.push(Action::ColorCommit {
                        command,
                        record: Some(picker.color),
                    });
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

impl Panel<'_, '_> {
    /// Route the font popover's widgets and requested catalog actions.
    pub(super) fn font_dialog(&mut self, ctx: &egui::Context) {
        if !self.state.font_picker.open {
            return;
        }
        // The row stores the field's right edge; open below it and let the
        // window's own constraint keep the popover inside the app frame.
        let anchor = self
            .state
            .font_picker
            .anchor
            .unwrap_or_else(|| egui::pos2(ctx.content_rect().left() + 24.0, 120.0));
        let event = crate::editor::properties::font_picker::show(
            &mut self.state.font_picker,
            ctx,
            self.cx.locale,
            self.cx.dark,
            &self.cx.preferences.font_picker,
            anchor,
        );
        for (id, response) in event.widgets {
            self.remember(id, &response);
        }
        for action in event.actions {
            self.actions.push(Action::Font(action));
        }
    }
}

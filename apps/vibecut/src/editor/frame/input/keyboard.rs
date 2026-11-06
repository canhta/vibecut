//! Routes editor keyboard events.

use crate::editor::{BlockedReason, Dispatch, DispatchContext, Editor, egui};

impl Editor {
    pub(in crate::editor) fn keyboard(&mut self, ctx: &egui::Context) {
        self.sync_keymap();
        if self.overlay_open || egui::Popup::is_any_open(ctx) {
            return;
        }
        if self.player.fullscreen {
            if consume(ctx, egui::Modifiers::NONE, egui::Key::Escape) {
                self.player.fullscreen = false;
            }
            return;
        }
        if self.player.timecode_edit.is_some() {
            if consume(ctx, egui::Modifiers::NONE, egui::Key::Escape) {
                self.cancel_timecode();
            }
            return;
        }
        if self.properties.picker.is_some() {
            return;
        }
        if self.media_delete_confirm.is_some() {
            if consume(ctx, egui::Modifiers::NONE, egui::Key::Escape) {
                self.media_delete_confirm = None;
            }
            return;
        }
        // An open sheet or dialog owns its own Escape handling and blocks every
        // global action. Text inputs are handled below so Escape still reaches
        // the focused field.
        if self.timeline.scenes.is_open() {
            if (self.timeline.scenes.is_confirming() || !self.timeline.scenes.is_renaming())
                && consume(ctx, egui::Modifiers::NONE, egui::Key::Escape)
            {
                self.timeline.scenes.escape();
            }
            return;
        }
        if self.timeline.marquee_active() || self.media_marquee.is_some() {
            if consume(ctx, egui::Modifiers::NONE, egui::Key::Escape) {
                self.cancel_interaction();
            }
            return;
        }
        if self.export.as_ref().is_some_and(|dialog| dialog.open) {
            return;
        }
        // The shortcuts dialog records the next chord itself and owns Escape.
        // Capture before panels so a recorded key cannot also press a focused
        // control in the same frame.
        if self.shortcuts_open {
            if self.shortcut_recording.is_some() {
                let _ = self.capture_shortcut(ctx);
            } else if consume(ctx, egui::Modifiers::NONE, egui::Key::Escape) {
                self.close_shortcuts();
            }
            return;
        }
        if self.timeline.renaming_track() && consume(ctx, egui::Modifiers::NONE, egui::Key::Escape)
        {
            self.timeline.cancel_track_rename();
            return;
        }
        let context = self.dispatch_context(ctx);
        let outcome = {
            let keymap = &self.keymap;
            ctx.input_mut(|input| keymap.take(input, context))
        };
        match outcome {
            Dispatch::Blocked(BlockedReason::TextInput) => {
                // The focused field sees Escape and blurs itself.
            }
            Dispatch::Blocked(_) | Dispatch::NoMatch | Dispatch::FellThrough(_) => {}
            Dispatch::Unavailable { action, .. } | Dispatch::Action { action, .. } => {
                self.action(action);
            }
        }
    }
    pub(in crate::editor) fn dispatch_context(&self, ctx: &egui::Context) -> DispatchContext {
        DispatchContext {
            overlay_open: self.subtitles.as_ref().is_some_and(|s| s.modal_open())
                || self.timeline.scenes.is_open()
                || self.shortcuts_open
                || self.export.as_ref().is_some_and(|dialog| dialog.open)
                || self.models_visible
                    && self
                        .models
                        .as_ref()
                        .is_some_and(|models| models.inspect().open),
            // There is no Editor while a project is loading; the workspace shows
            // its own progress surface and dispatches no actions.
            project_loading: false,
            recording: self.recording_active,
            text_input: self.subtitles.as_ref().is_some_and(|s| s.editing(ctx))
                || ctx.text_edit_focused()
                || self.frame.rename.is_some()
                || self.timeline.renaming_track()
                || self.timeline.scenes.is_renaming()
                || self.properties.editing(),
            clipboard_entry: self.clipboard_has_entry(),
        }
    }
    /// Clipboard payload and presence belong to p1-parity/20. Until that lands
    /// no entry exists, so Ctrl+V falls through to the media paste owner.
    pub(in crate::editor) fn clipboard_has_entry(&self) -> bool {
        false
    }
    pub fn set_recording_active(&mut self, active: bool) {
        self.recording_active = active;
    }
    pub fn recording_active(&self) -> bool {
        self.recording_active
    }
}

fn consume(ctx: &egui::Context, modifiers: egui::Modifiers, key: egui::Key) -> bool {
    ctx.input_mut(|input| input.consume_key(modifiers, key))
}

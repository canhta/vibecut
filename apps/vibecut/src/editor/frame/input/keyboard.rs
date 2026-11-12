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
        if self.properties.picker_open() {
            return;
        }
        // The armed eyedropper is a one-click mode: Escape leaves it without
        // also deselecting the clip whose panel opened it.
        if self.player.key_color_pick.is_some() {
            if consume(ctx, egui::Modifiers::NONE, egui::Key::Escape) {
                self.player.key_color_pick = None;
            }
            return;
        }
        if self.assets.media_delete_confirm.is_some() {
            if consume(ctx, egui::Modifiers::NONE, egui::Key::Escape) {
                self.cancel_media_delete();
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
        if self.timeline.bookmark_popover().is_some() {
            if consume(ctx, egui::Modifiers::NONE, egui::Key::Escape) {
                self.timeline.escape_bookmark();
            }
            return;
        }
        if self.timeline.marquee_active() || self.assets.media_marquee.is_some() {
            if consume(ctx, egui::Modifiers::NONE, egui::Key::Escape) {
                self.cancel_interaction();
            }
            return;
        }
        if self.dialogs.export.is_open() {
            return;
        }
        // The shortcuts dialog records the next chord itself and owns Escape.
        // Capture before panels so a recorded key cannot also press a focused
        // control in the same frame.
        if self.frame.shortcuts_open {
            if self.frame.shortcut_recording.is_some() {
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
            let keymap = &self.frame.keymap;
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
                || self.frame.shortcuts_open
                || self.dialogs.export.is_open()
                || self.dialogs.models.is_open()
                || self.dialogs.preferences.open,
            // There is no Editor while a project is loading; the workspace shows
            // its own progress surface and dispatches no actions.
            project_loading: false,
            recording: self.frame.recording_active,
            text_input: self.subtitles.as_ref().is_some_and(|s| s.editing(ctx))
                || ctx.text_edit_focused()
                || self.frame.rename.is_some()
                || self.timeline.renaming_track()
                || self.timeline.scenes.is_renaming()
                || self.properties.editing(),
            clipboard_entry: self.clipboard_has_entry(),
        }
    }
    /// Presence belongs to this editor's Session, shared with attached control.
    pub(in crate::editor) fn clipboard_has_entry(&self) -> bool {
        self.session.clipboard_has_entry()
    }
    pub fn set_recording_active(&mut self, active: bool) {
        self.frame.recording_active = active;
    }
    pub fn recording_active(&self) -> bool {
        self.frame.recording_active
    }
}

fn consume(ctx: &egui::Context, modifiers: egui::Modifiers, key: egui::Key) -> bool {
    ctx.input_mut(|input| input.consume_key(modifiers, key))
}

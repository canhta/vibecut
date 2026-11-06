//! Routes the actions an area returns to the owners that apply them.
use crate::editor::{
    Command, Context, CreatedId, Editor, ItemId, Result, Tick, egui, frame, timeline,
};

impl Editor {
    /// The read-only services every area receives.
    pub(super) fn context(&self) -> Context<'_> {
        Context {
            session: &self.session,
            selection: &self.selection,
            locale: self.preferences.locale,
            dark: self.preferences.dark,
            save_status: self.save_status(),
            playhead: self.playback.position().unwrap_or(Tick::ZERO),
            ripple: self.ripple,
            preferences: &self.preferences,
            playback: &self.playback,
            keybindings: &self.preferences.keybindings,
            scope: self.selection_scopes.active(&self.selection),
            thumbnails: frame::Thumbnails {
                images: &self.thumbnails,
                textures: &self.thumbnail_textures,
            },
            disabled: std::collections::BTreeSet::new(),
            effect_targets: None,
        }
    }
    /// Run `paint` with the timeline state and its context, then route what it returns.
    fn with_timeline(
        &mut self,
        paint: impl FnOnce(&mut timeline::State, &Context<'_>) -> Vec<timeline::Action>,
    ) {
        let mut state = std::mem::take(&mut self.timeline);
        let mut cx = self.context();
        cx.disabled = timeline::TOOLBAR_ACTIONS
            .iter()
            .copied()
            .filter(|id| crate::actions::action(id).is_some() && !self.action_enabled(id))
            .collect();
        cx.effect_targets = self.effect_drag.is_some().then(|| {
            self.session
                .project()
                .data()
                .scene()
                .timeline
                .iter()
                .flat_map(|track| track.items.iter().map(|item| item.id))
                .filter(|id| self.effect_refusal(*id).is_none())
                .collect()
        });
        let actions = paint(&mut state, &cx);
        drop(cx);
        let widgets = state.take_widgets();
        self.timeline = state;
        for widget in widgets {
            self.remember_with_icon(widget.id.clone(), widget.icon, &widget.response);
            if let (Some(tooltip), Some(state)) = (widget.tooltip, self.widgets.get_mut(&widget.id))
            {
                state.tooltip = Some(tooltip);
            }
        }
        self.route_timeline(actions);
    }
    /// Paint the timeline panel and apply its requests.
    pub(super) fn timeline_panel(&mut self, ui: &mut egui::Ui) {
        self.with_timeline(|state, cx| timeline::show(state, cx, ui));
        if let Err(error) = self.remember_view() {
            self.error = Some(error.to_string());
        }
    }
    /// Paint the scene sheet and apply its requests.
    pub(super) fn timeline_scenes(&mut self, ctx: &egui::Context) {
        self.with_timeline(|state, cx| timeline::show_scenes(state, cx, ctx));
    }
    fn route_timeline(&mut self, actions: Vec<timeline::Action>) {
        use timeline::Action;
        for action in actions {
            match action {
                Action::Activate(id) => self.action(&id),
                Action::Command { id, params } => self.track_command(id, params),
                Action::RenameScene { scene, name } => {
                    if let Err(error) = self.session.execute(
                        "scene.rename",
                        serde_json::json!({"scene":scene,"name":name}),
                    ) {
                        self.error = Some(error.to_string());
                        self.timeline.keep_scene_rename(scene, name);
                    }
                }
                Action::Seek(at) => self.seek_to(at),
                Action::Select { item, modifiers } => self.select_item(item, modifiers),
                Action::Marquee(items) => self.selection = items,
                Action::SettleSelection => self.selection_changed(),
                Action::ClearSelection => {
                    self.selection.clear();
                    self.selection_changed();
                }
                Action::BeginGesture { select } => self.begin_drag_gesture(select),
                Action::Preview(command) => self.preview_gesture(*command),
                Action::MoveToTrack {
                    item,
                    start,
                    target,
                } => self.move_to_track(item, start, target),
                Action::EndGesture => self.end_drag_gesture(),
                Action::CancelInteraction => self.cancel_interaction(),
                Action::Fail(message) => self.error = Some(message),
            }
        }
    }
    /// Open the preview gesture a pointer drag edits inside.
    fn begin_drag_gesture(&mut self, select: Option<ItemId>) {
        if let Some(item) = select {
            self.select_single(item);
        }
        self.session.end_merge();
        if self.session.transaction_active()
            && let Err(error) = self.session.cancel_gesture()
        {
            self.error = Some(error.to_string());
            self.timeline.abandon_gesture();
            return;
        }
        if let Err(error) = self
            .session
            .set_selection(self.selection.clone())
            .and_then(|_| self.session.begin_gesture())
        {
            self.error = Some(error.to_string());
            self.timeline.abandon_gesture();
        }
    }
    fn preview_gesture(&mut self, command: Command) {
        // A refused or cancelled gesture must not turn its previews into edits.
        if !self.session.gesture_active() {
            return;
        }
        match self.session.apply(command) {
            Ok(outcome) if outcome.changed => self.sync_preview(),
            Ok(_) => {}
            Err(error) => self.error = Some(error.to_string()),
        }
    }
    fn move_to_track(&mut self, item: ItemId, start: Tick, target: timeline::DropTarget) {
        if !self.session.gesture_active() {
            return;
        }
        let moved = (|| -> Result<()> {
            let track = match target {
                timeline::DropTarget::Track(track) => track,
                timeline::DropTarget::New { kind, placement } => {
                    let outcome = self.session.apply(Command::AddTrack {
                        name: format!("{kind:?}").into(),
                        kind,
                        placement,
                    })?;
                    match outcome.created.first() {
                        Some(CreatedId::Track(id)) => *id,
                        _ => return Err(engine::EditError::Invalid("track allocation").into()),
                    }
                }
            };
            self.session
                .apply(Command::MoveItem { item, track, start })?;
            Ok(())
        })();
        if let Err(error) = moved {
            // The release already ended the pointer drag, so only the gesture is left to undo.
            self.error = Some(error.to_string());
            self.discard_gesture();
            self.timeline.abandon_gesture();
        }
    }
    /// Publish the open gesture as one undo step.
    fn end_drag_gesture(&mut self) {
        if self.session.gesture_active() {
            match self.session.commit_gesture() {
                Ok(outcome) if outcome.changed => self.sync_preview(),
                Ok(_) => {}
                Err(error) => self.error = Some(error.to_string()),
            }
        }
        self.prune_selection();
    }
    /// Discard the open gesture and show the project it started from.
    fn discard_gesture(&mut self) {
        if self.session.gesture_active() {
            match self.session.cancel_gesture() {
                Ok(outcome) if outcome.changed => self.sync_preview(),
                Ok(_) => {}
                Err(error) => self.error = Some(error.to_string()),
            }
        }
    }
    /// Escape: abandon the innermost pointer interaction in progress.
    pub(in crate::editor) fn cancel_interaction(&mut self) {
        if self.assets.transition_drag.take().is_some() {
            return;
        }
        if self.media_marquee.take().is_some() {
            self.media_selection.clear();
            self.media_anchor = None;
            self.session.end_merge();
            return;
        }
        if self.timeline.cancel_bookmark_drag() {
            self.discard_gesture();
            return;
        }
        if let Some(scrub) = self.properties.scrub.take() {
            let changed = scrub.last != scrub.start;
            if self.session.gesture_active()
                && let Err(error) = self.session.cancel_gesture()
            {
                self.error = Some(error.to_string());
            }
            self.properties.drafts.remove(&scrub.target.draft_key());
            self.properties.focus = None;
            if changed {
                self.sync_preview();
            }
            return;
        }
        if self.timeline.cancel_track_edit() {
            return;
        }
        if self.effect_drag.take().is_some() {
            return;
        }
        if self.properties.reorder.take().is_some() {
            return;
        }
        if self.timeline.cancel_drag() {
            self.discard_gesture();
        } else if let Some(before) = self.timeline.cancel_marquee() {
            self.selection = before;
            self.selection_changed();
        } else {
            self.selection_scopes.clear_specific(&mut self.selection);
            if let Err(e) = self.session.set_selection(self.selection.clone()) {
                self.error = Some(e.to_string());
            }
            if !self.media_selection.is_empty() {
                self.media_selection.clear();
                self.media_anchor = None;
                self.session.end_merge();
            }
        }
    }
    /// Step the timeline zoom, keeping the playhead anchored.
    pub(in crate::editor) fn zoom_timeline(&mut self, factor: f32) {
        let playhead = self.playback.position().unwrap_or(Tick::ZERO);
        let duration = self.session.project().duration();
        self.timeline.zoom_by(factor, duration, playhead);
    }
    /// Run an engine command on the current selection (track edits).
    pub(in crate::editor) fn track_command(&mut self, command: &str, params: serde_json::Value) {
        let before = self.session.project().clone();
        match self
            .session
            .set_selection(self.selection.clone())
            .and_then(|_| self.session.execute(command, params))
        {
            Ok(_) => {
                self.prune_selection();
                if !std::sync::Arc::ptr_eq(&before, self.session.project()) {
                    self.sync_preview();
                }
            }
            Err(error) => self.error = Some(error.to_string()),
        }
    }
}

#[cfg(test)]
mod tests;

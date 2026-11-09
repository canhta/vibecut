//! Routes the actions an area returns to the owners that apply them.
use crate::editor::{Command, Context, Editor, ItemId, Tick, egui, frame, timeline};

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
            ripple: self.preferences.ripple,
            preferences: &self.preferences,
            playback: &self.playback,
            keybindings: &self.preferences.keybindings,
            scope: self.selection_scopes.active(&self.selection),
            keyframes: &self.selection_scopes.keyframes,
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
                Action::SelectKeyframe {
                    keyframe,
                    modifiers,
                } => self.select_keyframe(keyframe, modifiers),
                Action::MarqueeKeyframes(keyframes) => {
                    self.selection_scopes.keyframes = keyframes;
                }
                Action::SettleKeyframes => {}
                Action::ClearKeyframes => self.selection_scopes.keyframes.clear(),
                Action::SetLaneOpen { item, open } => self.set_lane_open(item, open),
                Action::DeleteKeyframes(keyframes) => self.delete_keyframes(&keyframes),
                Action::ClearSelection => {
                    self.selection.clear();
                    self.selection_changed();
                }
                Action::BeginGesture { select } => self.begin_drag_gesture(select),
                Action::Preview(command) => self.preview_gesture(*command),
                Action::EndGesture => self.end_drag_gesture(),
                Action::BeginPreview { select } => self.begin_timeline_preview(select),
                Action::PreviewGesture(gesture) => self.preview_timeline(*gesture),
                Action::EndPreview => self.end_timeline_preview(),
                Action::CancelInteraction => self.cancel_interaction(),
                Action::Fail(message) => {
                    self.timeline.refuse(message.clone());
                    self.error = Some(message);
                }
            }
        }
    }
    /// A keyframe click scopes the selection without dropping the item context.
    fn select_keyframe(
        &mut self,
        keyframe: crate::selection::KeyframeSelection,
        modifiers: egui::Modifiers,
    ) {
        let toggle = modifiers.command || modifiers.ctrl || modifiers.shift;
        if toggle {
            if !self.selection_scopes.keyframes.remove(&keyframe) {
                self.selection_scopes.keyframes.insert(keyframe.clone());
            }
        } else {
            self.selection_scopes.keyframes.clear();
            self.selection_scopes.keyframes.insert(keyframe.clone());
        }
        self.selection.clear();
        self.selection.insert(keyframe.item);
        self.properties.clear_drafts();
        if let Err(error) = self.session.set_selection(self.selection.clone()) {
            self.error = Some(error.to_string());
        }
    }
    /// Persist one item's expanded lane choice for the owning project only.
    fn set_lane_open(&mut self, item: ItemId, open: bool) {
        self.timeline.set_lane(item, open);
        let project = self.session.project().data().id;
        let mut lanes = self.preferences.timeline_lanes.clone();
        let items: std::collections::BTreeSet<u64> = self
            .timeline
            .expanded_lanes()
            .into_iter()
            .map(|id| id.0)
            .collect();
        if items.is_empty() {
            lanes.remove(&project);
        } else {
            lanes.insert(project, items);
        }
        if let Err(error) = self.dispatch(
            crate::editor::PREFERENCE_COMMAND,
            serde_json::json!({"key":"timeline_lanes","value":lanes}),
        ) {
            self.error = Some(error.to_string());
        }
    }
    /// Delete every selected keyframe as one undo step.
    pub(in crate::editor) fn delete_keyframes(
        &mut self,
        keyframes: &std::collections::BTreeSet<crate::selection::KeyframeSelection>,
    ) {
        if keyframes.is_empty() {
            return;
        }
        let keys: Vec<engine::KeyframeRef> = keyframes
            .iter()
            .map(|key| engine::KeyframeRef {
                item: key.item,
                path: key.path.clone(),
                component: key.component,
                time: key.time,
            })
            .collect();
        let result = self.session.transaction(|session| {
            session.execute(
                "keyframes.delete_selection",
                serde_json::json!({"keys": keys}),
            )?;
            Ok(())
        });
        match result {
            Ok(()) => {
                self.selection_scopes.keyframes.clear();
                self.sync_preview();
            }
            Err(error) => self.error = Some(error.to_string()),
        }
    }

    /// Stage absolute transform values for one item through the shared preview contract.
    pub(in crate::editor) fn preview_transform(
        &mut self,
        item: ItemId,
        values: &[(project::TransformField, f64)],
    ) {
        let commands: Vec<serde_json::Value> = values
            .iter()
            .map(|(field, value)| {
                serde_json::json!({
                    "command":"timeline.set_item_param",
                    "params":{"item":item,"patch":{"field":field.path(),"value":value}}
                })
            })
            .collect();
        match self
            .session
            .execute("timeline.preview", serde_json::json!({"commands":commands}))
        {
            Ok(_) => self.sync_preview(),
            Err(error) => self.error = Some(error.to_string()),
        }
    }
    /// Close a transform gesture with one undo step, or restore its entry project.
    pub(in crate::editor) fn end_preview_transform(&mut self, commit: bool) {
        if !self.session.gesture_active() {
            return;
        }
        let command = if commit {
            "timeline.commit_preview"
        } else {
            "timeline.discard_preview"
        };
        match self.session.execute(command, serde_json::json!({})) {
            Ok(_) => self.sync_preview(),
            Err(error) => self.error = Some(error.to_string()),
        }
    }
    /// Select the text item whose inline session just opened and keep the
    /// timeline's preview contract as the only writer.
    pub(in crate::editor) fn begin_text_edit(&mut self, item: ItemId) {
        if self.session.gesture_active() {
            self.end_preview_transform(false);
        }
        self.select_single(item);
    }
    /// Stage the whole text buffer as an absolute `text_content` candidate; each
    /// call replaces the candidate from the session's opening snapshot.
    pub(in crate::editor) fn preview_text(&mut self, item: ItemId, content: &str) {
        let commands = [serde_json::json!({
            "command":"timeline.set_item_param",
            "params":{"item":item,"patch":{"field":"text_content","value":content}}
        })];
        match self
            .session
            .execute("timeline.preview", serde_json::json!({"commands":commands}))
        {
            Ok(_) => self.sync_preview(),
            Err(error) => self.error = Some(error.to_string()),
        }
    }
    /// Commit an open inline edit as one undo step, or discard a net-zero session.
    pub(in crate::editor) fn commit_text_edit(&mut self) -> bool {
        let Some(commit) = self.player.transform.cancel_text() else {
            return false;
        };
        self.end_preview_transform(commit);
        true
    }
    /// Escape during a preview transform drops the candidate without a history entry.
    fn cancel_preview_transform(&mut self) -> bool {
        if !self.player.transform.cancel() {
            return false;
        }
        if self.session.gesture_active() {
            match self
                .session
                .execute("timeline.discard_preview", serde_json::json!({}))
            {
                Ok(_) => self.sync_preview(),
                Err(error) => self.error = Some(error.to_string()),
            }
        }
        true
    }

    fn begin_drag_gesture(&mut self, select: Option<ItemId>) {
        if self.timeline.preview_armed() {
            self.discard_timeline_preview();
        }
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
        // Absolute pointer targets are rebuilt from the shared preview baseline,
        // so reversing a drag also reverses every ripple shift.
        let command = match command {
            command @ (Command::MoveItems { .. }
            | Command::TrimStart { .. }
            | Command::TrimEnd { .. }) => Command::TimelinePreview {
                commands: vec![command],
            },
            command => command,
        };
        let previous = self
            .session
            .preview_project()
            .unwrap_or(self.session.project())
            .clone();
        match self
            .session
            .apply_with_ripple(command, self.preferences.ripple)
        {
            Ok(_)
                if previous.as_ref()
                    != self
                        .session
                        .preview_project()
                        .unwrap_or(self.session.project())
                        .as_ref() =>
            {
                self.sync_preview()
            }
            Ok(_) => {}
            Err(error) => self.error = Some(error.to_string()),
        }
    }
    /// Arm the group contract: only the selection is applied before the first
    /// staged command, so `timeline.preview` can open its own gesture.
    fn begin_timeline_preview(&mut self, select: Option<ItemId>) {
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
        if let Err(error) = self.session.set_selection(self.selection.clone()) {
            self.error = Some(error.to_string());
            self.timeline.abandon_gesture();
            return;
        }
        self.timeline.arm_preview();
    }
    /// Stage one `timeline.move_items`/`timeline.trim`; each call replaces the
    /// whole candidate from the gesture entry snapshot.
    fn preview_timeline(&mut self, gesture: timeline::Gesture) {
        if !self.timeline.preview_armed() {
            return;
        }
        let command = gesture.command();
        let params = serde_json::json!({
            "commands": [{"command": command, "params": gesture.params()}]
        });
        let previous = self
            .session
            .preview_project()
            .unwrap_or(self.session.project())
            .clone();
        match self
            .session
            .execute_with_ripple("timeline.preview", params, self.preferences.ripple)
        {
            Ok(_) => {
                // `project()` stays committed; the player follows the staged
                // candidate (including a reversal back to the entry state) so a
                // trim shows its frame live.
                self.timeline.mark_preview(command, gesture.is_noop());
                if previous.as_ref()
                    != self
                        .session
                        .preview_project()
                        .unwrap_or(self.session.project())
                        .as_ref()
                {
                    self.sync_preview();
                }
            }
            Err(error) => {
                // The engine refused the whole candidate: drop the gesture and
                // show its reason instead of leaving a stale preview.
                self.timeline.abandon_gesture();
                self.timeline.refuse(error.to_string());
                self.discard_staged_preview();
            }
        }
    }
    /// Publish the staged candidate as one undo step.
    fn end_timeline_preview(&mut self) {
        if self.timeline.preview_command().is_some() {
            // A reversal can leave every member at its entry position while
            // item order shifted; discard it so no undo step is published.
            if self.timeline.preview_noop() {
                self.discard_staged_preview();
            } else {
                match self
                    .session
                    .execute("timeline.commit_preview", serde_json::json!({}))
                {
                    Ok(outcome) if outcome.changed => self.sync_preview(),
                    Ok(_) => {}
                    Err(error) => {
                        // A refused publication must not leave the gesture open.
                        self.error = Some(error.to_string());
                        let _ = self
                            .session
                            .execute("timeline.discard_preview", serde_json::json!({}));
                    }
                }
                self.timeline.clear_preview();
            }
        } else {
            self.timeline.clear_preview();
        }
        self.prune_selection();
    }
    /// Restore the entry project without a history entry.
    fn discard_staged_preview(&mut self) {
        if self.session.gesture_active() {
            match self
                .session
                .execute("timeline.discard_preview", serde_json::json!({}))
            {
                Ok(_) => {}
                Err(error) => self.error = Some(error.to_string()),
            }
        }
        self.timeline.clear_preview();
    }
    /// Escape on a timeline drag: discard the staged candidate.
    fn discard_timeline_preview(&mut self) {
        if self.timeline.preview_command().is_some() {
            self.discard_staged_preview();
        } else {
            self.timeline.clear_preview();
            if self.session.gesture_active()
                && let Err(error) = self.session.cancel_gesture()
            {
                self.error = Some(error.to_string());
            }
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
                // Closing a preview lifetime fences even a net-zero drag.
                Ok(_) => self.sync_preview(),
                Err(error) => self.error = Some(error.to_string()),
            }
        }
    }
    /// Escape: abandon the innermost pointer interaction in progress.
    pub(in crate::editor) fn cancel_interaction(&mut self) {
        // An inline edit commits on Escape; it never cancels the typed buffer.
        if self.commit_text_edit() {
            return;
        }
        if self.cancel_preview_transform() {
            return;
        }
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
        if self.timeline.cancel_keyframe_drag() {
            self.discard_gesture();
            self.prune_selection();
            return;
        }
        if self.timeline.cancel_volume_drag() {
            self.discard_gesture();
            return;
        }
        if let Some(before) = self.timeline.cancel_keyframe_marquee() {
            self.selection_scopes.keyframes = before;
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
            self.discard_timeline_preview();
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
            .and_then(|_| {
                self.session
                    .execute_with_ripple(command, params, self.preferences.ripple)
            }) {
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

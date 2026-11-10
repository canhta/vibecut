//! Routes the actions an area returns to the owners that apply them.
use crate::editor::{Command, Context, Desk, Editor, ItemId, Request, Tick, egui, timeline};

impl Editor {
    /// The read-only services every area receives.
    pub(in crate::editor) fn context(&self) -> Context<'_> {
        Context {
            session: &self.session,
            selection: &self.selection,
            locale: self.preferences.locale,
            dark: self.preferences.dark,
            save_status: self.save_status(),
            playhead: self.playback.position().unwrap_or(Tick::ZERO),
            hover: self.playback.hover(),
            ripple: self.preferences.ripple,
            preferences: &self.preferences,
            playback: &self.playback,
            preview: self.player.texture.handle(),
            keybindings: &self.preferences.keybindings,
            scope: self.selection_scopes.active(&self.selection),
            keyframes: &self.selection_scopes.keyframes,
            visuals: &self.media,
            disabled: std::collections::BTreeSet::new(),
            effect_targets: None,
            key_color_pick: self.player.key_color_pick.clone(),
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
                .filter(|id| {
                    crate::editor::properties::effect_refusal(self.session.project(), *id).is_none()
                })
                .collect()
        });
        let actions = paint(&mut state, &cx);
        drop(cx);
        let widgets = state.take_widgets();
        self.timeline = state;
        for widget in widgets {
            self.remember_with_icon(widget.id.clone(), widget.icon, &widget.response);
            if let (Some(tooltip), Some(state)) =
                (widget.tooltip, self.frame.widgets.get_mut(&widget.id))
            {
                state.tooltip = Some(tooltip);
            }
        }
        self.route_timeline(actions);
    }
    /// Paint the timeline panel and apply its requests.
    pub(super) fn timeline_panel(&mut self, ui: &mut egui::Ui) {
        self.with_timeline(|state, cx| timeline::show(state, cx, ui));
        self.sync_hover();
        if let Err(error) = self.remember_view() {
            self.desk().fail(error.to_string());
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
                Action::Command(command) => self.track_command(*command),
                Action::Generate(request) => {
                    if let Err(error) = self.media.generate(&mut self.session, request) {
                        self.desk().fail(error.to_string());
                    }
                }
                Action::RenameScene { scene, name } => {
                    let command = Command::RenameScene {
                        scene,
                        name: name.clone().into(),
                    };
                    if self.desk().apply_result(command, false).is_err() {
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
                    self.desk().fail(message);
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
        self.desk().select();
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
        self.desk()
            .set_preference("timeline_lanes", serde_json::json!(lanes));
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
            session.apply(engine::Command::DeleteKeyframes { keys })?;
            Ok(())
        });
        match result {
            Ok(()) => {
                self.selection_scopes.keyframes.clear();
                self.desk().sync();
            }
            Err(error) => self.desk().fail(error.to_string()),
        }
    }

    /// Stage absolute transform values for one item through the shared preview contract.
    pub(in crate::editor) fn preview_transform(
        &mut self,
        item: ItemId,
        values: &[(project::TransformField, f64)],
    ) {
        let commands = values
            .iter()
            .map(|(field, value)| Command::SetItemField {
                item,
                patch: transform_patch(*field, *value),
            })
            .collect();
        self.desk()
            .preview(Command::TimelinePreview { commands }, false);
    }
    /// Close a transform gesture with one undo step, or restore its entry project.
    pub(in crate::editor) fn end_preview_transform(&mut self, commit: bool) {
        if !self.session.gesture_active() {
            return;
        }
        let command = if commit {
            Command::TimelineCommitPreview {}
        } else {
            Command::TimelineDiscardPreview {}
        };
        self.desk().preview(command, false);
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
        let commands = vec![Command::SetItemField {
            item,
            patch: engine::ItemParamPatch::TextContent(content.into()),
        }];
        self.desk()
            .preview(Command::TimelinePreview { commands }, false);
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
            self.desk()
                .preview(Command::TimelineDiscardPreview {}, false);
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
            self.desk().fail(error.to_string());
            self.timeline.abandon_gesture();
            return;
        }
        if !self.desk().select() {
            self.timeline.abandon_gesture();
            return;
        }
        if let Err(error) = self.session.begin_gesture() {
            self.desk().fail(error.to_string());
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
        let ripple = self.preferences.ripple;
        let result = self.desk().run_command(command, ripple);
        match result {
            Ok(_) => {
                let current = self
                    .session
                    .preview_project()
                    .unwrap_or(self.session.project());
                if previous.as_ref() != current.as_ref() {
                    self.desk().sync();
                }
            }
            Err(error) => self.desk().fail(error.to_string()),
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
            self.desk().fail(error.to_string());
            self.timeline.abandon_gesture();
            return;
        }
        if !self.desk().select() {
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
        let staged = Command::TimelinePreview {
            commands: vec![command.clone()],
        };
        let previous = self
            .session
            .preview_project()
            .unwrap_or(self.session.project())
            .clone();
        let ripple = self.preferences.ripple;
        let result = self.desk().run_command(staged, ripple);
        match result {
            Ok(_) => {
                // `project()` stays committed; the player follows the staged
                // candidate (including a reversal back to the entry state) so a
                // trim shows its frame live.
                self.timeline
                    .mark_preview(command.kind(), gesture.is_noop());
                let current = self
                    .session
                    .preview_project()
                    .unwrap_or(self.session.project());
                if previous.as_ref() != current.as_ref() {
                    self.desk().sync();
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
                match self.session.apply(Command::TimelineCommitPreview {}) {
                    Ok(outcome) if outcome.changed => self.desk().sync(),
                    Ok(_) => {}
                    Err(error) => {
                        // A refused publication must not leave the gesture open.
                        self.desk().fail(error.to_string());
                        let _ = self.session.apply(Command::TimelineDiscardPreview {});
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
        if self.session.gesture_active()
            && let Err(error) = self.session.apply(Command::TimelineDiscardPreview {})
        {
            self.desk().fail(error.to_string());
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
                self.desk().fail(error.to_string());
            }
        }
    }
    /// Publish the open gesture as one undo step.
    fn end_drag_gesture(&mut self) {
        if self.session.gesture_active() {
            match self.session.commit_gesture() {
                Ok(outcome) if outcome.changed => self.desk().sync(),
                Ok(_) => {}
                Err(error) => self.desk().fail(error.to_string()),
            }
        }
        self.prune_selection();
    }
    /// Discard the open gesture and show the project it started from.
    fn discard_gesture(&mut self) {
        if self.session.gesture_active() {
            match self.session.cancel_gesture() {
                // Closing a preview lifetime fences even a net-zero drag.
                Ok(_) => self.desk().sync(),
                Err(error) => self.desk().fail(error.to_string()),
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
        if let Some(changed) = self.properties.cancel_scrub() {
            if self.session.gesture_active()
                && let Err(error) = self.session.cancel_gesture()
            {
                self.desk().fail(error.to_string());
            }
            if changed {
                self.desk().sync();
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
        if self.timeline.cancel_fade_drag() {
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
        if self.properties.cancel_reorder() {
            return;
        }
        if self.timeline.cancel_drag() {
            self.discard_timeline_preview();
        } else if let Some(before) = self.timeline.cancel_marquee() {
            self.selection = before;
            self.selection_changed();
        } else {
            self.selection_scopes.clear_specific(&mut self.selection);
            self.desk().select();
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
    /// Run a typed engine command on the current selection (track edits).
    pub(in crate::editor) fn track_command(&mut self, command: Command) {
        let before = self.session.project().clone();
        let ripple = self.preferences.ripple;
        let result = self.desk().run_command(command, ripple);
        match result {
            Ok(_) => {
                self.prune_selection();
                if !std::sync::Arc::ptr_eq(&before, self.session.project()) {
                    self.desk().sync();
                }
            }
            Err(error) => self.desk().fail(error.to_string()),
        }
    }
    /// Lend the write context to one area for this frame.
    pub(in crate::editor) fn desk(&mut self) -> Desk<'_> {
        Desk::new(self)
    }
    /// Apply the requests areas queued; each runs once, in push order.
    pub(in crate::editor) fn drain_requests(&mut self) {
        while !self.requests.is_empty() {
            for request in std::mem::take(&mut self.requests) {
                match request {
                    Request::Seek(at) => {
                        if let Err(error) = self.seek(at) {
                            self.desk().fail(error.to_string());
                        }
                    }
                    Request::OpenModel(id) => {
                        if let Err(error) = self.open_model(&id) {
                            self.desk().fail(error.to_string());
                        }
                    }
                }
            }
        }
    }
}

/// The typed patch for one transform field the player stages.
fn transform_patch(field: project::TransformField, value: f64) -> engine::ItemParamPatch {
    use project::TransformField as F;
    match field {
        F::PositionX => engine::ItemParamPatch::PositionX(value),
        F::PositionY => engine::ItemParamPatch::PositionY(value),
        F::ScaleX => engine::ItemParamPatch::ScaleX(value),
        F::ScaleY => engine::ItemParamPatch::ScaleY(value),
        F::Rotate => engine::ItemParamPatch::Rotate(value),
    }
}

#[cfg(test)]
mod tests;

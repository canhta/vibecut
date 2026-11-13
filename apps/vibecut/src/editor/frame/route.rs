//! Routes the actions an area returns to the owners that apply them.
use crate::editor::{Command, Context, Editor, ItemId, Request, Tick, egui, timeline};

impl Editor {
    /// The read-only services every area receives.
    pub(in crate::editor) fn context(&self) -> Context<'_> {
        Context {
            session: &self.session,
            project_path: match &self.session {
                crate::services::lifecycle::EditorSession::Persistent(autosave) => {
                    autosave.saved_path()
                }
                crate::services::lifecycle::EditorSession::Memory(_) => None,
            },
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
            scope: self.selection_scopes.active(self.session.selection()),
            keyframes: &self.selection_scopes.keyframes,
            mask_points: &self.selection_scopes.mask_points,
            transition: self.selection_scopes.transition.filter(|_| {
                self.selection_scopes.active(self.session.selection())
                    == crate::editor::selection::SelectionScope::Transition
            }),
            visuals: self.timeline.visuals(),
            cache: self.timeline.cache_ui(),
            disabled: std::collections::BTreeSet::new(),
            effect_targets: None,
            key_color_pick: self.player.key_color_pick.clone(),
        }
    }
    /// Run `paint` with the timeline state and its context, then apply what it returns.
    fn paint_timeline(
        &mut self,
        paint: impl FnOnce(&mut timeline::State, &Context<'_>) -> Vec<timeline::Action>,
    ) {
        let disabled = timeline::TOOLBAR_ACTIONS
            .iter()
            .copied()
            .chain(["edit.copy", "edit.cut", "edit.paste"])
            .filter(|id| crate::actions::action(id).is_some() && !self.action_enabled(id))
            .collect();
        let mut state = std::mem::take(&mut self.timeline);
        let media = state.take_media();
        let mut cx = self.context();
        cx.visuals = &media;
        cx.cache = media.cache_ui();
        cx.disabled = disabled;
        cx.effect_targets = self.assets.effect_drag.is_some().then(|| {
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
        state.restore_media(media);
        self.timeline = state;
        self.with_timeline(|state, port| {
            for widget in state.take_widgets() {
                port.widget(widget.id, widget.icon, widget.tooltip, &widget.response);
            }
        });
        self.apply_timeline(actions);
    }
    /// Paint the timeline panel and apply its requests.
    pub(super) fn timeline_panel(&mut self, ui: &mut egui::Ui) {
        self.paint_timeline(|state, cx| timeline::show(state, cx, ui));
        self.sync_hover();
        if let Err(error) = self.sync_loop() {
            self.fail(error.to_string());
        }
        if let Err(error) = self.remember_view() {
            self.fail(error.to_string());
        }
    }
    /// Paint the scene sheet and apply its requests.
    pub(super) fn timeline_scenes(&mut self, ctx: &egui::Context) {
        self.paint_timeline(|state, cx| timeline::show_scenes(state, cx, ctx));
    }
    /// Apply the actions the timeline returned. The area applies what it can through its
    /// port; the actions that write the frame's selection scopes come back here, one at a
    /// time, with everything after them.
    pub(in crate::editor) fn apply_timeline(&mut self, mut actions: Vec<timeline::Action>) {
        while !actions.is_empty() {
            let mut rest = self.with_timeline(|state, port| timeline::apply(state, port, actions));
            if rest.is_empty() {
                return;
            }
            let head = rest.remove(0);
            self.route_selection(head);
            actions = rest;
        }
    }
    /// The timeline actions that read or write the selection scopes, the inspector's open
    /// scrub or the gesture openers, which the frame owns.
    fn route_selection(&mut self, action: timeline::Action) {
        use timeline::Action;
        match action {
            Action::Select { item, modifiers } => self.select_item(item, modifiers),
            Action::SelectTransition(transition) => self.select_transition(transition),
            Action::Marquee(items) => {
                self.selection_scopes.transition = None;
                self.write_selection(items);
            }
            Action::SettleSelection => self.selection_changed(),
            Action::SelectKeyframe {
                keyframe,
                modifiers,
            } => self.select_keyframe(keyframe, modifiers),
            Action::MarqueeKeyframes(keyframes) => {
                self.selection_scopes.keyframes = keyframes;
            }
            Action::ClearKeyframes => self.selection_scopes.keyframes.clear(),
            Action::DeleteKeyframes(keyframes) => self.delete_keyframes(&keyframes),
            Action::ClearSelection => {
                self.replace_selection(Default::default());
            }
            Action::BeginGesture { select } => self.begin_drag_gesture(select),
            Action::BeginPreview { select } => self.begin_timeline_preview(select),
            Action::CancelInteraction => self.cancel_interaction(),
            // The area applies every other action itself.
            _ => {}
        }
    }
    /// A keyframe click scopes the selection without dropping the item context.
    fn select_keyframe(
        &mut self,
        keyframe: crate::editor::selection::KeyframeSelection,
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
        self.write_selection(std::collections::BTreeSet::from([keyframe.item]));
        self.properties.clear_drafts();
    }
    /// Delete every selected keyframe as one undo step.
    pub(in crate::editor) fn delete_keyframes(
        &mut self,
        keyframes: &std::collections::BTreeSet<crate::editor::selection::KeyframeSelection>,
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
        let result =
            self.session
                .transaction(|session| -> std::result::Result<(), engine::EditError> {
                    session.apply(engine::Command::DeleteKeyframes { keys })?;
                    Ok(())
                });
        match result {
            Ok(()) => {
                self.selection_scopes.keyframes.clear();
                self.sync_preview();
            }
            Err(error) => self.fail(error.to_string()),
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
        self.preview_command(Command::TimelinePreview { commands });
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
        self.preview_command(command);
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
        self.preview_command(Command::TimelinePreview { commands });
    }
    /// Commit an open inline edit as one undo step, or discard a net-zero session.
    pub(in crate::editor) fn commit_text_edit(&mut self) -> bool {
        let Some(commit) = self.player.transform.cancel_text() else {
            return false;
        };
        self.end_preview_transform(commit);
        true
    }
    /// Select one mask point without leaving the item selection.
    pub(in crate::editor) fn select_mask_point(
        &mut self,
        target: crate::editor::selection::MaskPointSelection,
        toggle: bool,
    ) {
        if toggle {
            if !self.selection_scopes.mask_points.remove(&target) {
                self.selection_scopes.mask_points.insert(target.clone());
            }
        } else {
            self.selection_scopes.mask_points.clear();
            self.selection_scopes.mask_points.insert(target.clone());
        }
        let selected = self.session.selection();
        if selected.len() != 1 || !selected.contains(&target.item) {
            self.write_selection(std::collections::BTreeSet::from([target.item]));
        }
        self.properties.clear_drafts();
    }
    /// Replace the point scope with the anchors a marquee covered (Shift adds
    /// to it). The point scope only ever holds one mask of one item.
    pub(in crate::editor) fn select_mask_points(
        &mut self,
        item: ItemId,
        mask: &str,
        points: &[usize],
        toggle: bool,
    ) {
        if toggle {
            self.selection_scopes
                .mask_points
                .retain(|target| target.item == item && target.mask == mask);
        } else {
            self.selection_scopes.mask_points.clear();
        }
        for point in points {
            self.selection_scopes.mask_points.insert(
                crate::editor::selection::MaskPointSelection {
                    item,
                    mask: mask.to_owned(),
                    point: *point,
                },
            );
        }
        let selected = self.session.selection();
        if selected.len() != 1 || !selected.contains(&item) {
            self.write_selection(std::collections::BTreeSet::from([item]));
        }
        self.properties.clear_drafts();
    }
    /// Drop the mask-point scope and keep the item selected.
    pub(in crate::editor) fn clear_mask_points(&mut self) {
        if self.selection_scopes.mask_points.is_empty() {
            return;
        }
        self.selection_scopes.mask_points.clear();
    }
    /// Stage one absolute mask path from a completed gesture origin.
    pub(in crate::editor) fn preview_mask_path(
        &mut self,
        item: ItemId,
        mask: &str,
        path: &project::MaskPath,
    ) {
        let commands = vec![Command::SetMaskPath {
            item,
            mask: mask.to_owned(),
            path: path.clone(),
        }];
        self.preview_command(Command::TimelinePreview { commands });
    }
    /// Stage one absolute whole-mask candidate from a gizmo gesture origin.
    pub(in crate::editor) fn preview_mask_box(&mut self, item: ItemId, mask: project::Mask) {
        let commands = vec![Command::UpdateMask { item, mask }];
        self.preview_command(Command::TimelinePreview { commands });
    }
    /// The anchors of the one mask the point scope holds, as `(item, mask, indices)`
    /// in path order. The scope only ever holds one mask of one item.
    pub(in crate::editor) fn mask_point_group(&self) -> Option<(ItemId, String, Vec<usize>)> {
        let first = self.selection_scopes.mask_points.iter().next()?;
        let points = self
            .selection_scopes
            .mask_points
            .iter()
            .filter(|point| point.item == first.item && point.mask == first.mask)
            .map(|point| point.point)
            .collect();
        Some((first.item, first.mask.clone(), points))
    }
    /// Insert one point at the nearest cubic parameter, then select it.
    pub(in crate::editor) fn add_mask_point(
        &mut self,
        item: ItemId,
        mask: &str,
        segment: usize,
        t: f64,
    ) {
        if self.session.gesture_active() {
            self.end_preview_transform(false);
        }
        let command = Command::AddMaskPoint {
            item,
            mask: mask.to_owned(),
            segment,
            t,
        };
        match self.apply_result(command) {
            Ok(outcome) if outcome.changed => {
                self.selection_scopes.mask_points.clear();
                self.selection_scopes.mask_points.insert(
                    crate::editor::selection::MaskPointSelection {
                        item,
                        mask: mask.to_owned(),
                        point: segment + 1,
                    },
                );
            }
            Ok(_) => {}
            Err(_) => {}
        }
    }
    /// Delete the selected mask points as one undo step, never the item.
    pub(in crate::editor) fn delete_mask_points(&mut self) {
        let mut groups: std::collections::BTreeMap<(ItemId, String), Vec<usize>> =
            std::collections::BTreeMap::new();
        for target in &self.selection_scopes.mask_points {
            groups
                .entry((target.item, target.mask.clone()))
                .or_default()
                .push(target.point);
        }
        if groups.is_empty() {
            return;
        }
        let commands = groups
            .into_iter()
            .map(|((item, mask), points)| Command::RemoveMaskPoint { item, mask, points })
            .collect();
        if let Ok(outcome) = self.apply_result(Command::Batch { commands })
            && outcome.changed
        {
            self.selection_scopes.mask_points.clear();
            self.prune_selection();
        }
    }
    /// Escape during a preview transform drops the candidate without a history entry.
    fn cancel_preview_transform(&mut self) -> bool {
        if !self.player.transform.cancel() {
            return false;
        }
        if self.session.gesture_active() {
            self.preview_command(Command::TimelineDiscardPreview {});
        }
        true
    }

    fn begin_drag_gesture(&mut self, select: Option<ItemId>) {
        if self.timeline.preview_armed() {
            self.discard_timeline_preview();
        }
        if select.is_some() {
            self.selection_changed();
        }
        if self.session.transaction_active()
            && let Err(error) = self.session.cancel_gesture()
        {
            self.fail(error.to_string());
            self.timeline.abandon_gesture();
            return;
        }
        if let Some(item) = select
            && !self.write_selection(std::collections::BTreeSet::from([item]))
        {
            self.timeline.abandon_gesture();
            return;
        }
        if let Err(error) = self.session.begin_gesture() {
            self.fail(error.to_string());
            self.timeline.abandon_gesture();
        }
    }
    /// Arm the group contract: only the selection is applied before the first
    /// staged command, so `timeline.preview` can open its own gesture.
    fn begin_timeline_preview(&mut self, select: Option<ItemId>) {
        if select.is_some() {
            self.selection_changed();
        }
        if self.session.transaction_active()
            && let Err(error) = self.session.cancel_gesture()
        {
            self.fail(error.to_string());
            self.timeline.abandon_gesture();
            return;
        }
        if let Some(item) = select
            && !self.write_selection(std::collections::BTreeSet::from([item]))
        {
            self.timeline.abandon_gesture();
            return;
        }
        self.timeline.arm_preview();
    }
    /// Escape on a timeline drag: discard the staged candidate.
    fn discard_timeline_preview(&mut self) {
        self.with_timeline(timeline::discard_timeline_preview);
    }
    /// Discard the open gesture and show the project it started from.
    pub(super) fn discard_gesture(&mut self) {
        self.with_timeline(|_, port| timeline::discard_gesture(port));
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
        // Mask gestures and the point scope pop before the item scope.
        if self.player.mask_path.cancel() {
            if self.session.gesture_active()
                && let Err(error) = self.session.apply(Command::TimelineDiscardPreview {})
            {
                self.fail(error.to_string());
            }
            return;
        }
        if !self.selection_scopes.mask_points.is_empty() {
            self.clear_mask_points();
            return;
        }
        if self.assets.transition_drag.take().is_some() {
            return;
        }
        if self.assets.cancel_media_marquee() {
            return;
        }
        if self.timeline.cancel_bookmark_drag() {
            self.discard_gesture();
            return;
        }
        if let Some(changed) = self.properties.cancel_graph() {
            if self.session.gesture_active()
                && let Err(error) = self.session.cancel_gesture()
            {
                self.fail(error.to_string());
            }
            if changed {
                self.sync_preview();
            }
            return;
        }
        if let Some(changed) = self.properties.cancel_scrub() {
            if self.session.gesture_active()
                && let Err(error) = self.session.cancel_gesture()
            {
                self.fail(error.to_string());
            }
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
        if self.assets.effect_drag.take().is_some() {
            return;
        }
        if self.properties.cancel_reorder() {
            return;
        }
        if self.timeline.cancel_drag() {
            self.discard_timeline_preview();
        } else if let Some(before) = self.timeline.cancel_marquee() {
            self.replace_selection(before);
        } else {
            if self
                .selection_scopes
                .clear_specific(self.session.selection())
            {
                self.write_selection(Default::default());
            }
            if self.assets.has_media_selection() {
                self.assets.clear_media_selection();
            }
        }
    }
    /// Step the timeline zoom, keeping the playhead anchored.
    pub(in crate::editor) fn zoom_timeline(&mut self, zoom_in: bool) {
        let playhead = self.playback.position().unwrap_or(Tick::ZERO);
        let duration = self.session.project().duration();
        self.timeline.zoom_step(zoom_in, duration, playhead);
    }
    /// Apply one proxy intention. The on/off choice is session presentation
    /// state: it reaches preview through the shared index on the next frame
    /// request and never edits the project, its history or export.
    fn proxy_intention(&mut self, action: crate::editor::context::ProxyAction) {
        if let crate::editor::context::ProxyAction::Toggle(on) = action {
            if let Err(error) = self.session.set_proxy_preview(on) {
                self.fail(error.to_string());
                return;
            }
            if let Err(error) = self.playback.set_proxy_preview(on) {
                self.fail(error.to_string());
            }
            self.sync_preview();
        }
        let budget = self.playback.budget().clone();
        let assets = self.session.project().data().assets.clone();
        self.timeline.request_proxy(action, &assets, &budget);
    }
    /// Show a creator-facing failure on the editor's error line.
    pub(in crate::editor) fn fail(&mut self, message: impl Into<String>) {
        self.error = Some(message.into());
    }
    /// Stage a preview and always show it; a reversal must restore the entry
    /// frame even when the engine reports no net change.
    pub(in crate::editor) fn preview_command(&mut self, command: Command) -> bool {
        match self.session.apply(command) {
            Ok(_) => {
                self.sync_preview();
                true
            }
            Err(error) => {
                self.fail(error.to_string());
                false
            }
        }
    }
    /// Apply one typed command; resync the preview once when it committed a change
    /// and show a refusal on the error line.
    pub(in crate::editor) fn apply_result(
        &mut self,
        command: Command,
    ) -> crate::Result<engine::Outcome> {
        let before = self.session.project().clone();
        match self.session.apply(command) {
            Ok(outcome) => {
                if outcome.changed || !std::sync::Arc::ptr_eq(&before, self.session.project()) {
                    self.sync_preview();
                }
                Ok(outcome)
            }
            Err(error) => {
                self.fail(error.to_string());
                Err(error.into())
            }
        }
    }
    /// Apply the requests areas queued; each runs once, in push order.
    pub(in crate::editor) fn drain_requests(&mut self) {
        while !self.requests.is_empty() {
            for request in std::mem::take(&mut self.requests) {
                match request {
                    Request::Cache(action) => self.timeline.request_cache(action),
                    Request::Proxy(action) => self.proxy_intention(action),
                    Request::ImportLut => {
                        self.lifecycle_action =
                            Some(crate::services::lifecycle::EditorAction::ImportLut);
                    }
                    Request::Seek(at) => {
                        if let Err(error) = self.seek(at) {
                            self.fail(error.to_string());
                        }
                    }
                    Request::OpenModel(id) => {
                        if let Err(error) = self.open_model(&id) {
                            self.fail(error.to_string());
                        }
                    }
                    Request::Action(id) => self.action(&id),
                    Request::ExportClips => {
                        self.lifecycle_action =
                            Some(crate::services::lifecycle::EditorAction::ExportClips);
                    }
                    Request::AddEffect { item, kind } => {
                        self.with_properties(|_, port| {
                            crate::editor::properties::add_effect_to_item(port, item, &kind);
                        });
                    }
                    Request::Lifecycle(action) => self.lifecycle_action = Some(action),
                    Request::ArmKeyColor(pick) => self.player.key_color_pick = pick,
                    Request::ReplaceMedia(item) => self.timeline.request_replace(item),
                    Request::PruneSelection => self.prune_selection(),
                    Request::InsertShape(id) => {
                        if self.action_enabled(&id) {
                            self.action(&id);
                            self.with_assets(|assets, port| assets.remember_sticker(port, &id));
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

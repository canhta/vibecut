//! Owns the actions the properties area requests, the gesture bookkeeping on
//! its state and the read-only projection of the selected element.
use super::{
    Arc, ColorTarget, ElementKind, ItemId, ItemParams, Rect, ScrubTarget, State, StreamKind,
    TrackKind, egui,
};

/// A control the frame records for automation.
pub(in crate::editor) struct Widget {
    pub(in crate::editor) id: String,
    pub(in crate::editor) icon: Option<&'static str>,
    pub(in crate::editor) tooltip: Option<String>,
    pub(in crate::editor) response: egui::Response,
}

/// Requests the properties area cannot satisfy alone; the frame routes them
/// in order after `show` returns.
pub(in crate::editor) enum Action {
    /// Start one footage analysis of a clip's source (`footage.analyze`).
    AnalyzeFootage {
        item: ItemId,
        kind: engine::footage::CorrectionKind,
    },
    /// Stop the running analysis of a clip.
    CancelFootage {
        item: ItemId,
        kind: engine::footage::CorrectionKind,
    },
    /// Show a model in the model manager: the link a missing-model row offers.
    OpenModel(String),
    /// Open Link media: the selected clip's source file is gone.
    OpenLinkMedia,
    /// Keyword emphasis of the caption cue under the playhead; the frame sends it to the
    /// Captions panel.
    Keywords(crate::editor::context::KeywordAction),
    ImportLut {
        item: ItemId,
        expected: Option<project::LutId>,
    },
    SelectLut {
        item: ItemId,
        id: Option<project::LutId>,
    },
    /// One In, Out or Combo slot of the selected clip, through `clip.set_animation`.
    SetClipAnimation {
        item: ItemId,
        slot: engine::AnimationSlot,
        preset: Option<engine::AnimationPreset>,
        duration: Option<time::Tick>,
        snapshot: Arc<project::Project>,
    },
    ApplyTextPreset {
        item: ItemId,
        preset: engine::TextPresetApply,
        snapshot: Arc<project::Project>,
    },
    /// Star or unstar one animation look (`animations` family); a preference write without history.
    ToggleAnimationFavorite {
        preset: engine::AnimationPreset,
        favorite: bool,
    },
    SaveTextPreset {
        item: ItemId,
        snapshot: Arc<project::Project>,
    },
    /// Switch the Project/Details strip through the action registry.
    Activate(&'static str),
    Cache(crate::editor::context::CacheAction),
    Proxy(crate::editor::context::ProxyAction),
    SetField {
        item: ItemId,
        field: String,
        value: String,
    },
    SetShared {
        field: String,
        value: String,
    },
    SetSpeed {
        item: ItemId,
        speed: f64,
        maintain_pitch: bool,
    },
    /// One fade edge's duration in seconds, through `audio.set_fade`.
    SetFade {
        item: ItemId,
        edge: project::FadeEdge,
        seconds: f64,
    },
    /// The Audio tab's Noise reduction switch, through the cleanup effect commands.
    SetNoiseReduction {
        item: ItemId,
        enabled: bool,
    },
    /// The Speed tab's curve editor: the whole curve as `(span in thousandths, speed)`
    /// stretches, through `timeline.set_speed_curve`.
    SetSpeedCurve {
        item: ItemId,
        segments: Vec<(u16, f64)>,
        maintain_pitch: bool,
    },
    /// A speed ramp preset, through `timeline.apply_speed_preset`.
    ApplySpeedPreset {
        item: ItemId,
        preset: project::SpeedPreset,
        maintain_pitch: bool,
    },
    /// The Audio tab's Voice enhancement switch: one batched edit adds or enables the preset's
    /// effects, or disables them keeping their values.
    SetVoiceEnhancement {
        item: ItemId,
        enabled: bool,
    },
    /// The preset's strength: every preset effect's dry/wet mix, in percent.
    SetVoiceEnhancementStrength {
        item: ItemId,
        wet_percent: f32,
    },
    /// One loudness scope switched on or off: `audio.set_loudness` or `audio.clear_loudness`.
    SetLoudnessScope {
        scope: project::LoudnessScope,
        enabled: bool,
    },
    /// The shared loudness target, applied to every switched-on scope in one batch.
    SetLoudnessTarget(project::LoudnessTarget),
    /// Apply: measure the switched-on scopes and store their gains.
    MeasureLoudness,
    /// Stop the running loudness measurement.
    CancelLoudness,
    /// The Speed tab's Reverse switch, through `timeline.set_reverse`.
    SetReverse {
        item: ItemId,
        reverse: bool,
    },
    /// The selected transition's duration, through `transition.set_duration`.
    SetTransitionDuration {
        transition: project::TransitionId,
        duration: time::Tick,
    },
    /// Realign the selected transition: `transition.move`, batched with a shrink.
    SetTransitionAlignment {
        transition: project::TransitionId,
        alignment: project::TransitionAlignment,
    },
    /// Copy the selected transition's kind and duration to every other transition, through
    /// `transition.apply_to_all`.
    ApplyTransitionToAll(project::TransitionId),
    RemoveTransition(project::TransitionId),
    SetCanvas {
        width: u32,
        height: u32,
        preset: Option<project::AspectPreset>,
    },
    SetFps {
        num: i64,
        den: i64,
    },
    /// One clip-background commit over the listed video items, one undo step.
    SetClipBackground {
        items: Vec<ItemId>,
        background: project::ClipBackground,
    },
    ResetTransform(ItemId),
    /// The Blend section's defaults (blend mode and opacity) as one undo step.
    ResetBlend(ItemId),
    /// Add a key at the playhead on every listed channel that lacks one, or
    /// remove them all when each already has one: one diamond, one undo step.
    ToggleKeyframe {
        item: ItemId,
        /// Animation path (a field or `effects.<id>.params.<key>`) and its value.
        fields: Vec<(String, f64)>,
        time: time::Tick,
    },
    /// Move the playhead to a neighbouring key (project time), through the shared transport.
    SeekToKey(time::Tick),
    /// Upsert a key at the playhead while a channel is already animated.
    SetKeyframeAt {
        item: ItemId,
        field: String,
        time: time::Tick,
        value: f64,
    },
    SetSection {
        key: &'static str,
        open: bool,
    },
    SetTab {
        kind: &'static str,
        tab: &'static str,
    },
    /// Open an engine gesture for the scrub recorded in the state.
    ScrubBegin,
    /// End the open gesture; `commit` keeps it as one undo step.
    ScrubEnd {
        commit: bool,
        changed: bool,
    },
    SetEffectParam {
        item: ItemId,
        effect: String,
        key: String,
        value: f64,
    },
    /// Typed parameter values on one instance; a reset of a wheel or curve is one edit.
    SetEffectValues {
        item: ItemId,
        effect: String,
        values: Vec<(String, project::EffectParamUpdate)>,
    },
    /// Copy the selected item's grade onto every video and image item in one batch.
    ApplyGradeToAll {
        item: ItemId,
    },
    /// Store the selected item's grade as a named app-owned preset.
    SaveGradePreset {
        item: ItemId,
    },
    ToggleEffect {
        item: ItemId,
        effect: String,
        enabled: bool,
    },
    RemoveEffect {
        item: ItemId,
        effect: String,
    },
    ReorderEffect {
        item: ItemId,
        effect: String,
        target: String,
    },
    /// Arm or leave the matte refinement brush the player paints with.
    SetMatteBrush(Option<crate::editor::MatteBrush>),
    /// Set the matte brush's Size, 1 to 100.
    SetMatteBrushSize(u32),
    /// Arm or leave the eyedropper that samples the rendered preview frame.
    SetKeyColorPick(Option<crate::editor::KeyPick>),
    /// Turn the chroma-key effect on or off for one clip.
    ToggleChromaKey {
        item: ItemId,
        enabled: bool,
    },
    /// Restore the chroma key's default colour.
    ResetKeyColor {
        item: ItemId,
        effect: String,
    },
    /// The preview returns to the project (picker closed or failed).
    SyncPreview,
    SetAnimatedParam {
        item: ItemId,
        path: project::AnimationPath,
        param: project::Param,
    },
    ColorPreview {
        command: engine::Command,
    },
    /// One colour commit: the command and, when a picked colour, its recent-colour record
    /// (one undo step together).
    ColorCommit {
        command: engine::Command,
        record: Option<project::Color>,
    },
    /// Font picker commands, preferences and family selection.
    Font(super::font_picker::Action),
    /// Mask row sub-panel edits.
    Mask(super::mask_picker::Action),
    /// The Text to speech tab: generate, consent, place and ducking.
    Narration(super::workflow::jobs::narration::Action),
    Error(String),
}

/// Draft and focus key of the animation duration field.
pub(in crate::editor) const ANIMATION_DRAFT: &str = "animation_duration";

impl State {
    pub(in crate::editor) fn input_ready(&self, session: &engine::Session) -> bool {
        self.ready_except(session, None)
    }

    /// `input_ready`, not counting the focus and draft of the field named by `own`.
    fn ready_except(&self, session: &engine::Session, own: Option<&str>) -> bool {
        !session.transaction_active()
            && session.preview_project().is_none()
            && self.focus.as_deref().is_none_or(|focus| Some(focus) == own)
            && self.drafts.keys().all(|key| Some(key.as_str()) == own)
            && self.scrub.is_none()
            && self.graph.is_none()
            && self.picker.is_none()
            && self.reorder.is_none()
            && !self.font_picker.open
            && self.font_import.is_none()
    }

    pub(in crate::editor) fn animation_target(
        &self,
        session: &engine::Session,
        selection: &std::collections::BTreeSet<ItemId>,
    ) -> Option<ItemId> {
        // The duration field is part of the control, so typing in it must not disable it.
        if !self.ready_except(session, Some(ANIMATION_DRAFT)) {
            return None;
        }
        let id = crate::editor::context::single(selection)?;
        session
            .project()
            .data()
            .scene()
            .timeline
            .iter()
            .find_map(|track| {
                (!track.locked)
                    .then(|| {
                        track
                            .items
                            .iter()
                            .find(|item| item.id == id && item.content.supports_animation())
                            .map(|item| item.id)
                    })
                    .flatten()
            })
    }

    pub(super) fn cancel_opacity(&mut self) -> Vec<Action> {
        self.opacity_edit = None;
        self.drafts
            .retain(|key, _| key != "opacity" && !key.starts_with("params."));
        if self
            .focus
            .as_deref()
            .is_some_and(|f| f == "opacity" || f.starts_with("params."))
        {
            self.focus = None;
        }
        if self
            .scrub
            .as_ref()
            .is_some_and(|s| matches!(s.target, ScrubTarget::Item(field) if field == "opacity" || field.starts_with("params.")))
        {
            return self.end_scrub(false);
        }
        Vec::new()
    }
    pub(in crate::editor) fn take_widgets(&mut self) -> Vec<Widget> {
        std::mem::take(&mut self.widgets)
    }

    /// Selection changed outside the panel: stale drafts go.
    pub(in crate::editor) fn clear_drafts(&mut self) {
        self.drafts.clear();
    }

    /// Whether a field of the panel holds the keyboard.
    pub(in crate::editor) fn editing(&self) -> bool {
        self.focus.is_some()
    }

    /// Open the colour popover for `target`, reading its current colour.
    pub(in crate::editor) fn open_picker(
        &mut self,
        project: &Arc<project::Project>,
        target: ColorTarget,
        anchor: Option<egui::Pos2>,
    ) -> crate::Result<Vec<Action>> {
        let mut actions = self.close_picker();
        actions.extend(self.end_scrub(false));
        let color = target.color(project)?;
        self.color_base = Some(project.clone());
        let mut picker = crate::services::color_picker::Picker::new(color);
        picker.opaque =
            matches!(&target, ColorTarget::EffectColor { key, .. } if key != "key_color");
        // Hang off the panel's left edge, below its tab strip, unless a row anchors it.
        picker.anchor = anchor.or_else(|| {
            self.area.map(|panel| {
                egui::pos2(
                    panel.left() - crate::kit::space::PANEL_GUTTER,
                    panel.top() + crate::kit::size::TAB_STRIP,
                )
            })
        });
        self.picker = Some(super::color_picker::Draft { target, picker });
        Ok(actions)
    }

    pub(in crate::editor) fn close_picker(&mut self) -> Vec<Action> {
        if self.picker.take().is_some() {
            self.color_base = None;
            return vec![Action::SyncPreview];
        }
        Vec::new()
    }

    /// Start a drag gesture on `target`; a previous one is cancelled first.
    pub(super) fn begin_scrub(
        &mut self,
        item: ItemId,
        target: ScrubTarget,
        value: f64,
    ) -> Vec<Action> {
        let mut actions = self.end_scrub(false);
        self.drafts.remove(&target.draft_key());
        self.focus = None;
        self.scrub = Some(super::ScrubDrag {
            item,
            target,
            start: value,
            last: value,
        });
        actions.push(Action::ScrubBegin);
        actions
    }

    pub(in crate::editor) fn end_scrub(&mut self, commit: bool) -> Vec<Action> {
        let Some(scrub) = self.scrub.take() else {
            return Vec::new();
        };
        self.drafts.remove(&scrub.target.draft_key());
        vec![Action::ScrubEnd {
            commit,
            changed: scrub.last != scrub.start,
        }]
    }

    /// Escape: drop the open scrub draft and report whether the drag moved.
    pub(in crate::editor) fn cancel_scrub(&mut self) -> Option<bool> {
        let scrub = self.scrub.take()?;
        self.drafts.remove(&scrub.target.draft_key());
        self.focus = None;
        Some(scrub.last != scrub.start)
    }

    /// Open one wheel or curve gesture; a previous unfinished one is cancelled.
    pub(super) fn begin_graph(&mut self, item: ItemId, effect: &str, key: &str) -> Vec<Action> {
        let mut actions = self.end_graph(false);
        self.graph = Some(super::gesture::GraphDrag {
            item,
            effect: effect.to_owned(),
            key: key.to_owned(),
        });
        self.graph_changed = false;
        actions.push(Action::ScrubBegin);
        actions
    }

    /// A wheel or curve gesture of this panel is open; its own transaction is not a block.
    pub(super) fn graph_open(&self) -> bool {
        self.graph.is_some()
    }

    pub(super) fn graph_matches(&self, item: ItemId, effect: &str, key: &str) -> bool {
        self.graph
            .as_ref()
            .is_some_and(|graph| graph.item == item && graph.effect == effect && graph.key == key)
    }

    pub(super) fn touch_graph(&mut self) {
        self.graph_changed = true;
    }

    /// End the open wheel or curve gesture as one undo step when it changed a value.
    pub(super) fn end_graph(&mut self, commit: bool) -> Vec<Action> {
        if self.graph.take().is_none() {
            return Vec::new();
        }
        let changed = std::mem::take(&mut self.graph_changed);
        vec![Action::ScrubEnd { commit, changed }]
    }

    /// Escape: drop the open wheel or curve gesture and report whether it wrote a value.
    pub(in crate::editor) fn cancel_graph(&mut self) -> Option<bool> {
        self.graph.take()?;
        Some(std::mem::take(&mut self.graph_changed))
    }

    /// Escape: drop the open effect reorder draft.
    pub(in crate::editor) fn cancel_reorder(&mut self) -> bool {
        self.reorder.take().is_some()
    }

    /// Gestures belong to the element and view visible at pointer-down; agents
    /// can change selection or the project between pointer events.
    pub(in crate::editor) fn reconcile(
        &mut self,
        project: &Arc<project::Project>,
        selected: Option<ItemId>,
    ) -> Vec<Action> {
        let mut actions = Vec::new();
        if self.picker.as_ref().is_some_and(|picker| {
            picker
                .target
                .item()
                .is_some_and(|item| selected != Some(item))
        }) || self
            .color_base
            .as_ref()
            .is_some_and(|base| !Arc::ptr_eq(base, project))
        {
            actions.extend(self.close_picker());
        }
        if self.font_picker.open && self.details {
            self.font_picker.close();
        }
        if self.mask.is_open() && (selected != self.mask.item() || self.details) {
            self.mask.close();
        }
        if self
            .chroma_staged
            .is_some_and(|staged| selected != Some(staged) || self.details)
        {
            self.chroma_staged = None;
        }
        if self
            .reorder
            .as_ref()
            .is_some_and(|drag| selected != Some(drag.item) || self.details)
        {
            self.reorder = None;
        }
        if self.graph.as_ref().is_some_and(|graph| {
            selected != Some(graph.item)
                || self.details
                || !graph_target(project, graph.item, &graph.effect)
        }) {
            actions.extend(self.end_graph(false));
        }
        if self
            .scrub
            .as_ref()
            .is_some_and(|scrub| selected != Some(scrub.item) || self.details)
        {
            actions.extend(self.end_scrub(false));
        }
        actions
    }

    pub(super) fn set_area(&mut self, area: Rect) {
        self.area = Some(area);
    }
}

/// Whether one clip still owns the effect a wheel or curve drag edits.
fn graph_target(project: &project::Project, item: ItemId, effect: &str) -> bool {
    let data = project.data();
    data.scene().timeline.iter().any(|track| {
        !track.locked
            && track.items.iter().any(|candidate| {
                candidate.id == item
                    && candidate
                        .effects
                        .iter()
                        .any(|candidate| candidate.id == effect)
            })
    })
}

/// Element kind of `id` and whether it has source audio, from project data.
pub(in crate::editor) fn element_kind(
    project: &project::Project,
    id: ItemId,
) -> Option<(ElementKind, bool)> {
    let data = project.data();
    let (track, item) = data.scene().timeline.iter().find_map(|track| {
        track
            .items
            .iter()
            .find(|item| item.id == id)
            .map(|item| (track, item))
    })?;
    let kind = match (&item.content, track.kind) {
        (ItemParams::Text(_) | ItemParams::Caption(_), _) => ElementKind::Text,
        (ItemParams::Audio(_), _) => ElementKind::Audio,
        (_, TrackKind::Sticker) => ElementKind::Sticker,
        (ItemParams::Effect(_), _) | (_, TrackKind::Effect) => ElementKind::Effect,
        (_, TrackKind::Filter | TrackKind::Adjustment) => ElementKind::Graphic,
        (ItemParams::Overlay(_) | ItemParams::Graphic(_), _) => ElementKind::Graphic,
        (ItemParams::Sticker(_), _) => ElementKind::Sticker,
        (ItemParams::Video(_), _) => {
            let Some(asset) = item
                .source_asset
                .and_then(|asset| data.assets.iter().find(|candidate| candidate.id() == asset))
            else {
                return Some((ElementKind::Graphic, false));
            };
            let video = asset
                .probe()
                .streams
                .iter()
                .find(|stream| stream.kind == StreamKind::Video);
            match video {
                Some(stream) if stream.frame_count == Some(1) => ElementKind::Image,
                Some(_) => ElementKind::Video,
                None => ElementKind::Image,
            }
        }
    };
    let has_audio = kind == ElementKind::Video
        && item.source_asset.is_some_and(|asset| {
            data.assets.iter().any(|candidate| {
                candidate.id() == asset
                    && candidate
                        .probe()
                        .streams
                        .iter()
                        .any(|stream| stream.kind == StreamKind::Audio)
            })
        });
    Some((kind, has_audio))
}

#[cfg(test)]
mod tests {
    use super::{Action, ItemId, ScrubTarget, State};

    fn names(actions: &[Action]) -> Vec<&'static str> {
        actions
            .iter()
            .map(|action| match action {
                Action::ScrubBegin => "begin",
                Action::ScrubEnd { commit: true, .. } => "commit",
                Action::ScrubEnd { commit: false, .. } => "cancel",
                Action::SyncPreview => "sync",
                _ => "other",
            })
            .collect()
    }

    #[test]
    fn scrub_gestures_request_the_engine_gesture_and_end_the_previous_one_first() {
        let item = ItemId(1);
        let target = || ScrubTarget::Item("opacity");
        let mut state = State::default();
        assert_eq!(names(&state.begin_scrub(item, target(), 0.5)), ["begin"]);
        assert_eq!(
            names(&state.begin_scrub(item, target(), 0.5)),
            ["cancel", "begin"],
            "a second scrub cancels the first"
        );
        state.drafts.insert("opacity".into(), "50".into());
        assert_eq!(names(&state.end_scrub(true)), ["commit"]);
        assert!(state.drafts.is_empty(), "ending a scrub drops its draft");
        assert!(names(&state.end_scrub(true)).is_empty());
    }

    #[test]
    fn a_scrub_outlives_neither_its_element_nor_the_details_view() {
        let item = ItemId(1);
        let project = std::sync::Arc::new(
            project::Project::empty(project::ProjectId(1), "State".into()).unwrap(),
        );
        for (selected, details, kept) in [
            (Some(item), false, true),
            (Some(ItemId(2)), false, false),
            (None, false, false),
            (Some(item), true, false),
        ] {
            let mut state = State::default();
            state.begin_scrub(item, ScrubTarget::Item("opacity"), 1.0);
            state.details = details;
            let actions = state.reconcile(&project, selected);
            assert_eq!(state.scrub.is_some(), kept);
            assert_eq!(names(&actions), if kept { vec![] } else { vec!["cancel"] });
        }
    }
}

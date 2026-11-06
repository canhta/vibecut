//! Owns the actions the properties area requests, the gesture bookkeeping on
//! its state and the read-only projection of the selected element.
use super::{
    Arc, BTreeSet, ColorTarget, ElementKind, ItemId, ItemParams, Rect, ScrubTarget, State,
    StreamKind, TrackKind, egui,
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
    /// Switch the Project/Details strip through the action registry.
    Activate(&'static str),
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
    SetCanvas {
        width: u32,
        height: u32,
        preset: Option<project::AspectPreset>,
    },
    SetFps {
        num: i64,
        den: i64,
    },
    SetBackground(project::Color),
    ResetTransform(ItemId),
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
    /// The preview returns to the project (picker closed or failed).
    SyncPreview,
    ColorPreview {
        command: &'static str,
        params: serde_json::Value,
    },
    ColorCommit {
        command: &'static str,
        params: serde_json::Value,
        recent: Vec<project::Color>,
    },
    Error(String),
}

impl State {
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
    pub(super) fn open_picker(
        &mut self,
        project: &Arc<project::Project>,
        target: ColorTarget,
        anchor: Option<egui::Pos2>,
    ) -> crate::Result<Vec<Action>> {
        let mut actions = self.close_picker();
        actions.extend(self.end_scrub(false));
        let color = target.color(project)?;
        self.color_base = Some(project.clone());
        let mut picker = crate::color_picker::Picker::new(target, color);
        // Hang off the panel's left edge, below its tab strip, unless a row anchors it.
        picker.anchor = anchor.or_else(|| {
            self.area.map(|panel| {
                egui::pos2(
                    panel.left() - crate::kit::space::PANEL_GUTTER,
                    panel.top() + crate::kit::size::TAB_STRIP,
                )
            })
        });
        self.picker = Some(picker);
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
        if self
            .reorder
            .as_ref()
            .is_some_and(|drag| selected != Some(drag.item) || self.details)
        {
            self.reorder = None;
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
        (_, TrackKind::Effect) => ElementKind::Effect,
        (_, TrackKind::Filter | TrackKind::Adjustment) => ElementKind::Graphic,
        (ItemParams::Overlay(_), _) => ElementKind::Graphic,
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

/// The single selected item, if exactly one is selected.
pub(in crate::editor) fn single(selection: &BTreeSet<ItemId>) -> Option<ItemId> {
    if selection.len() != 1 {
        return None;
    }
    selection.iter().next().copied()
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

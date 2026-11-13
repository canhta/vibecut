//! A forked session: an isolated copy whose recorded edits apply back as one batch.
use super::wire::Revision;
use super::{Command, Session};
use project::{Item, ItemId, ItemParams, Project, TrackId};
use serde::Serialize;
use std::{collections::BTreeMap, sync::Arc};
use time::Tick;

/// One command the fork's session applied, with the entry it ran through: replay
/// repeats exactly that admission.
pub(in crate::session) struct Logged {
    pub(in crate::session) command: Command,
    pub(in crate::session) wire: bool,
    /// The fork's ID counter when the command started. A failed transaction keeps the IDs
    /// its children took, so replay resumes from this counter to allocate the same IDs.
    pub(in crate::session) next_id: Option<u64>,
}

/// A copy of a [`Session`] taken at one revision. Commands run on the fork never touch the
/// live session; [`Session::apply_fork`] replays the ones that changed the project onto it.
///
/// The fork holds the project data, the ID counter, the ripple policy, the clipboards and the
/// view state of its parent, no history and no jobs. Its session has its own identity, so
/// receipts and job targets never cross between the two.
pub struct Fork {
    pub(in crate::session) session: Session,
    pub(in crate::session) base: Arc<Project>,
    pub(in crate::session) base_revision: Revision,
}

impl Fork {
    /// The fork's own session, for reads and for `build_request`. Mutation goes through
    /// the control path so every change is recorded.
    pub fn session(&self) -> &Session {
        &self.session
    }
    /// Mutable access for the control layer, which runs commands on the fork.
    pub(crate) fn session_mut(&mut self) -> &mut Session {
        &mut self.session
    }
    /// [`Session::transaction`] on the fork, for in-process workflows that group edits. The
    /// group is logged as one unit and replays as one, keeping the trim sources its children
    /// share; a failed group leaves no entry.
    pub fn transaction<T, E: From<super::EditError>>(
        &mut self,
        run: impl FnOnce(&mut Session) -> std::result::Result<T, E>,
    ) -> std::result::Result<T, E> {
        self.session.transaction(run)
    }
    /// The parent's revision when the fork was taken; the only revision it applies to.
    pub fn base_revision(&self) -> &Revision {
        &self.base_revision
    }
    /// The parent's project when the fork was taken.
    pub fn base_project(&self) -> &Arc<Project> {
        &self.base
    }
    /// The commands the kernel applied to the fork, in order, as they replay on the parent.
    /// A job-result command appears as the edit it resolved to, never as the job it named.
    pub fn commands(&self) -> impl Iterator<Item = &Command> {
        self.session
            .recorded
            .iter()
            .flatten()
            .flatten()
            .map(|logged| &logged.command)
    }
    /// Whether no command changed the fork's project.
    pub fn is_empty(&self) -> bool {
        self.session.recorded.as_ref().is_none_or(Vec::is_empty)
    }
    /// What the fork's edits did to the base project.
    pub fn summary(&self) -> ChangeSummary {
        ChangeSummary::between(&self.base, self.session.project())
    }
}

/// How a project differs from the one it was forked from, in the active scene.
///
/// An item can appear in several lists: a clip that was moved and trimmed is in both.
/// `captions_changed` holds caption items whose cues or caption parameters changed;
/// `params_changed` holds every other change to an item's content, effects, masks,
/// animations, blend mode, visibility or source asset.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub struct ChangeSummary {
    pub added: Vec<ItemId>,
    pub removed: Vec<ItemId>,
    /// Start or track changed.
    pub moved: Vec<ItemId>,
    /// Source range changed.
    pub trimmed: Vec<ItemId>,
    /// Retime changed.
    pub retimed: Vec<ItemId>,
    pub params_changed: Vec<ItemId>,
    pub captions_changed: Vec<ItemId>,
    pub duration_before: Tick,
    pub duration_after: Tick,
}

fn items(project: &Project) -> BTreeMap<ItemId, (TrackId, &Item)> {
    project
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| track.items.iter().map(move |item| (item.id, (track.id, item))))
        .collect()
}

impl ChangeSummary {
    /// Compare the active scenes of two projects.
    pub fn between(before: &Project, after: &Project) -> Self {
        let (old, new) = (items(before), items(after));
        let mut summary = Self {
            added: new.keys().filter(|id| !old.contains_key(*id)).copied().collect(),
            removed: old.keys().filter(|id| !new.contains_key(*id)).copied().collect(),
            moved: Vec::new(),
            trimmed: Vec::new(),
            retimed: Vec::new(),
            params_changed: Vec::new(),
            captions_changed: Vec::new(),
            duration_before: before.duration(),
            duration_after: after.duration(),
        };
        for (id, (old_track, old_item)) in &old {
            let Some((new_track, new_item)) = new.get(id) else {
                continue;
            };
            if old_track != new_track || old_item.start != new_item.start {
                summary.moved.push(*id);
            }
            if old_item.source_range != new_item.source_range {
                summary.trimmed.push(*id);
            }
            if old_item.retime != new_item.retime {
                summary.retimed.push(*id);
            }
            let content = old_item.content != new_item.content;
            if content && matches!(new_item.content, ItemParams::Caption(_)) {
                summary.captions_changed.push(*id);
            }
            if (content && !matches!(new_item.content, ItemParams::Caption(_)))
                || old_item.effects != new_item.effects
                || old_item.masks != new_item.masks
                || old_item.animations != new_item.animations
                || old_item.blend_mode != new_item.blend_mode
                || old_item.hidden != new_item.hidden
                || old_item.source_asset != new_item.source_asset
            {
                summary.params_changed.push(*id);
            }
        }
        summary
    }
    /// Whether nothing differs, durations included.
    pub fn is_empty(&self) -> bool {
        self.added.is_empty()
            && self.removed.is_empty()
            && self.moved.is_empty()
            && self.trimmed.is_empty()
            && self.retimed.is_empty()
            && self.params_changed.is_empty()
            && self.captions_changed.is_empty()
            && self.duration_before == self.duration_after
    }
}

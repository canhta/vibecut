use crate::{Command, EditError, Outcome, Result, edit};
use project::{ItemId, Project};
use serde_json::Value;
use std::collections::BTreeSet;
use std::{
    collections::VecDeque,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::Arc,
};

#[derive(Clone)]
struct Snapshot {
    project: Arc<Project>,
    selection: Arc<BTreeSet<ItemId>>,
}
#[derive(Clone)]
struct State {
    project: Arc<Project>,
    undo: VecDeque<Snapshot>,
    redo: Vec<Snapshot>,
    selection: Arc<BTreeSet<ItemId>>,
    merge_key: Option<String>,
    trim_sources: edit::TrimSources,
    next_id: Option<u64>,
    change: Option<Arc<Project>>,
}
/// Opaque identity for exactly one Session revision, including save boundaries.
#[derive(Clone)]
pub struct JobTarget {
    session: Arc<crate::control::Revision>,
    revision: Arc<crate::control::Revision>,
}
/// Owns project editing, bounded history, deferred changes and gesture lifecycle.
pub struct Session {
    metadata: project::Metadata,
    pub(crate) identity: Arc<crate::control::Revision>,
    pub(crate) revision: Arc<crate::control::Revision>,
    pub(crate) generation: u64,
    pub(crate) control: crate::control::State,
    state: State,
    limit: usize,
    gesture: Option<State>,
    grouping: bool,
}
impl Session {
    /// Retain 1..=1000 undo snapshots plus current/pending/active transaction snapshots.
    /// Candidates copy metadata; no media payload is copied.
    pub fn new(project: Project, history_limit: usize) -> Result<Self> {
        if !(1..=1000).contains(&history_limit) {
            return Err(EditError::Limit);
        }
        let data = project.data();
        let next_id = data
            .scenes
            .iter()
            .flat_map(|s| {
                std::iter::once(s.id.0)
                    .chain(s.timeline.iter().map(|t| t.id.0))
                    .chain(
                        s.timeline
                            .iter()
                            .flat_map(|t| t.items.iter().map(|i| i.id.0)),
                    )
                    .chain(s.bookmarks.iter().map(|b| b.id.0))
            })
            .max()
            .map_or(Some(0), |id| id.checked_add(1));
        let mut entropy = [0u8; 32];
        getrandom::fill(&mut entropy).map_err(|_| EditError::ControlIdentity)?;
        let identity = Arc::new(crate::control::Revision(
            blake3::hash(&entropy).to_hex().to_string(),
        ));
        Ok(Self {
            metadata: project::Metadata::default(),
            revision: Arc::new(crate::control::Revision(format!("{}:0", identity.0))),
            identity,
            generation: 0,
            control: crate::control::State::default(),
            state: State {
                project: Arc::new(project),
                undo: VecDeque::new(),
                redo: Vec::new(),
                selection: Arc::new(BTreeSet::new()),
                merge_key: None,
                trim_sources: edit::TrimSources::new(),
                next_id,
                change: None,
            },
            limit: history_limit,
            gesture: None,
            grouping: false,
        })
    }
    /// Create the default canvas/timeline through the registered lifecycle command.
    pub fn create(command: Command, history_limit: usize) -> Result<Self> {
        match command {
            Command::CreateProject { id, name } => {
                Self::new(Project::empty(id, name)?, history_limit)
            }
            _ => Err(EditError::Invalid("expected project.create")),
        }
    }
    pub fn metadata(&self) -> &project::Metadata {
        &self.metadata
    }
    /// Load optional sidecar presentation state, outside project history.
    pub fn restore_metadata(&mut self, metadata: project::Metadata) -> Result<()> {
        if self.transaction_active() {
            return Err(EditError::TransactionActive);
        }
        metadata.validate()?;
        self.metadata = metadata;
        Ok(())
    }
    /// Capture before submitting processing. Old results are rejected even after
    /// edit + undo returns to equal content, or another Session opens the project.
    pub fn job_target(&self) -> JobTarget {
        JobTarget {
            session: self.identity.clone(),
            revision: self.revision.clone(),
        }
    }
    /// Call at a successful save boundary; processing started before it is stale.
    pub fn saved(&mut self) -> Result<()> {
        if self.transaction_active() {
            return Err(EditError::TransactionActive);
        }
        self.generation.checked_add(1).ok_or(EditError::Limit)?;
        self.end_merge();
        self.invalidate_revision(false)?;
        Ok(())
    }
    pub(crate) fn matches_target(&self, target: &JobTarget) -> bool {
        Arc::ptr_eq(&self.identity, &target.session)
            && Arc::ptr_eq(&self.revision, &target.revision)
    }
    pub fn apply_job_result(&mut self, target: &JobTarget, command: Command) -> Result<Outcome> {
        if !Arc::ptr_eq(&self.identity, &target.session)
            || !Arc::ptr_eq(&self.revision, &target.revision)
        {
            return Err(EditError::StaleResult);
        }
        if self.transaction_active() {
            return Err(EditError::TransactionActive);
        }
        self.apply(command)
    }
    /// Current immutable snapshot, including a gesture/group preview while active.
    pub fn project(&self) -> &Arc<Project> {
        &self.state.project
    }
    /// Transient item selection, retained with history but never serialized into Project.
    pub fn selection(&self) -> &BTreeSet<ItemId> {
        &self.state.selection
    }
    /// Replace selection without making an undo step. Missing IDs and changes during
    /// a gesture/group refuse; changing selection ends implicit coalescing.
    pub fn set_selection(&mut self, selection: BTreeSet<ItemId>) -> Result<()> {
        if selection == *self.state.selection {
            return Ok(());
        }
        if self.transaction_active() {
            return Err(EditError::TransactionActive);
        }
        if selection.len() > crate::control::MAX_COLLECTION {
            return Err(EditError::Limit);
        }
        let existing: BTreeSet<_> = self
            .project()
            .data()
            .scene()
            .timeline
            .iter()
            .flat_map(|t| t.items.iter().map(|i| i.id))
            .collect();
        if let Some(id) = selection.iter().find(|id| !existing.contains(id)) {
            return Err(EditError::NoItem(*id));
        }
        self.end_merge();
        self.state.selection = Arc::new(selection);
        Ok(())
    }
    fn prune_selection(&mut self) {
        let existing: BTreeSet<_> = self
            .project()
            .data()
            .scene()
            .timeline
            .iter()
            .flat_map(|t| t.items.iter().map(|i| i.id))
            .collect();
        if self.state.selection.iter().any(|id| !existing.contains(id)) {
            Arc::make_mut(&mut self.state.selection).retain(|id| existing.contains(id));
        }
    }
    pub fn can_undo(&self) -> bool {
        !self.transaction_active() && !self.state.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.transaction_active() && !self.state.redo.is_empty()
    }
    pub fn history_len(&self) -> (usize, usize) {
        (self.state.undo.len(), self.state.redo.len())
    }
    pub fn gesture_active(&self) -> bool {
        self.gesture.is_some()
    }
    pub fn transaction_active(&self) -> bool {
        self.gesture.is_some() || self.grouping
    }
    /// End coalescing at gesture, selection or save boundaries. Finish/cancel an explicit
    /// gesture before saving its preview or changing the selection it edits.
    pub fn end_merge(&mut self) {
        self.state.merge_key = None;
        if !self.transaction_active() {
            self.state.trim_sources.clear();
        }
    }
    /// Drain the latest committed project change. One slot coalesces slow consumers;
    /// previews/groups defer notification until commit and cannot drain older events.
    pub fn take_change(&mut self) -> Option<Arc<Project>> {
        if self.transaction_active() {
            None
        } else {
            self.state.change.take()
        }
    }
    /// Start one preview gesture. Nested gestures/groups are refused without changing state.
    pub fn begin_gesture(&mut self) -> Result<()> {
        self.apply(Command::BeginGesture {}).map(|_| ())
    }
    fn start_gesture(&mut self) -> Result<Outcome> {
        if self.transaction_active() {
            return Err(EditError::TransactionActive);
        }
        self.end_merge();
        self.gesture = Some(self.state.clone());
        Ok(Outcome {
            changed: false,
            created: Vec::new(),
            placements: Vec::new(),
        })
    }
    /// Publish a gesture as exactly one bounded undo step and one change event.
    pub fn commit_gesture(&mut self) -> Result<Outcome> {
        self.apply(Command::CommitGesture {})
    }
    fn finish_gesture(&mut self) -> Result<Outcome> {
        let before = self.gesture.take().ok_or(EditError::NoGesture)?;
        self.squash(before)
    }
    /// Restore project, history and deferred event to gesture entry. Preview IDs are
    /// retired so stale preview references cannot alias future items.
    pub fn cancel_gesture(&mut self) -> Result<Outcome> {
        self.apply(Command::CancelGesture {})
    }
    fn revert_gesture(&mut self) -> Result<Outcome> {
        let before = self.gesture.take().ok_or(EditError::NoGesture)?;
        let changed = *self.state.project != *before.project;
        let next_id = self.state.next_id;
        self.state = before;
        self.state.next_id = next_id;
        self.end_merge();
        self.invalidate_revision(false)?;
        Ok(Outcome {
            changed,
            created: Vec::new(),
            placements: Vec::new(),
        })
    }
    /// Stage commands as one undo step. An error or unwind restores project, undo/redo,
    /// merging and deferred events. Nested transactions/gestures and undo/redo are refused.
    /// Successful child IDs remain retired even if the outer group rolls back.
    pub fn transaction<T>(&mut self, run: impl FnOnce(&mut Self) -> Result<T>) -> Result<T> {
        if self.transaction_active() {
            return Err(EditError::TransactionActive);
        }
        self.generation.checked_add(1).ok_or(EditError::Limit)?;
        let before = self.state.clone();
        self.state.trim_sources.clear();
        self.grouping = true;
        let result = catch_unwind(AssertUnwindSafe(|| run(self)));
        self.grouping = false;
        match result {
            Ok(Ok(value)) => {
                self.squash(before)?;
                Ok(value)
            }
            failed => {
                // Targets captured by child commands belong to discarded previews.
                self.invalidate_revision(false)?;
                let next_id = self.state.next_id;
                self.state = before;
                self.state.next_id = next_id;
                match failed {
                    Ok(Err(error)) => Err(error),
                    _ => Err(EditError::Panicked),
                }
            }
        }
    }
    fn squash(&mut self, before: State) -> Result<Outcome> {
        // Close the preview lifetime even for a net-zero group/gesture.
        self.generation.checked_add(1).ok_or(EditError::Limit)?;
        let project = self.state.project.clone();
        let next_id = self.state.next_id;
        let changed = *project != *before.project;
        self.state = before;
        self.state.next_id = next_id;
        self.end_merge();
        if changed {
            self.publish(project, None);
        }
        self.invalidate_revision(changed)?;
        Ok(Outcome {
            changed,
            created: Vec::new(),
            placements: Vec::new(),
        })
    }
    /// Strict JSON dispatch. All timing values are integer ticks; malformed/oversized
    /// parameters fail without leaking user text or changing transaction state.
    pub fn execute(&mut self, command_id: &str, params: Value) -> Result<Outcome> {
        self.boundary(|session| {
            let command = crate::control::decode_command(command_id, params)?;
            session.commit(command, None, true)
        })
    }
    pub fn apply(&mut self, command: Command) -> Result<Outcome> {
        self.apply_merged(command, None)
    }
    /// Equal keys coalesce consecutive changed edits. Explicit gestures group all their
    /// commands regardless of key. Refusal/unwind preserves the previous merge state.
    pub fn apply_merged(&mut self, command: Command, key: Option<&str>) -> Result<Outcome> {
        self.boundary(|session| session.commit(command, key, false))
    }
    /// Control and JSON admission share the candidate invariant; typed
    /// in-process editing retains the owner's full integer domain.
    pub(crate) fn apply_control(&mut self, command: Command) -> Result<Outcome> {
        self.boundary(|session| session.commit(command, None, true))
    }
    fn boundary<T>(&mut self, run: impl FnOnce(&mut Self) -> Result<T>) -> Result<T> {
        let before = self.state.clone();
        let gesture = self.gesture.clone();
        let revision = self.revision.clone();
        let event_mark = self.control_mark();
        let metadata = self.metadata.clone();
        match catch_unwind(AssertUnwindSafe(|| run(self))) {
            Ok(Ok(value)) => {
                if Arc::ptr_eq(&revision, &self.revision)
                    && !Arc::ptr_eq(&before.project, &self.state.project)
                    && let Err(error) = self.invalidate_revision(!self.transaction_active())
                {
                    self.state = before;
                    self.gesture = gesture;
                    self.metadata = metadata;
                    self.revision = revision;
                    self.restore_control_mark(event_mark);
                    return Err(error);
                }
                Ok(value)
            }
            Ok(Err(error)) => {
                self.state = before;
                self.gesture = gesture;
                self.metadata = metadata;
                self.revision = revision;
                self.restore_control_mark(event_mark);
                Err(error)
            }
            Err(_) => {
                self.state = before;
                self.gesture = gesture;
                self.metadata = metadata;
                self.revision = revision;
                self.restore_control_mark(event_mark);
                Err(EditError::Panicked)
            }
        }
    }
    fn commit(&mut self, command: Command, key: Option<&str>, wire: bool) -> Result<Outcome> {
        // Keep one generation available to close an active preview lifetime even
        // if its final child reaches exhaustion and the group must roll back.
        let closing = matches!(
            command,
            Command::CommitGesture {} | Command::CancelGesture {}
        );
        self.generation
            .checked_add(if self.transaction_active() && !closing {
                2
            } else {
                1
            })
            .ok_or(EditError::Limit)?;
        if key.is_some_and(|k| k.is_empty() || k.len() > 256) {
            return Err(EditError::InvalidParams);
        }
        match command {
            Command::DuplicateProject { .. }
            | Command::DeleteProject {}
            | Command::ThumbnailProject {}
            | Command::CreateProject { .. }
            | Command::SaveProject {} => {
                return Err(EditError::Invalid(
                    "lifecycle command requires its library or persistence owner",
                ));
            }
            Command::SwitchScene { scene } => {
                if self.transaction_active() {
                    return Err(EditError::TransactionActive);
                }
                if self.project().data().scenes.get(scene).is_none() {
                    return Err(EditError::Invalid("scene not found"));
                }
                let mut data = self.project().data().clone();
                if wire {
                    crate::control::validate_wire(&data)?;
                }
                let changed = data.current_scene_id != scene;
                if changed {
                    data.current_scene_id = scene;
                    if wire {
                        crate::control::validate_wire(&data)?;
                    }
                    self.change_scene_view(&data);
                    self.state.project = Arc::new(Project::new(data)?);
                    self.state.selection = Arc::default();
                    self.end_merge();
                    self.state.change = Some(self.state.project.clone());
                }
                return Ok(Outcome {
                    changed,
                    created: Vec::new(),
                    placements: Vec::new(),
                });
            }
            Command::SetViewState {
                zoom,
                scroll,
                playhead,
            } => {
                if self.transaction_active() {
                    return Err(EditError::TransactionActive);
                }
                if wire {
                    crate::control::validate_wire(self.project().data())?;
                }
                let view = project::ViewState {
                    zoom,
                    scroll,
                    playhead,
                };
                view.validate()?;
                let changed = self.metadata.view != view;
                self.metadata.view = view;
                if changed {
                    self.invalidate_revision(false)?;
                }
                return Ok(Outcome {
                    changed,
                    created: Vec::new(),
                    placements: Vec::new(),
                });
            }
            Command::BeginGesture {} => {
                if wire {
                    crate::control::validate_wire(self.project().data())?;
                }
                return self.start_gesture();
            }
            Command::CommitGesture {} => {
                if wire {
                    crate::control::validate_wire(self.project().data())?;
                }
                return self.finish_gesture();
            }
            Command::CancelGesture {} => {
                if wire && let Some(before) = &self.gesture {
                    crate::control::validate_wire(before.project.data())?;
                }
                return self.revert_gesture();
            }
            Command::Undo {} => return self.history(false, wire),
            Command::Redo {} => return self.history(true, wire),
            _ => {}
        }
        edit::check_budget(&command)?;
        let candidate = self.state.project.data().clone();
        let mut created = Vec::new();
        let mut next_id = self.state.next_id;
        let mut sources = if self.transaction_active()
            || (key.is_some() && self.state.merge_key.as_deref() == key)
        {
            self.state.trim_sources.clone()
        } else {
            edit::TrimSources::new()
        };
        let mut copied_text_bytes = 0;
        let next = edit::apply(
            candidate,
            command,
            &mut created,
            &mut next_id,
            &mut sources,
            &mut copied_text_bytes,
        )?;
        if wire {
            // The batch is still one local candidate: refuse before publishing,
            // allocating persistent IDs, changing history or advancing revision.
            crate::control::validate_wire(next.data())?;
            crate::control::validate_wire(&created)?;
        }
        if next == *self.state.project {
            if self.state.merge_key.as_deref() != key {
                self.end_merge();
            }
            if self.transaction_active() || key.is_some() {
                self.state.trim_sources = sources;
            } else {
                self.state.trim_sources.clear();
            }
            return Ok(Outcome {
                changed: false,
                created: Vec::new(),
                placements: Vec::new(),
            });
        }
        let next = Arc::new(next);
        if self.transaction_active() {
            self.state.project = next;
            self.end_merge();
        } else {
            self.publish(next, key);
        }
        if self.transaction_active() || key.is_some() {
            self.state.trim_sources = sources;
        } else {
            self.state.trim_sources.clear();
        }
        self.state.next_id = next_id;
        let allocated: std::collections::BTreeSet<_> = created
            .iter()
            .filter_map(|id| match id {
                crate::CreatedId::Item(id) => Some(*id),
                _ => None,
            })
            .collect();
        let locations: std::collections::BTreeMap<_, _> = if allocated.is_empty() {
            Default::default()
        } else {
            self.project()
                .data()
                .scene()
                .timeline
                .iter()
                .flat_map(|track| {
                    track
                        .items
                        .iter()
                        .filter(|item| allocated.contains(&item.id))
                        .map(|item| (item.id, track.id))
                })
                .collect()
        };
        let placements = created
            .iter()
            .filter_map(|id| {
                let crate::CreatedId::Item(id) = id else {
                    return None;
                };
                locations.get(id).map(|track| crate::ItemPlacement {
                    track: *track,
                    item: *id,
                })
            })
            .collect();
        Ok(Outcome {
            changed: true,
            created,
            placements,
        })
    }
    fn change_scene_view(&mut self, next: &project::ProjectData) {
        let old = self.project().data().current_scene_id;
        if old != next.current_scene_id {
            self.metadata.scene_views.insert(old, self.metadata.view);
            self.metadata.view = self
                .metadata
                .scene_views
                .get(&next.current_scene_id)
                .copied()
                .unwrap_or_default();
        }
        self.metadata
            .scene_views
            .retain(|id, _| next.scenes.get(*id).is_some());
    }
    fn publish(&mut self, next: Arc<Project>, key: Option<&str>) {
        if key.is_none() || self.state.merge_key.as_deref() != key {
            self.state.undo.push_back(Snapshot {
                project: self.state.project.clone(),
                selection: self.state.selection.clone(),
            });
            if self.state.undo.len() > self.limit {
                self.state.undo.pop_front();
            }
        }
        self.change_scene_view(next.data());
        self.state.project = next;
        self.prune_selection();
        self.state.redo.clear();
        self.state.merge_key = key.map(str::to_owned);
        self.state.change = Some(self.state.project.clone());
    }
    pub fn undo(&mut self) -> Result<Outcome> {
        self.apply(Command::Undo {})
    }
    pub fn redo(&mut self) -> Result<Outcome> {
        self.apply(Command::Redo {})
    }
    fn history(&mut self, redo: bool, wire: bool) -> Result<Outcome> {
        if self.transaction_active() {
            return Err(EditError::TransactionActive);
        }
        if wire {
            let candidate = if redo {
                self.state.redo.last()
            } else {
                self.state.undo.back()
            }
            .ok_or(EditError::NoHistory)?;
            crate::control::validate_wire(candidate.project.data())?;
        }
        let mut snapshot = if redo {
            self.state.redo.pop()
        } else {
            self.state.undo.pop_back()
        }
        .ok_or(EditError::NoHistory)?;
        let current = self.project().data().current_scene_id;
        if snapshot.project.data().current_scene_id != current
            && snapshot.project.data().scenes.get(current).is_some()
        {
            let mut data = snapshot.project.data().clone();
            data.current_scene_id = current;
            snapshot.project = Arc::new(Project::new(data)?);
        }
        if wire {
            crate::control::validate_wire(snapshot.project.data())?;
        }
        self.change_scene_view(snapshot.project.data());
        // Each entry remembers the selection that produced its edit, even if the
        // user subsequently deselected everything before invoking history.
        let old = Snapshot {
            project: std::mem::replace(&mut self.state.project, snapshot.project),
            selection: snapshot.selection.clone(),
        };
        self.state.selection = snapshot.selection;
        self.prune_selection();
        if redo {
            self.state.undo.push_back(old);
        } else {
            self.state.redo.push(old);
        }
        self.end_merge();
        self.state.change = Some(self.state.project.clone());
        Ok(Outcome {
            changed: true,
            created: Vec::new(),
            placements: Vec::new(),
        })
    }
}
// The caller already owns Value; this caps additional processing, not its creation.
pub(crate) fn budget(value: &Value) -> Result<()> {
    let mut stack = vec![(value, 0)];
    let mut nodes = 0usize;
    let mut bytes = 0usize;
    while let Some((value, depth)) = stack.pop() {
        nodes += 1;
        if nodes > crate::control::MAX_NODES || depth > crate::control::MAX_DEPTH {
            return Err(EditError::Limit);
        }
        match value {
            Value::Array(a) => {
                if a.len() > crate::control::MAX_COLLECTION
                    || nodes + stack.len() + a.len() > crate::control::MAX_NODES
                {
                    return Err(EditError::Limit);
                }
                stack.extend(a.iter().map(|v| (v, depth + 1)));
            }
            Value::Object(o) => {
                if o.len() > crate::control::MAX_COLLECTION
                    || nodes + stack.len() + o.len() > crate::control::MAX_NODES
                {
                    return Err(EditError::Limit);
                }
                for (k, v) in o {
                    bytes = bytes.saturating_add(k.len());
                    stack.push((v, depth + 1));
                }
            }
            Value::Number(n)
                if n.as_i64().is_some_and(|v| {
                    !(-crate::control::MAX_SAFE_INTEGER..=crate::control::MAX_SAFE_INTEGER)
                        .contains(&v)
                }) || n
                    .as_u64()
                    .is_some_and(|v| v > crate::control::MAX_SAFE_INTEGER as u64)
                    || n.as_f64()
                        .is_some_and(|v| v.abs() > crate::control::MAX_SAFE_INTEGER as f64) =>
            {
                return Err(EditError::InvalidParams);
            }
            Value::String(s) => bytes = bytes.saturating_add(s.len()),
            _ => {}
        }
        if bytes > crate::control::MAX_TEXT_BYTES {
            return Err(EditError::Limit);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unwind_after_staged_publication_restores_command_state() {
        let mut session =
            Session::new(Project::empty(project::ProjectId(0), "".into()).unwrap(), 1).unwrap();
        let before = session.state.clone();
        let result: Result<()> = session.boundary(|s| {
            s.commit(
                Command::AddTrack {
                    placement: crate::TrackPlacement::Auto,
                    name: "".into(),
                    kind: project::TrackKind::Video,
                },
                None,
                false,
            )?;
            panic!("injected command unwind");
        });
        assert!(matches!(result, Err(EditError::Panicked)));
        assert!(Arc::ptr_eq(&before.project, session.project()));
        assert_eq!(session.history_len(), (0, 0));
        assert!(session.take_change().is_none());
        assert_eq!(session.state.next_id, before.next_id);
        assert_eq!(session.state.merge_key, before.merge_key);
    }
}

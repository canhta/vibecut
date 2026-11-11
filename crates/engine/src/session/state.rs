//! Owns editable session history.
use super::edit;
mod checkpoint;
mod session_control;
use crate::{Command, EditError, NewItem, Outcome, Placement, Result};
use checkpoint::Checkpoint;
use project::{AudioParams, AudioSource, ItemId, ItemParams, LibrarySound, Project};
use serde_json::Value;
use std::collections::BTreeSet;
use std::{
    collections::VecDeque,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::Arc,
};
use time::{SourceRange, Tick};

#[derive(Clone)]
struct Snapshot {
    project: Arc<Project>,
    selection: Arc<BTreeSet<ItemId>>,
}
/// A reviewed span cut needs the ripple pass wherever it is nested.
fn cuts_spans(command: &Command) -> bool {
    match command {
        Command::RemovePauses { .. }
        | Command::TranscriptDeleteRange { .. }
        | Command::RemoveFillers { .. } => true,
        Command::Batch { commands } => commands.iter().any(cuts_spans),
        _ => false,
    }
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
impl State {
    /// Return to `before` while keeping the ID counter: a gesture or transaction
    /// already returned its children's IDs, so they stay retired.
    fn rewind(&mut self, before: State) {
        let next_id = self.next_id;
        *self = before;
        self.next_id = next_id;
    }
}
/// Fill an identity or private nonce without exposing the platform entropy failure.
pub(crate) fn identity_entropy(bytes: &mut [u8]) -> std::result::Result<(), super::SessionError> {
    getrandom::fill(bytes).map_err(|_| super::SessionError::ControlIdentity)
}

/// Opaque identity for exactly one Session revision, including save boundaries.
#[derive(Clone)]
pub struct JobTarget {
    session: Arc<super::wire::Revision>,
    revision: Arc<super::wire::Revision>,
}
/// Owns project editing, bounded history, deferred changes and gesture lifecycle.
pub struct Session {
    clipboard: Option<Arc<super::KeyframeClipboard>>,
    metadata: project::Metadata,
    protected: Vec<std::path::PathBuf>,
    pub(crate) identity: Arc<super::wire::Revision>,
    pub(crate) revision: Arc<super::wire::Revision>,
    pub(crate) generation: u64,
    pub(crate) ledger: super::wire::Ledger,
    state: State,
    limit: usize,
    gesture: Option<State>,
    timeline_preview: bool,
    grouping: bool,
    ripple: bool,
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
                    .chain(s.timeline.transitions.iter().map(|t| t.id.0))
                    .chain(s.timeline.iter().map(|t| t.id.0))
                    .chain(
                        s.timeline
                            .iter()
                            .flat_map(|t| t.items.iter().map(|i| i.id.0)),
                    )
                    .chain(s.bookmarks.iter().map(|b| b.id.0))
            })
            .chain(data.caption_presets.iter().map(|preset| preset.id.0))
            .max()
            .map_or(Some(0), |id| id.checked_add(1));
        let mut entropy = [0u8; 32];
        identity_entropy(&mut entropy)?;
        let identity = Arc::new(super::wire::Revision(
            blake3::hash(&entropy).to_hex().to_string(),
        ));
        Ok(Self {
            clipboard: None,
            metadata: project::Metadata::default(),
            protected: Vec::new(),
            revision: Arc::new(super::wire::Revision(format!("{}:0", identity.0))),
            identity,
            generation: 0,
            ledger: super::wire::Ledger::default(),
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
            timeline_preview: false,
            grouping: false,
            ripple: false,
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
    /// Whether this Session holds a bounded keyframe snapshot for desktop actions.
    pub fn clipboard_has_entry(&self) -> bool {
        self.clipboard.is_some()
    }
    /// Current committed selection admission for the desktop Copy action; no state changes.
    pub fn can_copy_keyframes(&self, keys: &[crate::KeyframeRef]) -> bool {
        !self.transaction_active()
            && !keys.is_empty()
            && super::KeyframeClipboard::can_capture(self.project().data(), keys)
    }
    /// Current target/lock/value/time admission for desktop Paste; zero-key Params qualify.
    pub fn can_paste_keyframes(&self, item: ItemId, at: Tick) -> bool {
        if self.transaction_active() {
            return false;
        }
        let Some(payload) = &self.clipboard else {
            return false;
        };
        self.project()
            .data()
            .scene()
            .timeline
            .iter()
            .filter(|t| !t.locked)
            .flat_map(|t| &t.items)
            .find(|i| i.id == item)
            .is_some_and(|i| payload.admit_paste(i, at).is_ok())
    }
    pub(crate) fn clipboard_payload(&self) -> Option<super::KeyframeClipboard> {
        self.clipboard.as_deref().cloned()
    }
    pub fn metadata(&self) -> &project::Metadata {
        &self.metadata
    }
    /// Register active project/metadata/recovery paths that file publication must
    /// never overwrite. Composition roots supply the same set used for export.
    pub fn protect_paths(&mut self, paths: Vec<std::path::PathBuf>) -> Result<()> {
        if paths.len() > 256
            || paths
                .iter()
                .any(|path| path.as_os_str().len() > super::wire::MAX_PATH_BYTES)
        {
            return Err(EditError::Limit);
        }
        self.protected = paths;
        Ok(())
    }
    pub fn protected_paths(&self) -> &[std::path::PathBuf] {
        &self.protected
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
    /// Validate only the original Session epoch and revision pointer identity.
    pub(crate) fn validate_job_target(
        &self,
        target: &JobTarget,
    ) -> std::result::Result<(), super::SessionError> {
        if Arc::ptr_eq(&self.identity, &target.session)
            && Arc::ptr_eq(&self.revision, &target.revision)
        {
            Ok(())
        } else {
            Err(super::SessionError::StaleResult)
        }
    }
    pub fn apply_job_result(&mut self, target: &JobTarget, command: Command) -> Result<Outcome> {
        self.validate_job_target(target)?;
        if self.transaction_active() {
            return Err(EditError::TransactionActive);
        }
        self.apply(command)
    }
    /// Current immutable snapshot. Timeline previews retain committed content here;
    /// explicit incremental gestures/groups expose their candidate during the transaction.
    pub fn project(&self) -> &Arc<Project> {
        if self.timeline_preview
            && let Some(before) = &self.gesture
        {
            return &before.project;
        }
        &self.state.project
    }
    /// Uncommitted gesture snapshot for timeline, transform and graph rendering.
    /// `timeline.preview` keeps `project()` byte-identical until commit.
    pub fn preview_project(&self) -> Option<&Arc<Project>> {
        self.gesture.as_ref().map(|_| &self.state.project)
    }
    /// Adopt an acknowledged Save As without adding or discarding history.
    pub(crate) fn adopt_saved_project(&mut self, project: Arc<Project>) {
        if self.project().data().id != project.data().id
            || self.project().data().current_scene_id != project.data().current_scene_id
        {
            self.clipboard = None;
        }
        self.state.project = project;
        self.state.change = Some(self.state.project.clone());
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
        if selection.len() > super::wire::MAX_COLLECTION {
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
    /// Publish a gesture as exactly one bounded undo step and one change event.
    pub fn commit_gesture(&mut self) -> Result<Outcome> {
        self.apply(Command::CommitGesture {})
    }
    /// Restore project, history and deferred event to gesture entry. Preview IDs are
    /// retired so stale preview references cannot alias future items.
    pub fn cancel_gesture(&mut self) -> Result<Outcome> {
        self.apply(Command::CancelGesture {})
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
                self.state.rewind(before);
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
        let identity_changed = before.project.data().id != self.state.project.data().id
            || before.project.data().current_scene_id != self.state.project.data().current_scene_id;
        let project = self.state.project.clone();
        let changed = *project != *before.project;
        self.state.rewind(before);
        self.end_merge();
        if changed {
            self.publish(project, None);
        }
        self.invalidate_revision(changed)?;
        if identity_changed {
            self.clipboard = None;
        }
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
            let command = super::wire::decode_command(command_id, params)?;
            session.commit(command, None, true, session.ripple)
        })
    }
    pub fn apply(&mut self, command: Command) -> Result<Outcome> {
        self.apply_merged(command, None)
    }
    /// Apply one atomic edit using the caller's explicit ripple policy. The policy
    /// also covers replacement previews; it never reads an app preference store.
    pub fn apply_with_ripple(&mut self, command: Command, ripple: bool) -> Result<Outcome> {
        self.boundary(|session| session.commit(command, None, false, ripple))
    }
    /// Strict JSON dispatch with an explicit ripple policy for this edit only.
    pub fn execute_with_ripple(
        &mut self,
        id: &str,
        params: Value,
        ripple: bool,
    ) -> Result<Outcome> {
        self.boundary(|session| {
            let command = super::wire::decode_command(id, params)?;
            session.commit(command, None, true, ripple)
        })
    }
    /// Equal keys coalesce consecutive changed edits. Explicit gestures group all their
    /// commands regardless of key. Refusal/unwind preserves the previous merge state.
    pub fn apply_merged(&mut self, command: Command, key: Option<&str>) -> Result<Outcome> {
        self.boundary(|session| session.commit(command, key, false, session.ripple))
    }
    /// Control and JSON admission share the candidate invariant; typed
    /// in-process editing retains the owner's full integer domain.
    pub(crate) fn apply_control(&mut self, command: Command) -> Result<Outcome> {
        self.boundary(|session| session.commit(command, None, true, session.ripple))
    }
    /// Run `run` atomically: a refusal or an unwind leaves the session as
    /// [`Checkpoint`] captured it.
    fn boundary<T>(&mut self, run: impl FnOnce(&mut Self) -> Result<T>) -> Result<T> {
        let checkpoint = Checkpoint::take(self);
        let result = match catch_unwind(AssertUnwindSafe(|| run(self))) {
            Ok(Ok(value)) => {
                if checkpoint.project_changed_at_same_revision(self) {
                    self.invalidate_revision(!self.transaction_active())
                        .map(|()| value)
                } else {
                    Ok(value)
                }
            }
            Ok(Err(error)) => Err(error),
            Err(_) => Err(EditError::Panicked),
        };
        if result.is_err() {
            checkpoint.restore(self);
        } else if !self.transaction_active() && checkpoint.identity_changed(self) {
            self.clipboard = None;
        }
        result
    }
    /// One undo step: the asset and its audio item commit or roll back together.
    /// The caller supplies an already cached, probed media file and provider
    /// metadata; no network or cache IO happens while a project transaction is open.
    fn add_library_sound(
        &mut self,
        placement: Placement,
        start: Tick,
        asset: media::SourceAsset,
        sound: LibrarySound,
    ) -> Result<Outcome> {
        let duration = asset
            .probe()
            .duration
            .ok_or(EditError::Invalid("library sound duration"))?;
        if duration <= Tick::ZERO {
            return Err(EditError::Invalid("library sound duration"));
        }
        let source_asset = asset.id();
        let mut created = Vec::new();
        let mut placements = Vec::new();
        self.transaction(|session| {
            created.extend(
                session
                    .apply(Command::AddAsset {
                        asset: asset.clone(),
                    })?
                    .created,
            );
            let outcome = session.apply(Command::InsertItem {
                placement,
                item: NewItem {
                    retime: Default::default(),
                    animations: project::Animations::default(),
                    source_asset: Some(source_asset),
                    source_range: SourceRange::new(Tick::ZERO, duration)?,
                    start,
                    content: ItemParams::Audio(AudioParams {
                        fades: Default::default(),
                        volume: 0.0,
                        muted: false,
                        source: AudioSource::Library { sound },
                    }),
                },
            })?;
            created.extend(outcome.created);
            placements.extend(outcome.placements);
            Ok(())
        })?;
        Ok(Outcome {
            changed: true,
            created,
            placements,
        })
    }

    fn commit(
        &mut self,
        command: Command,
        key: Option<&str>,
        wire: bool,
        ripple: bool,
    ) -> Result<Outcome> {
        let (command, ripple) = match command {
            Command::TimelineEdit { commands, ripple } => {
                let command = Command::Batch { commands };
                edit::check_budget(&command)?;
                (command, ripple)
            }
            command => (command, ripple),
        };
        let command = match command {
            Command::AddLibrarySound {
                placement,
                start,
                asset,
                sound,
            } => return self.add_library_sound(placement, start, asset, sound),
            command => command,
        };
        // A reviewed span cut always closes its vacated tails; the caller cannot
        // turn the ripple path off, including inside a batch.
        let ripple = ripple || cuts_spans(&command);
        // Keep one generation available to close an active preview lifetime even
        // if its final child reaches exhaustion and the group must roll back.
        let closing = matches!(
            command,
            Command::CommitGesture {}
                | Command::CancelGesture {}
                | Command::TimelineCommitPreview {}
                | Command::TimelineDiscardPreview {}
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
        if command.lifecycle().is_some() {
            return Err(EditError::Invalid(
                "lifecycle command requires its library or persistence owner",
            ));
        }
        if matches!(command.effect(), crate::Effect::SessionControl) {
            return self.session_control(command, wire, ripple);
        }
        edit::check_budget(&command)?;
        let mut command = command;
        if ripple {
            edit::lift_ripple_deletes(&mut command);
        }
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
            self.clipboard.as_deref(),
        )?;
        let next = if ripple {
            edit::ripple(self.state.project.data(), next)?
        } else {
            next
        };
        if wire {
            // The batch is still one local candidate: refuse before publishing,
            // allocating persistent IDs, changing history or advancing revision.
            super::wire::validate_wire(next.data())?;
            super::wire::validate_wire(&created)?;
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
            self.state
                .project
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
}
#[cfg(test)]
mod tests {
    use super::*;

    fn add_track() -> Command {
        Command::AddTrack {
            placement: crate::TrackPlacement::Auto,
            name: "".into(),
            kind: project::TrackKind::Video,
        }
    }
    fn view(zoom: f64) -> Command {
        Command::SetViewState {
            zoom,
            scroll: 0.0,
            playhead: Tick::ZERO,
        }
    }

    /// Each exit of a boundary that is not success leaves every restored field
    /// as it was at entry, after the body changed all of them.
    #[test]
    fn refused_boundary_restores_every_checkpointed_field() {
        #[derive(Clone, Copy, Debug)]
        enum Exit {
            Refusal,
            Unwind,
            RevisionExhausted,
        }
        for exit in [Exit::Refusal, Exit::Unwind, Exit::RevisionExhausted] {
            let mut session =
                Session::new(Project::empty(project::ProjectId(0), "".into()).unwrap(), 4).unwrap();
            session.apply(add_track()).unwrap();
            session.apply(view(2.0)).unwrap();
            let payload = crate::KeyframeClipboard::new(
                Tick::ZERO,
                vec![(
                    project::BuiltinPath::Opacity.into(),
                    project::Param::Value(
                        project::Channel::new(
                            project::ParamValue::Scalar(0.7),
                            vec![project::Keyframe::new(
                                Tick::ZERO,
                                project::ParamValue::Scalar(0.25),
                            )],
                            project::Extrapolation::Hold,
                            project::Extrapolation::Hold,
                        )
                        .unwrap(),
                    ),
                )],
            )
            .unwrap();
            session.clipboard = Some(Arc::new(payload));
            let clipboard = session.clipboard.clone();
            let state = session.state.clone();
            let revision = session.revision.clone();
            let metadata = session.metadata.clone();
            let ledger = (session.ledger.sequence, session.ledger.events.len());
            let result: Result<()> = session.boundary(|s| {
                // Transient snapshot custody must rewind with the revision.
                s.clipboard = None;
                // Project, history, IDs, selection and the deferred change.
                s.commit(add_track(), None, false, false)?;
                if matches!(exit, Exit::RevisionExhausted) {
                    // The body changed the project without advancing the
                    // revision; the boundary then cannot advance it either.
                    s.generation = u64::MAX;
                    return Ok(());
                }
                // Metadata, revision and ledger events.
                s.commit(view(3.0), None, false, false)?;
                // Ripple policy.
                s.commit(
                    Command::TimelineSetRipple { enabled: true },
                    None,
                    false,
                    false,
                )?;
                // Gesture entry and the staged preview flag.
                s.commit(
                    Command::TimelinePreview {
                        commands: vec![add_track()],
                    },
                    None,
                    false,
                    false,
                )?;
                assert!(s.gesture.is_some() && s.timeline_preview && s.ripple);
                assert!(!Arc::ptr_eq(&s.revision, &revision));
                match exit {
                    Exit::Unwind => panic!("injected command unwind"),
                    _ => Err(EditError::Invalid("injected refusal")),
                }
            });
            match exit {
                Exit::Refusal => assert!(matches!(result, Err(EditError::Invalid(_))), "{exit:?}"),
                Exit::Unwind => assert!(matches!(result, Err(EditError::Panicked)), "{exit:?}"),
                Exit::RevisionExhausted => {
                    assert!(matches!(result, Err(EditError::Limit)), "{exit:?}")
                }
            }
            assert!(
                Arc::ptr_eq(&state.project, &session.state.project),
                "{exit:?}"
            );
            assert_eq!(session.clipboard, clipboard, "{exit:?}");
            assert_eq!(session.history_len(), (state.undo.len(), 0), "{exit:?}");
            assert_eq!(session.state.next_id, state.next_id, "{exit:?}");
            assert_eq!(session.state.merge_key, state.merge_key, "{exit:?}");
            assert!(
                Arc::ptr_eq(&state.selection, &session.state.selection),
                "{exit:?}"
            );
            assert_eq!(
                session.state.change.as_ref().map(Arc::as_ptr),
                state.change.as_ref().map(Arc::as_ptr),
                "{exit:?}"
            );
            assert!(
                session.gesture.is_none() && !session.timeline_preview,
                "{exit:?}"
            );
            assert!(!session.ripple, "{exit:?}");
            assert!(Arc::ptr_eq(&revision, &session.revision), "{exit:?}");
            assert!(session.metadata == metadata, "{exit:?}");
            assert_eq!(
                (session.ledger.sequence, session.ledger.events.len()),
                ledger,
                "{exit:?}"
            );
        }
    }
}

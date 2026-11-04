use crate::{Command, EditError, Outcome, Result, edit, registry};
use project::Project;
use serde_json::Value;
use std::{
    collections::VecDeque,
    panic::{AssertUnwindSafe, catch_unwind},
    sync::Arc,
};

#[derive(Clone)]
struct State {
    project: Arc<Project>,
    undo: VecDeque<Arc<Project>>,
    redo: Vec<Arc<Project>>,
    merge_key: Option<String>,
    trim_sources: edit::TrimSources,
    next_id: Option<u64>,
    change: Option<Arc<Project>>,
}
/// Opaque identity for exactly one Session revision, including save boundaries.
#[derive(Clone)]
pub struct JobTarget {
    session: Arc<()>,
    revision: Arc<()>,
}
/// Owns project editing, bounded history, deferred changes and gesture lifecycle.
pub struct Session {
    identity: Arc<()>,
    revision: Arc<()>,
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
            .timeline
            .tracks
            .iter()
            .map(|t| t.id.0)
            .chain(
                data.timeline
                    .tracks
                    .iter()
                    .flat_map(|t| t.items.iter().map(|i| i.id.0)),
            )
            .chain(data.bookmarks.iter().map(|b| b.id.0))
            .max()
            .map_or(Some(0), |id| id.checked_add(1));
        Ok(Self {
            identity: Arc::new(()),
            revision: Arc::new(()),
            state: State {
                project: Arc::new(project),
                undo: VecDeque::new(),
                redo: Vec::new(),
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
        self.end_merge();
        self.revision = Arc::new(());
        Ok(())
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
        })
    }
    /// Publish a gesture as exactly one bounded undo step and one change event.
    pub fn commit_gesture(&mut self) -> Result<Outcome> {
        self.apply(Command::CommitGesture {})
    }
    fn finish_gesture(&mut self) -> Result<Outcome> {
        let before = self.gesture.take().ok_or(EditError::NoGesture)?;
        Ok(self.squash(before))
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
        Ok(Outcome {
            changed,
            created: Vec::new(),
        })
    }
    /// Stage commands as one undo step. An error or unwind restores project, undo/redo,
    /// merging and deferred events. Nested transactions/gestures and undo/redo are refused.
    /// Successful child IDs remain retired even if the outer group rolls back.
    pub fn transaction<T>(&mut self, run: impl FnOnce(&mut Self) -> Result<T>) -> Result<T> {
        if self.transaction_active() {
            return Err(EditError::TransactionActive);
        }
        let before = self.state.clone();
        self.state.trim_sources.clear();
        self.grouping = true;
        let result = catch_unwind(AssertUnwindSafe(|| run(self)));
        self.grouping = false;
        match result {
            Ok(Ok(value)) => {
                self.squash(before);
                Ok(value)
            }
            failed => {
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
    fn squash(&mut self, before: State) -> Outcome {
        let project = self.state.project.clone();
        let next_id = self.state.next_id;
        let changed = *project != *before.project;
        self.state = before;
        self.state.next_id = next_id;
        self.end_merge();
        if changed {
            self.publish(project, None);
        }
        Outcome {
            changed,
            created: Vec::new(),
        }
    }
    /// Strict JSON dispatch. All timing values are integer ticks; malformed/oversized
    /// parameters fail without leaking user text or changing transaction state.
    pub fn execute(&mut self, command_id: &str, params: Value) -> Result<Outcome> {
        self.boundary(|session| {
            if !registry().iter().any(|s| s.id == command_id) {
                return Err(EditError::UnknownCommand);
            }
            budget(&params)?;
            let command =
                serde_json::from_value(serde_json::json!({"command":command_id,"params":params}))
                    .map_err(|_| EditError::InvalidParams)?;
            session.commit(command, None)
        })
    }
    pub fn apply(&mut self, command: Command) -> Result<Outcome> {
        self.apply_merged(command, None)
    }
    /// Equal keys coalesce consecutive changed edits. Explicit gestures group all their
    /// commands regardless of key. Refusal/unwind preserves the previous merge state.
    pub fn apply_merged(&mut self, command: Command, key: Option<&str>) -> Result<Outcome> {
        self.boundary(|session| session.commit(command, key))
    }
    fn boundary<T>(&mut self, run: impl FnOnce(&mut Self) -> Result<T>) -> Result<T> {
        let before = self.state.clone();
        let gesture = self.gesture.clone();
        match catch_unwind(AssertUnwindSafe(|| run(self))) {
            Ok(Ok(value)) => {
                if !Arc::ptr_eq(&before.project, &self.state.project) {
                    self.revision = Arc::new(());
                }
                Ok(value)
            }
            Ok(Err(error)) => {
                self.state = before;
                self.gesture = gesture;
                Err(error)
            }
            Err(_) => {
                self.state = before;
                self.gesture = gesture;
                Err(EditError::Panicked)
            }
        }
    }
    fn commit(&mut self, command: Command, key: Option<&str>) -> Result<Outcome> {
        if key.is_some_and(|k| k.is_empty() || k.len() > 256) {
            return Err(EditError::InvalidParams);
        }
        match command {
            Command::BeginGesture {} => return self.start_gesture(),
            Command::CommitGesture {} => return self.finish_gesture(),
            Command::CancelGesture {} => return self.revert_gesture(),
            Command::Undo {} => return self.history(false),
            Command::Redo {} => return self.history(true),
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
        let next = edit::apply(candidate, command, &mut created, &mut next_id, &mut sources)?;
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
        Ok(Outcome {
            changed: true,
            created,
        })
    }
    fn publish(&mut self, next: Arc<Project>, key: Option<&str>) {
        if key.is_none() || self.state.merge_key.as_deref() != key {
            self.state.undo.push_back(self.state.project.clone());
            if self.state.undo.len() > self.limit {
                self.state.undo.pop_front();
            }
        }
        self.state.project = next;
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
    fn history(&mut self, redo: bool) -> Result<Outcome> {
        if self.transaction_active() {
            return Err(EditError::TransactionActive);
        }
        let snapshot = if redo {
            self.state.redo.pop()
        } else {
            self.state.undo.pop_back()
        }
        .ok_or(EditError::NoHistory)?;
        let old = std::mem::replace(&mut self.state.project, snapshot);
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
        })
    }
}
// The caller already owns Value; this caps additional processing, not its creation.
fn budget(value: &Value) -> Result<()> {
    let mut stack = vec![(value, 0)];
    let mut nodes = 0usize;
    let mut bytes = 0usize;
    while let Some((value, depth)) = stack.pop() {
        nodes += 1;
        if nodes > 200_000 || depth > 32 {
            return Err(EditError::Limit);
        }
        match value {
            Value::Array(a) => {
                if a.len() > 100_000 || nodes + stack.len() + a.len() > 200_000 {
                    return Err(EditError::Limit);
                }
                stack.extend(a.iter().map(|v| (v, depth + 1)));
            }
            Value::Object(o) => {
                if o.len() > 100_000 || nodes + stack.len() + o.len() > 200_000 {
                    return Err(EditError::Limit);
                }
                for (k, v) in o {
                    bytes = bytes.saturating_add(k.len());
                    stack.push((v, depth + 1));
                }
            }
            Value::String(s) => bytes = bytes.saturating_add(s.len()),
            _ => {}
        }
        if bytes > 16 * 1024 * 1024 {
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
                    name: "".into(),
                    kind: project::TrackKind::Overlay,
                },
                None,
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

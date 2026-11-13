//! Executes the commands declared `SessionControl`.
use super::{Session, Snapshot};
use crate::session::{CutTarget, SessionError, edit, wire};
use crate::session::{EditError, Result};
use crate::{Command, Outcome};
use project::Project;
use std::sync::Arc;

fn outcome(changed: bool) -> Outcome {
    Outcome {
        changed,
        created: Vec::new(),
        placements: Vec::new(),
    }
}

impl Session {
    /// Run one session-control command. `commit` has already reserved the
    /// generation and refused lifecycle commands, which their owners execute.
    pub(super) fn session_control(
        &mut self,
        command: Command,
        wire: bool,
        ripple: bool,
    ) -> Result<Outcome> {
        // A fork replays onto its parent, which owns the view, history and clipboard.
        if self.recorded.is_some() {
            return Err(EditError::Unsupported("session control on a fork"));
        }
        match command {
            Command::CopyItems { items } => {
                if self.transaction_active() {
                    return Err(EditError::Session(
                        crate::session::SessionError::TransactionActive,
                    ));
                }
                if let Some(payload) = crate::session::clipboard::ItemClipboard::capture(
                    self.project().data(),
                    &items,
                )? {
                    let same = self
                        .item_clipboard
                        .as_deref()
                        .map(|stored| stored.same_bytes(&payload))
                        .transpose()?
                        .unwrap_or(false);
                    if !same {
                        self.item_clipboard = Some(Arc::new(payload));
                        self.clipboard = None;
                        self.invalidate_revision(false)?;
                    }
                }
                Ok(outcome(false))
            }
            Command::CopyKeyframes { keys } => {
                if self.transaction_active() {
                    return Err(EditError::Session(
                        crate::session::SessionError::TransactionActive,
                    ));
                }
                if let Some(payload) = crate::session::clipboard::KeyframeClipboard::capture(
                    self.project().data(),
                    &keys,
                )? {
                    let same = self
                        .clipboard
                        .as_deref()
                        .map(|stored| stored.same_bytes(&payload))
                        .transpose()?
                        .unwrap_or(false);
                    if !same {
                        self.clipboard = Some(Arc::new(payload));
                        self.item_clipboard = None;
                        self.invalidate_revision(false)?;
                    }
                }
                Ok(outcome(false))
            }
            Command::CutSelection { target } => self.cut(target),
            Command::TimelineSetRipple { enabled } => self.set_ripple(enabled),
            Command::TimelinePreview { commands } => self.preview(commands, wire, ripple),
            Command::SwitchScene { scene } => self.switch_scene(scene, wire),
            Command::SetViewState {
                zoom,
                scroll,
                playhead,
            } => self.set_view_state(
                project::ViewState {
                    zoom,
                    scroll,
                    playhead,
                },
                wire,
            ),
            Command::BeginGesture {} => {
                if wire {
                    wire::validate_wire(self.project().data())?;
                }
                Ok(self.start_gesture()?)
            }
            Command::CommitGesture {} | Command::TimelineCommitPreview {} => {
                if wire {
                    wire::validate_wire(self.state.project.data())?;
                }
                self.finish_gesture()
            }
            Command::CancelGesture {} | Command::TimelineDiscardPreview {} => {
                if wire && let Some(before) = &self.gesture {
                    wire::validate_wire(before.project.data())?;
                }
                self.revert_gesture()
            }
            Command::Undo {} => self.history(false, wire),
            Command::Redo {} => self.history(true, wire),
            _ => Err(EditError::Invalid(
                "lifecycle command requires its library or persistence owner",
            )),
        }
    }

    /// Copy the target into its clipboard, then delete it in one transaction. The
    /// payload is captured first and stored only after the delete commits, so a
    /// refused cut (locked, stale or empty) leaves the previous clipboard in place.
    /// The delete follows the session ripple setting like any other delete.
    fn cut(&mut self, target: CutTarget) -> Result<Outcome> {
        if self.transaction_active() {
            return Err(EditError::Session(SessionError::TransactionActive));
        }
        match target {
            CutTarget::Items { items } => {
                let Some(payload) = crate::session::clipboard::ItemClipboard::capture(
                    self.project().data(),
                    &items,
                )?
                else {
                    return Ok(outcome(false));
                };
                self.transaction(|session| session.apply(Command::Delete { items }))?;
                self.item_clipboard = Some(Arc::new(payload));
                self.clipboard = None;
            }
            CutTarget::Keyframes { keys } => {
                let Some(payload) = crate::session::clipboard::KeyframeClipboard::capture(
                    self.project().data(),
                    &keys,
                )?
                else {
                    return Ok(outcome(false));
                };
                self.transaction(|session| session.apply(Command::DeleteKeyframes { keys }))?;
                self.clipboard = Some(Arc::new(payload));
                self.item_clipboard = None;
            }
        }
        self.invalidate_revision(false)?;
        Ok(outcome(true))
    }

    fn set_ripple(&mut self, enabled: bool) -> Result<Outcome> {
        if self.transaction_active() {
            return Err(EditError::Session(
                crate::session::SessionError::TransactionActive,
            ));
        }
        let changed = self.ripple != enabled;
        self.ripple = enabled;
        if changed {
            self.invalidate_revision(false)?;
        }
        Ok(outcome(changed))
    }

    /// Replace the staged preview with `commands` applied to the gesture entry.
    fn preview(&mut self, commands: Vec<Command>, wire: bool, ripple: bool) -> Result<Outcome> {
        let batch = Command::Batch { commands };
        edit::check_budget(&batch)?;
        if !self.timeline_preview {
            if let Some(before) = &self.gesture {
                // The UI may promote its empty pointer gesture, but a
                // replacement cannot discard an earlier incremental edit.
                if before.project != self.state.project {
                    return Err(EditError::Session(
                        crate::session::SessionError::TransactionActive,
                    ));
                }
            } else {
                self.start_gesture()?;
            }
        }
        let before = self
            .gesture
            .as_ref()
            .ok_or(EditError::Session(crate::session::SessionError::NoGesture))?
            .clone();
        self.state.rewind(before);
        let outcome = self.commit(batch, wire, ripple)?;
        self.timeline_preview = true;
        Ok(outcome)
    }

    fn switch_scene(&mut self, scene: project::SceneId, wire: bool) -> Result<Outcome> {
        if self.transaction_active() {
            return Err(EditError::Session(
                crate::session::SessionError::TransactionActive,
            ));
        }
        if self.project().data().scenes.get(scene).is_none() {
            return Err(EditError::Invalid("scene not found"));
        }
        let mut data = self.project().data().clone();
        if wire {
            wire::validate_wire(&data)?;
        }
        let changed = data.current_scene_id != scene;
        if changed {
            data.current_scene_id = scene;
            if wire {
                wire::validate_wire(&data)?;
            }
            self.change_scene_view(&data);
            self.state.project = Arc::new(Project::new(data)?);
            self.state.selection = Arc::default();
            self.state.change = Some(self.state.project.clone());
        }
        Ok(outcome(changed))
    }

    fn set_view_state(&mut self, view: project::ViewState, wire: bool) -> Result<Outcome> {
        if self.transaction_active() {
            return Err(EditError::Session(
                crate::session::SessionError::TransactionActive,
            ));
        }
        if wire {
            wire::validate_wire(self.project().data())?;
        }
        view.validate()?;
        let changed = self.metadata.view != view;
        self.metadata.view = view;
        if changed {
            self.invalidate_revision(false)?;
        }
        Ok(outcome(changed))
    }

    fn start_gesture(&mut self) -> std::result::Result<Outcome, SessionError> {
        if self.transaction_active() {
            return Err(SessionError::TransactionActive);
        }
        self.gesture = Some(self.state.clone());
        Ok(outcome(false))
    }

    fn finish_gesture(&mut self) -> Result<Outcome> {
        let before = self
            .gesture
            .take()
            .ok_or(EditError::Session(crate::session::SessionError::NoGesture))?;
        self.timeline_preview = false;
        self.squash(before)
    }

    fn revert_gesture(&mut self) -> Result<Outcome> {
        let before = self
            .gesture
            .take()
            .ok_or(EditError::Session(crate::session::SessionError::NoGesture))?;
        self.timeline_preview = false;
        let changed = *self.state.project != *before.project;
        self.state.rewind(before);
        self.invalidate_revision(false)?;
        Ok(outcome(changed))
    }

    fn history(&mut self, redo: bool, wire: bool) -> Result<Outcome> {
        if self.transaction_active() {
            return Err(EditError::Session(
                crate::session::SessionError::TransactionActive,
            ));
        }
        if wire {
            let candidate = if redo {
                self.state.redo.last()
            } else {
                self.state.undo.back()
            }
            .ok_or(EditError::Session(crate::session::SessionError::NoHistory))?;
            wire::validate_wire(candidate.project.data())?;
        }
        let mut snapshot = if redo {
            self.state.redo.pop()
        } else {
            self.state.undo.pop_back()
        }
        .ok_or(EditError::Session(crate::session::SessionError::NoHistory))?;
        let id = self.project().data().id;
        let current = self.project().data().current_scene_id;
        if snapshot.project.data().id != id
            || (snapshot.project.data().current_scene_id != current
                && snapshot.project.data().scenes.get(current).is_some())
        {
            let mut data = snapshot.project.data().clone();
            data.id = id;
            if data.scenes.get(current).is_some() {
                data.current_scene_id = current;
            }
            snapshot.project = Arc::new(Project::new(data)?);
        }
        if wire {
            wire::validate_wire(snapshot.project.data())?;
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
        self.state.change = Some(self.state.project.clone());
        Ok(outcome(true))
    }
}

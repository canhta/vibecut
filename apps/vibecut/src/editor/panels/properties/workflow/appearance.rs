//! Applies colour picker drafts and commits.
use super::super::State;
use crate::editor::{Port, Result, Session};

impl State {
    /// Show a colour draft without touching the editable session.
    pub(super) fn preview_color(
        &mut self,
        port: &mut Port<'_>,
        command: engine::Command,
    ) -> Result<()> {
        let result = (|| -> Result<()> {
            let project = {
                let view = port.view();
                let mut draft = Session::new(view.session.project().as_ref().clone(), 1)?;
                draft.apply(command)?;
                draft.project().clone()
            };
            let at = port.view().playhead;
            port.stage_frame(at, project);
            Ok(())
        })();
        if result.is_err() {
            port.sync();
        }
        result
    }

    /// Commit a released colour as one undoable command and remember it as recent.
    pub(super) fn commit_color(
        &mut self,
        port: &mut Port<'_>,
        command: engine::Command,
        recent: Vec<project::Color>,
    ) -> Result<()> {
        let mut preferences = port.view().preferences.clone();
        preferences.set("recent_colors", serde_json::to_value(recent)?)?;
        if let Err(error) = port.apply_result(command) {
            // The engine refused: show the committed project again.
            port.sync();
            return Err(error);
        }
        if let Some(draft) = self.picker.as_mut() {
            draft.picker.original = draft.picker.color;
        }
        self.color_base = self
            .picker
            .as_ref()
            .map(|_| port.view().session.project().clone());
        port.adopt_preferences(preferences);
        Ok(())
    }
}

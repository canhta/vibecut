//! Lends one write context to an area for one frame.
use crate::editor::{Arc, Command, Editor, Tick};

/// A cross-area request the frame drains after the area that raised it returns.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Request {
    /// Move the shared transport.
    Seek(Tick),
    /// Open the model manager on a missing model.
    OpenModel(String),
}

/// The shared editor context. One area receives it for one frame; its edit
/// entry pushes the selection, applies the command, reports the failure and
/// resyncs the preview once per committed change.
pub(crate) struct Desk<'a> {
    editor: &'a mut Editor,
}

impl<'a> Desk<'a> {
    pub(in crate::editor) fn new(editor: &'a mut Editor) -> Self {
        Self { editor }
    }

    /// Show a creator-facing failure on the editor's error line.
    pub(in crate::editor) fn fail(&mut self, message: impl Into<String>) {
        self.editor.error = Some(message.into());
    }

    /// Clear the error line after a successful retry.
    pub(in crate::editor) fn clear_error(&mut self) {
        self.editor.error = None;
    }

    /// Resync the preview with the current project snapshot.
    pub(in crate::editor) fn sync(&mut self) {
        self.editor.sync_preview();
    }

    /// Push the editor's selection into the engine copy.
    pub(in crate::editor) fn select(&mut self) -> bool {
        match self.push_selection() {
            Ok(()) => true,
            Err(error) => {
                self.fail(error.to_string());
                false
            }
        }
    }

    /// Push the selection for callers that own the failure path.
    pub(in crate::editor) fn push_selection(&mut self) -> engine::Result<()> {
        self.editor
            .session
            .set_selection(self.editor.selection.clone())
    }

    /// Push the selection and apply one typed command without reporting or
    /// resyncing.
    pub(in crate::editor) fn run_command(
        &mut self,
        command: Command,
        ripple: bool,
    ) -> engine::Result<engine::Outcome> {
        self.push_selection()?;
        self.editor.session.apply_with_ripple(command, ripple)
    }

    /// Run one registered action through the editor's shared activation path.
    pub(in crate::editor) fn run_action(&mut self, id: &str) -> crate::Result<()> {
        self.editor.activate(id)
    }

    /// The transaction entry for callers that propagate the failure.
    pub(in crate::editor) fn transaction_result<T>(
        &mut self,
        run: impl FnOnce(&mut engine::Session) -> engine::Result<T>,
    ) -> crate::Result<T> {
        if !self.select() {
            return Err(crate::Error::Invalid("selection"));
        }
        let result = self.editor.session.transaction(run);
        match result {
            Ok(value) => {
                self.sync();
                Ok(value)
            }
            Err(error) => {
                self.fail(error.to_string());
                Err(error.into())
            }
        }
    }

    /// Stage a preview and always show it; a reversal must restore the entry
    /// frame even when the engine reports no net change.
    pub(in crate::editor) fn preview(&mut self, command: Command, ripple: bool) -> bool {
        match self.run_command(command, ripple) {
            Ok(_) => {
                self.sync();
                true
            }
            Err(error) => {
                self.fail(error.to_string());
                false
            }
        }
    }

    /// The typed-command entry for callers that propagate the failure.
    pub(in crate::editor) fn apply_result(
        &mut self,
        command: Command,
        ripple: bool,
    ) -> crate::Result<engine::Outcome> {
        if !self.select() {
            return Err(crate::Error::Invalid("selection"));
        }
        let before = self.editor.session.project().clone();
        match self.editor.session.apply_with_ripple(command, ripple) {
            Ok(outcome) => {
                if outcome.changed || !Arc::ptr_eq(&before, self.editor.session.project()) {
                    self.sync();
                }
                Ok(outcome)
            }
            Err(error) => {
                self.fail(error.to_string());
                Err(error.into())
            }
        }
    }

    /// Run one registered action, reporting a failure.
    pub(in crate::editor) fn act(&mut self, id: &str) -> bool {
        match self.run_action(id) {
            Ok(()) => true,
            Err(error) => {
                self.fail(error.to_string());
                false
            }
        }
    }

    /// Record one preference edit in the app's own history, reporting a failure;
    /// preference edits never touch the preview or the project document.
    pub(in crate::editor) fn set_preference(
        &mut self,
        key: &str,
        value: serde_json::Value,
    ) -> bool {
        match self.editor.set_preference(key, value) {
            Ok(()) => true,
            Err(error) => {
                self.fail(error.to_string());
                false
            }
        }
    }

    /// Queue a cross-area request; the frame drains it after this area returns.
    pub(in crate::editor) fn request(&mut self, request: Request) {
        self.editor.requests.push(request);
    }

    /// The editor, for reads and state the edit entry does not own.
    pub(in crate::editor) fn editor(&mut self) -> &mut Editor {
        self.editor
    }
}

impl Editor {
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

//! Updates asset feedback deadlines.
use crate::editor::assets::{Duration, Editor, Toast};

impl Editor {
    pub(in crate::editor) fn set_toast(&mut self, text: &str, detail: Option<String>) {
        self.toast = Some(Toast {
            text: text.to_owned(),
            detail,
            until: self.clock.now() + Duration::from_secs(5),
        });
    }
    pub(in crate::editor) fn truncate_toast(&mut self) {
        if self
            .toast
            .as_ref()
            .is_some_and(|toast| self.clock.now() >= toast.until)
        {
            self.toast = None;
        }
    }
}

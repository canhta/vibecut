//! Applies font picker actions: preferences, catalog commands and selection.
use super::super::State;
use super::super::font_picker::{self, Action, Download};
use super::commands::set_shared_field;
use crate::editor::{Desk, Error, Result};
use crate::preferences;
use engine::control::FontLoadState;
use engine::jobs::JobState;

impl State {
    pub(super) fn font_action(&mut self, desk: &mut Desk<'_>, action: Action) -> Result<()> {
        match action {
            Action::Close => {
                self.font_picker.close();
                Ok(())
            }
            Action::List => self.font_list(desk),
            Action::Tab(tab) => self.font_tab(desk, tab),
            Action::Favorite { family, favorite } => self.font_favorite(desk, &family, favorite),
            Action::Select(family) => self.font_select(desk, &family),
            Action::Download(family) => self.font_download(desk, &family),
            Action::Cancel => self.font_cancel(desk),
        }
    }

    fn ensure_font_backend(&mut self) -> Result<()> {
        if self.font_backend.is_none() {
            let backend: Box<dyn font_picker::Backend> = Box::new(
                font_picker::EngineBackend::native()
                    .map_err(|_| Error::Invalid("font catalog unavailable"))?,
            );
            self.font_backend = Some(backend);
        }
        Ok(())
    }

    fn font_list(&mut self, desk: &mut Desk<'_>) -> Result<()> {
        self.font_picker.status = font_picker::Status::Loading;
        self.font_picker.request_sent = true;
        if self.ensure_font_backend().is_err() {
            self.font_picker.status = font_picker::Status::Failed;
            self.font_picker.request_sent = false;
            return Ok(());
        }
        let Some(backend) = self.font_backend.as_mut() else {
            return Err(Error::Invalid("font catalog unavailable"));
        };
        let entries = desk.with_session(|session| backend.list(session, None));
        match entries {
            Ok(entries) => {
                self.font_picker.entries = entries;
                self.font_picker.loaded = true;
                self.font_picker.status = font_picker::Status::Ready;
            }
            Err(_) => {
                self.font_picker.status = font_picker::Status::Failed;
                self.font_picker.request_sent = false;
            }
        }
        Ok(())
    }

    fn font_tab(&mut self, desk: &mut Desk<'_>, tab: preferences::FontTab) -> Result<()> {
        let mut preferences = desk.view().preferences.font_picker.clone();
        preferences.tab = tab;
        self.font_picker.category = font_picker::Category::All;
        desk.set_preference_result("font_picker", serde_json::to_value(preferences)?)
    }

    fn font_favorite(&mut self, desk: &mut Desk<'_>, family: &str, favorite: bool) -> Result<()> {
        let mut preferences = desk.view().preferences.font_picker.clone();
        preferences.favorites.retain(|name| name != family);
        if favorite {
            preferences.favorites.push(family.to_owned());
            preferences.favorites.sort();
        }
        desk.set_preference_result("font_picker", serde_json::to_value(preferences)?)
    }

    fn font_select(&mut self, desk: &mut Desk<'_>, family: &str) -> Result<()> {
        let unchanged = self.font_picker.current.as_deref() == Some(family);
        if self.ensure_font_backend().is_err() {
            self.font_picker.close();
            return Ok(());
        }
        let loaded = self.font_backend.as_mut().is_some_and(|backend| {
            desk.with_session(|session| backend.load(session, family))
                .is_ok_and(|font| font.state == FontLoadState::Loaded)
        });
        let chosen = if loaded {
            Some(family.to_owned())
        } else {
            // A failed preview falls back silently to the next available family.
            self.font_picker
                .next_available(&desk.view().preferences.font_picker, family)
        };
        self.font_picker.current = chosen.clone();
        self.font_picker.close();
        if let Some(chosen) = chosen
            && !unchanged
        {
            set_shared_field(desk, "text_font", &chosen)?;
        }
        Ok(())
    }

    fn font_download(&mut self, desk: &mut Desk<'_>, family: &str) -> Result<()> {
        self.font_picker.download = Some(Download {
            family: family.to_owned(),
            job: None,
            progress: 0.0,
            cancelling: false,
            failed: false,
        });
        if self.ensure_font_backend().is_err() {
            if let Some(download) = self.font_picker.download.as_mut() {
                download.failed = true;
            }
            return Ok(());
        }
        let job = self.font_backend.as_mut().and_then(|backend| {
            desk.with_session(|session| backend.download(session, family))
                .ok()
        });
        match job {
            Some(job) => {
                if let Some(download) = self.font_picker.download.as_mut() {
                    download.job = Some(job);
                }
            }
            None => {
                if let Some(download) = self.font_picker.download.as_mut() {
                    download.failed = true;
                }
            }
        }
        Ok(())
    }

    fn font_cancel(&mut self, desk: &mut Desk<'_>) -> Result<()> {
        let Some(job) = self
            .font_picker
            .download
            .as_ref()
            .and_then(|download| download.job)
        else {
            return Ok(());
        };
        if let Some(backend) = self.font_backend.as_mut() {
            desk.with_session(|session| backend.cancel(session, job))?;
        }
        if let Some(download) = self.font_picker.download.as_mut() {
            download.cancelling = true;
        }
        Ok(())
    }

    /// Poll the active download from the job runner; no control receipt grows.
    pub(in crate::editor) fn poll_fonts(&mut self) {
        let Some(download) = self.font_picker.download.clone() else {
            return;
        };
        let Some(job) = download.job else {
            return;
        };
        let Some(backend) = self.font_backend.as_mut() else {
            return;
        };
        let Some(view) = backend.job(job) else {
            return;
        };
        let state = &mut self.font_picker;
        let Some(active) = state.download.as_mut() else {
            return;
        };
        if active.job != Some(job) {
            return;
        }
        active.progress = view.progress.clamp(0.0, 1.0);
        match view.state {
            JobState::Completed => {
                state.download = None;
                state.loaded = false;
                state.request_sent = false;
                state.status = font_picker::Status::Loading;
            }
            JobState::Failed => {
                active.job = None;
                active.failed = true;
            }
            JobState::Cancelled => state.download = None,
            JobState::Cancelling => active.cancelling = true,
            _ => {}
        }
    }
}

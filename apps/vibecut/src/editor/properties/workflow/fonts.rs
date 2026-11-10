//! Applies font picker actions: preferences, catalog commands and selection.
use super::super::font_picker::{Action, Download};
use crate::editor::Editor;
use crate::preferences;
use crate::{Error, Result};
use engine::control::FontLoadState;
use engine::jobs::JobState;

impl Editor {
    pub(super) fn font_action(&mut self, action: Action) -> Result<()> {
        match action {
            Action::Close => {
                self.properties.font_picker.close();
                Ok(())
            }
            Action::List => self.font_list(),
            Action::Tab(tab) => self.font_tab(tab),
            Action::Favorite { family, favorite } => self.font_favorite(&family, favorite),
            Action::Select(family) => self.font_select(&family),
            Action::Download(family) => self.font_download(&family),
            Action::Cancel => self.font_cancel(),
        }
    }

    fn ensure_font_backend(&mut self) -> Result<()> {
        if self.font_backend.is_none() {
            let backend: Box<dyn super::super::font_picker::Backend> = Box::new(
                super::super::font_picker::EngineBackend::native()
                    .map_err(|_| Error::Invalid("font catalog unavailable"))?,
            );
            self.font_backend = Some(backend);
        }
        Ok(())
    }

    fn font_list(&mut self) -> Result<()> {
        self.properties.font_picker.status = super::super::font_picker::Status::Loading;
        self.properties.font_picker.request_sent = true;
        if self.ensure_font_backend().is_err() {
            self.properties.font_picker.status = super::super::font_picker::Status::Failed;
            self.properties.font_picker.request_sent = false;
            return Ok(());
        }
        let Some(backend) = self.font_backend.as_mut() else {
            return Err(Error::Invalid("font catalog unavailable"));
        };
        match backend.list(&mut self.session, None) {
            Ok(entries) => {
                self.properties.font_picker.entries = entries;
                self.properties.font_picker.loaded = true;
                self.properties.font_picker.status = super::super::font_picker::Status::Ready;
            }
            Err(_) => {
                self.properties.font_picker.status = super::super::font_picker::Status::Failed;
                self.properties.font_picker.request_sent = false;
            }
        }
        Ok(())
    }

    fn font_tab(&mut self, tab: preferences::FontTab) -> Result<()> {
        let mut preferences = self.preferences.font_picker.clone();
        preferences.tab = tab;
        self.properties.font_picker.category = super::super::font_picker::Category::All;
        self.set_preference("font_picker", serde_json::to_value(preferences)?)
    }

    fn font_favorite(&mut self, family: &str, favorite: bool) -> Result<()> {
        let mut preferences = self.preferences.font_picker.clone();
        preferences.favorites.retain(|name| name != family);
        if favorite {
            preferences.favorites.push(family.to_owned());
            preferences.favorites.sort();
        }
        self.set_preference("font_picker", serde_json::to_value(preferences)?)
    }

    fn font_select(&mut self, family: &str) -> Result<()> {
        let unchanged = self.properties.font_picker.current.as_deref() == Some(family);
        if self.ensure_font_backend().is_err() {
            self.properties.font_picker.close();
            return Ok(());
        }
        let loaded = self
            .font_backend
            .as_mut()
            .and_then(|backend| backend.load(&mut self.session, family).ok())
            .is_some_and(|font| font.state == FontLoadState::Loaded);
        let chosen = if loaded {
            Some(family.to_owned())
        } else {
            // A failed preview falls back silently to the next available family.
            self.properties
                .font_picker
                .next_available(&self.preferences.font_picker, family)
        };
        self.properties.font_picker.current = chosen.clone();
        self.properties.font_picker.close();
        if let Some(chosen) = chosen
            && !unchanged
        {
            self.set_shared_field("text_font", &chosen)?;
        }
        Ok(())
    }

    fn font_download(&mut self, family: &str) -> Result<()> {
        self.properties.font_picker.download = Some(Download {
            family: family.to_owned(),
            job: None,
            progress: 0.0,
            cancelling: false,
            failed: false,
        });
        if self.ensure_font_backend().is_err() {
            if let Some(download) = self.properties.font_picker.download.as_mut() {
                download.failed = true;
            }
            return Ok(());
        }
        let job = self
            .font_backend
            .as_mut()
            .and_then(|backend| backend.download(&mut self.session, family).ok());
        match job {
            Some(job) => {
                if let Some(download) = self.properties.font_picker.download.as_mut() {
                    download.job = Some(job);
                }
            }
            None => {
                if let Some(download) = self.properties.font_picker.download.as_mut() {
                    download.failed = true;
                }
            }
        }
        Ok(())
    }

    fn font_cancel(&mut self) -> Result<()> {
        let Some(job) = self
            .properties
            .font_picker
            .download
            .as_ref()
            .and_then(|download| download.job)
        else {
            return Ok(());
        };
        if let Some(backend) = self.font_backend.as_mut() {
            backend.cancel(&mut self.session, job)?;
        }
        if let Some(download) = self.properties.font_picker.download.as_mut() {
            download.cancelling = true;
        }
        Ok(())
    }

    /// Poll the active download from the job runner; no control receipt grows.
    pub(in crate::editor) fn poll_fonts(&mut self) {
        let Some(download) = self.properties.font_picker.download.clone() else {
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
        let state = &mut self.properties.font_picker;
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
                state.status = super::super::font_picker::Status::Loading;
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

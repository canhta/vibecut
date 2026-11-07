//! Advances the editor event loop.
use crate::editor::{Editor, Text, catalog};

impl Editor {
    /// Advance jobs and the device clock even when the OS suppresses painting.
    pub fn tick(&mut self) {
        if let Some(autosave) = self.session.autosave()
            && let Err(e) = autosave.tick()
        {
            self.persistence_error = Some(std::sync::Arc::new(e));
        }
        if let Err(e) = self.playback.poll() {
            self.error = Some(e.to_string());
        }
        if let Some(previews) = &mut self.effect_previews
            && let Err(e) = previews.tick()
        {
            self.error = Some(e.to_string());
        }
        self.tick_sounds();
        if let Some(dialog) = &mut self.export
            && let Err(error) = dialog.tick(self.session.project(), self.preferences.export)
        {
            dialog.failed(error);
        }
        if let Some(subtitles) = &mut self.subtitles {
            subtitles.invalidate_options(&self.preferences.subtitles);
            let result = subtitles.tick(&mut self.session);
            let notice = subtitles.notice.take();
            match result {
                Ok(true) => self.sync_preview(),
                Ok(false) => {}
                Err(e) => self.error = Some(e.to_string()),
            }
            if let Some(count) = notice {
                let message = catalog::format(
                    self.preferences.locale,
                    Text::SubDone,
                    &[(
                        "count",
                        &catalog::number(self.preferences.locale, count as u64),
                    )],
                );
                self.set_toast(&message, None);
            }
        }
        self.apply_import_events();
        self.restore_asset_state();
        self.poll_fonts();
        self.truncate_toast();
        if self
            .media_reveal
            .as_ref()
            .is_some_and(|reveal| self.clock.now() >= reveal.until)
        {
            self.media_reveal = None;
        }
        if let Err(error) = self.remember_view() {
            self.error = Some(error.to_string());
        }
    }
}

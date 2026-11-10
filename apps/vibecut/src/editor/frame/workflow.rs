//! Advances the editor event loop.
use crate::editor::{Editor, Text};
use crate::i18n;

impl Editor {
    /// Advance jobs and the device clock even when the OS suppresses painting.
    pub fn tick(&mut self) {
        if let Some(autosave) = self.session.autosave()
            && let Err(e) = autosave.tick()
        {
            self.persistence_error = Some(std::sync::Arc::new(e));
        }
        if let Err(e) = self.playback.poll() {
            self.desk().fail(e.to_string());
        }
        if let Some(previews) = &mut self.effect_previews
            && let Err(e) = previews.tick()
        {
            self.desk().fail(e.to_string());
        }
        self.tick_sounds();
        self.dialogs
            .export
            .tick(self.session.project(), self.preferences.export);
        if let Some(subtitles) = &mut self.subtitles {
            subtitles.invalidate_options(&self.preferences.subtitles);
            let result = subtitles.tick(&mut self.session);
            let notice = subtitles.notice.take();
            match result {
                Ok(true) => self.desk().sync(),
                Ok(false) => {}
                Err(e) => self.desk().fail(e.to_string()),
            }
            if let Some(count) = notice {
                let message = i18n::format(
                    self.preferences.locale,
                    Text::SubDone,
                    &[(
                        "count",
                        &i18n::number(self.preferences.locale, count as u64),
                    )],
                );
                self.set_toast(&message, None);
            }
        }
        self.apply_import_events();
        self.restore_asset_state();
        self.poll_fonts();
        if let Err(error) = self.media.tick(&mut self.session) {
            self.desk().fail(error.to_string());
        }
        self.truncate_toast();
        if self
            .media_reveal
            .as_ref()
            .is_some_and(|reveal| self.clock.now() >= reveal.until)
        {
            self.media_reveal = None;
        }
        if let Err(error) = self.remember_view() {
            self.desk().fail(error.to_string());
        }
    }
}

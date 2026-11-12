//! Advances the editor event loop.
use crate::editor::{Editor, Text};
use crate::i18n;

impl Editor {
    /// Advance jobs and the device clock even when the OS suppresses painting.
    pub fn tick(&mut self) {
        self.drain_requests();
        if let Some(autosave) = self.session.autosave()
            && let Err(e) = autosave.tick()
        {
            self.persistence_error = Some(std::sync::Arc::new(e));
        }
        if let Err(e) = self.playback.poll(std::time::Instant::now()) {
            self.desk().fail(e.to_string());
        }
        if let Some(previews) = &mut self.effect_previews {
            previews.update(
                self.preferences.assets_tab == crate::editor::AssetsTab::Effects,
                self.clock.now(),
            );
            if let Err(e) = previews.tick() {
                self.desk().fail(e.to_string());
            }
        }
        self.tick_sounds();
        self.tick_music();
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
        let mut properties = std::mem::take(&mut self.properties);
        if let Err(error) = properties.poll_lut(&mut self.desk()) {
            properties.abort_lut();
            self.desk().fail(error.to_string());
        }
        properties.set_lut_failure(self.playback.error());
        self.properties = properties;
        if let Err(error) = self.media.tick(&mut self.session) {
            self.desk().fail(error.to_string());
        }
        self.truncate_toast();
        if self
            .assets
            .media_reveal
            .as_ref()
            .is_some_and(|reveal| self.clock.now() >= reveal.until)
        {
            self.assets.media_reveal = None;
        }
        if let Err(error) = self.remember_view() {
            self.desk().fail(error.to_string());
        }
    }

    /// Wait for every requested effect preview to reach a terminal tile state.
    /// The audit calls this before capture so job timing cannot choose pixels.
    pub fn settle_previews(&mut self, timeout: std::time::Duration) -> crate::Result<()> {
        if let Some(previews) = &mut self.effect_previews {
            previews.settle(timeout)?;
        }
        Ok(())
    }
}

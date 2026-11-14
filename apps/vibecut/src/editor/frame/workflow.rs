//! Advances the editor event loop.
use crate::editor::{Editor, Text};
use crate::i18n;

impl Editor {
    /// Advance jobs and the device clock even when the OS suppresses painting.
    pub fn tick(&mut self) {
        self.drain_requests();
        self.agent.tick();
        if let Some(autosave) = self.session.autosave() {
            let was_pending = autosave.pending();
            if let Err(e) = autosave.tick() {
                self.persistence_error = Some(std::sync::Arc::new(e));
            }
            // The header reads the save state at paint time; a write that finishes (or
            // fails) here with the loop idle needs one more paint to show the result.
            if was_pending && !autosave.pending() {
                wake::notify();
            }
        }
        self.end_scrub_burst();
        if let Err(e) = self.playback.poll(std::time::Instant::now()) {
            self.fail(e.to_string());
        }
        if let Some(previews) = &mut self.assets.effect_previews {
            previews.update(
                self.preferences.assets_tab == crate::editor::AssetsTab::Effects,
                self.clock.now(),
            );
            if let Err(e) = previews.tick() {
                self.fail(e.to_string());
            }
        }
        self.with_assets(|assets, port| {
            assets.tick_sounds(port);
            assets.tick_music(port);
        });
        let finished = self
            .dialogs
            .export
            .tick(self.session.project(), self.preferences.export);
        if finished && self.preferences.export_sound {
            crate::editor::dialogs::export::play_chime();
        }
        if let Some(subtitles) = &mut self.assets.captions {
            subtitles.invalidate_options(&self.preferences.subtitles);
            let result = subtitles.tick(&mut self.session);
            let notice = subtitles.notice.take();
            match result {
                Ok(true) => self.sync_preview(),
                Ok(false) => {}
                Err(e) => self.fail(e.to_string()),
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
        self.with_assets(|assets, port| {
            assets.apply_import_events(port);
            assets.restore_asset_state(port);
        });
        self.poll_fonts();
        self.poll_clip_download();
        self.with_properties(|properties, port| {
            if let Err(error) = properties.poll_lut(port) {
                properties.abort_lut();
                port.fail(error.to_string());
            }
            if let Err(error) = properties.poll_footage(port) {
                port.fail(error.to_string());
            }
            properties.poll_loudness(port);
            properties.poll_narration(port);
        });
        let failure = self.playback.error();
        self.properties.set_lut_failure(failure);
        if let Err(error) = self.timeline.tick_media(&mut self.session) {
            self.fail(error.to_string());
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
            self.fail(error.to_string());
        }
        // Clock-driven state has no worker to wake the loop: the autosave debounce and the
        // toast and reveal expiry. Ask for one wake at the nearest deadline; a state with
        // none leaves the loop asleep, so an idle editor requests nothing.
        let now = self.clock.now();
        let due = [
            self.session
                .autosave()
                .and_then(|autosave| autosave.next_tick_in()),
            self.toast
                .as_ref()
                .map(|toast| toast.until.saturating_sub(now)),
            self.assets
                .media_reveal
                .as_ref()
                .map(|reveal| reveal.until.saturating_sub(now)),
        ]
        .into_iter()
        .flatten()
        .min();
        if let Some(due) = due {
            wake::notify_after(due.max(std::time::Duration::from_millis(1)));
        }
    }

    /// Wait for every requested effect preview to reach a terminal tile state.
    /// The audit calls this before capture so job timing cannot choose pixels.
    pub fn settle_previews(&mut self, timeout: std::time::Duration) -> crate::Result<()> {
        if let Some(previews) = &mut self.assets.effect_previews {
            previews.settle(timeout)?;
        }
        if let Some(previews) = &mut self.assets.filter_previews {
            previews.settle(timeout)?;
        }
        Ok(())
    }
}

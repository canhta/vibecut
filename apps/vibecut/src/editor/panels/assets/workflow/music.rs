//! Applies the Music section's provider, cache, key and add workflows.
use super::super::sounds::music::AddIntent;
use crate::editor::{Editor, Result};
use crate::services::music::MusicSession;
use engine::CreatedId;
use std::path::PathBuf;
use time::Tick;

impl Editor {
    /// Attach the process-shared music session. One library and credential are
    /// shared by every project window; each editor owns only its presentation.
    pub fn set_music_session(&mut self, session: std::sync::Arc<MusicSession>) -> Result<()> {
        let music =
            super::super::sounds::music::Music::new(session, self.playback.budget().clone())?;
        self.music = Some(music);
        self.assets.music_open = None;
        Ok(())
    }

    /// Injection boundary for tests and audit scenes.
    pub fn set_music(&mut self, music: super::super::sounds::music::Music) {
        self.music = Some(music);
    }

    /// Project-owned media directory used when a track is added.
    pub fn set_music_media_dir(&mut self, directory: PathBuf) {
        self.music_media_dir = Some(directory);
    }

    /// Load the first browse page once, when the Music section is first opened.
    pub(crate) fn music_loaded(&mut self) {
        if let Some(music) = &mut self.music {
            music.ensure_loaded();
        }
    }

    pub(crate) fn tick_music(&mut self) {
        let mut failure = None;
        let ready = if let Some(music) = &mut self.music {
            if let Err(error) = music.tick() {
                failure = Some(error.to_string());
            }
            music.take_ready_add()
        } else {
            None
        };
        if let Some(message) = failure {
            self.desk().fail(message);
        }
        let Some(intent) = ready else {
            return;
        };
        if let Err(error) = self.add_music_to_timeline(&intent) {
            if let Some(music) = &mut self.music {
                music.add_failed(
                    intent,
                    super::super::sounds::music::MusicProblem::from_app(&error),
                );
            }
            self.desk().fail(error.to_string());
        }
    }

    pub(crate) fn music_set_mode(&mut self, mode: super::super::sounds::music::MusicMode) {
        self.assets.music_mode = mode;
        self.music_search(false);
    }

    pub(crate) fn music_set_chip(&mut self, chip: &'static str) {
        self.assets.music_chip = chip;
        self.assets.music_mode = super::super::sounds::music::MusicMode::Browse;
        self.assets.music_search.clear();
        self.music_search(false);
    }

    pub(crate) fn music_search(&mut self, more: bool) {
        let mode = self.assets.music_mode;
        let chip = self.assets.music_chip;
        let query = self.assets.music_search.clone();
        if let Some(music) = &mut self.music {
            if more {
                music.search_more();
            } else {
                music.search(&query, mode, chip, false);
            }
        }
    }

    pub(crate) fn music_retry(&mut self) {
        if let Some(music) = &mut self.music {
            music.clear_problems();
        }
        self.music_search(false);
    }

    /// Resubmit the retained failed add; it never reloads search.
    pub(crate) fn music_retry_add(&mut self) {
        if let Some(music) = &mut self.music {
            music.retry_add();
        }
    }

    pub(crate) fn music_preview(&mut self, id: &str) {
        if let Some(music) = &mut self.music {
            music.play(id);
        }
    }

    pub(crate) fn music_toggle_saved(&mut self, id: &str) {
        if let Some(music) = &mut self.music {
            music.toggle_saved(id);
        }
    }

    pub(crate) fn music_cancel_cache(&mut self) {
        if let Some(music) = &mut self.music {
            music.cancel_cache();
        }
    }

    /// Apply the masked draft to this process only. A refused key keeps the
    /// previous credential and surfaces its typed failure; the draft is cleared
    /// only on success.
    pub(crate) fn music_key_apply(&mut self) {
        let draft = self.assets.music_key.clone();
        let key = (!draft.is_empty()).then_some(draft.as_str());
        let Some(music) = &mut self.music else {
            return;
        };
        match music.apply_key(key) {
            Ok(()) => {
                self.assets.music_key.clear();
                let saved =
                    crate::i18n::text(self.preferences.locale, crate::i18n::Text::MusicKeySaved)
                        .to_owned();
                self.set_toast(&saved, None);
            }
            Err(_) => {
                let refused =
                    crate::i18n::text(self.preferences.locale, crate::i18n::Text::MusicAuth)
                        .to_owned();
                self.desk().fail(refused);
            }
        }
    }

    /// Remove the session key and return to anonymous operation.
    pub(crate) fn music_key_remove(&mut self) {
        if let Some(music) = &mut self.music {
            let _ = music.apply_key(None);
        }
        self.assets.music_key.clear();
        let removed =
            crate::i18n::text(self.preferences.locale, crate::i18n::Text::MusicKeyRemoved)
                .to_owned();
        self.set_toast(&removed, None);
    }

    pub(crate) fn music_add(&mut self, id: &str) {
        let Some(track) = self.music.as_ref().and_then(|state| state.music(id)) else {
            return;
        };
        // The target is fixed at click time, so a later playhead move or a retry
        // never retargets this add.
        let intent = AddIntent {
            music: track,
            start: self.playback.position().unwrap_or(Tick::ZERO),
        };
        if self
            .music
            .as_ref()
            .is_some_and(|state| state.cached(&intent.music).is_some())
        {
            if let Err(error) = self.add_music_to_timeline(&intent) {
                if let Some(state) = &mut self.music {
                    state.add_failed(
                        intent,
                        super::super::sounds::music::MusicProblem::from_app(&error),
                    );
                }
                self.desk().fail(error.to_string());
            }
        } else if let Some(state) = &mut self.music {
            state.request_add(intent);
        }
    }

    fn add_music_to_timeline(&mut self, intent: &AddIntent) -> Result<()> {
        let cached = self
            .music
            .as_ref()
            .and_then(|state| state.cached(&intent.music))
            .ok_or(crate::Error::Invalid("music cache"))?;
        let media = self.music_media_path();
        let outcome = libraries::music::add_to_timeline(
            &mut self.session,
            &intent.music,
            &cached,
            &media,
            engine::Placement::Auto,
            intent.start,
        )?;
        if let Some(CreatedId::Item(item)) = outcome
            .created
            .iter()
            .find(|id| matches!(id, CreatedId::Item(_)))
            .copied()
        {
            self.desk().select(std::collections::BTreeSet::from([item]));
            self.selection_scopes = Default::default();
        }
        self.desk().sync();
        let added =
            crate::i18n::text(self.preferences.locale, crate::i18n::Text::MusicAdded).to_owned();
        self.set_toast(&added, None);
        Ok(())
    }

    fn music_media_path(&self) -> PathBuf {
        if let Some(directory) = &self.music_media_dir {
            return directory.clone();
        }
        if let crate::services::lifecycle::EditorSession::Persistent(autosave) = &self.session
            && let Some(path) = autosave.saved_path()
        {
            let mut media = path.as_os_str().to_os_string();
            media.push(".media");
            return PathBuf::from(media);
        }
        std::env::temp_dir().join("vibecut-music-media")
    }
}

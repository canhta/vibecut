//! Applies the Music section's provider, cache, key and add workflows.
use super::super::sounds::music::{AddIntent, Music, MusicMode, MusicProblem};
use super::State;
use crate::editor::{Port, Result};
use crate::services::assets::music::MusicSession;
use engine::CreatedId;
use std::path::PathBuf;
use time::Tick;

impl State {
    /// Attach the process-shared music session. One library and credential are
    /// shared by every project window; each editor owns only its presentation.
    pub(in crate::editor) fn set_music_session(
        &mut self,
        port: &mut Port<'_>,
        session: std::sync::Arc<MusicSession>,
    ) -> Result<()> {
        self.music = Some(Music::new(session, port.read.budget.clone())?);
        self.music_open = None;
        Ok(())
    }

    /// Injection boundary for tests and audit scenes.
    pub(in crate::editor) fn set_music(&mut self, music: Music) {
        self.music = Some(music);
    }

    /// Project-owned media directory used when a track is added.
    pub(in crate::editor) fn set_music_media_dir(&mut self, directory: PathBuf) {
        self.music_media_dir = Some(directory);
    }

    /// Load the first browse page once, when the Music section is first opened.
    pub(in crate::editor) fn music_loaded(&mut self) {
        if let Some(music) = &mut self.music {
            music.ensure_loaded();
        }
    }

    pub(in crate::editor) fn tick_music(&mut self, port: &mut Port<'_>) {
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
            port.fail(message);
        }
        let Some(intent) = ready else {
            return;
        };
        if let Err(error) = self.add_music_to_timeline(port, &intent) {
            if let Some(music) = &mut self.music {
                music.add_failed(intent, MusicProblem::from_app(&error));
            }
            port.fail(error.to_string());
        }
    }

    pub(in crate::editor) fn music_set_mode(&mut self, mode: MusicMode) {
        self.music_mode = mode;
        self.music_search(false);
    }

    pub(in crate::editor) fn music_set_chip(&mut self, chip: &'static str) {
        self.music_chip = chip;
        self.music_mode = MusicMode::Browse;
        self.music_search.clear();
        self.music_search(false);
    }

    pub(in crate::editor) fn music_search(&mut self, more: bool) {
        let mode = self.music_mode;
        let chip = self.music_chip;
        let query = self.music_search.clone();
        if let Some(music) = &mut self.music {
            if more {
                music.search_more();
            } else {
                music.search(&query, mode, chip, false);
            }
        }
    }

    pub(in crate::editor) fn music_retry(&mut self) {
        if let Some(music) = &mut self.music {
            music.clear_problems();
        }
        self.music_search(false);
    }

    /// Resubmit the retained failed add; it never reloads search.
    pub(in crate::editor) fn music_retry_add(&mut self) {
        if let Some(music) = &mut self.music {
            music.retry_add();
        }
    }

    pub(in crate::editor) fn music_preview(&mut self, id: &str) {
        if let Some(music) = &mut self.music {
            music.play(id);
        }
    }

    pub(in crate::editor) fn music_toggle_saved(&mut self, id: &str) {
        if let Some(music) = &mut self.music {
            music.toggle_saved(id);
        }
    }

    pub(in crate::editor) fn music_cancel_cache(&mut self) {
        if let Some(music) = &mut self.music {
            music.cancel_cache();
        }
    }

    /// Apply the masked draft to this process only. A refused key keeps the
    /// previous credential and surfaces its typed failure; the draft is cleared
    /// only on success.
    pub(in crate::editor) fn music_key_apply(&mut self, port: &mut Port<'_>) {
        let draft = self.music_key.clone();
        let key = (!draft.is_empty()).then_some(draft.as_str());
        let Some(music) = &mut self.music else {
            return;
        };
        match music.apply_key(key) {
            Ok(()) => {
                self.music_key.clear();
                let saved = port.tr(crate::i18n::Text::MusicKeySaved);
                port.toast(saved, None);
            }
            Err(_) => {
                let refused = port.tr(crate::i18n::Text::MusicAuth);
                port.fail(refused);
            }
        }
    }

    /// Remove the session key and return to anonymous operation.
    pub(in crate::editor) fn music_key_remove(&mut self, port: &mut Port<'_>) {
        if let Some(music) = &mut self.music {
            let _ = music.apply_key(None);
        }
        self.music_key.clear();
        let removed = port.tr(crate::i18n::Text::MusicKeyRemoved);
        port.toast(removed, None);
    }

    pub(in crate::editor) fn music_add(&mut self, port: &mut Port<'_>, id: &str) {
        let Some(track) = self.music.as_ref().and_then(|state| state.music(id)) else {
            return;
        };
        // The target is fixed at click time, so a later playhead move or a retry
        // never retargets this add.
        let intent = AddIntent {
            music: track,
            start: port.read.playhead.unwrap_or(Tick::ZERO),
        };
        if self
            .music
            .as_ref()
            .is_some_and(|state| state.cached(&intent.music).is_some())
        {
            if let Err(error) = self.add_music_to_timeline(port, &intent) {
                if let Some(state) = &mut self.music {
                    state.add_failed(intent, MusicProblem::from_app(&error));
                }
                port.fail(error.to_string());
            }
        } else if let Some(state) = &mut self.music {
            state.request_add(intent);
        }
    }

    fn add_music_to_timeline(&mut self, port: &mut Port<'_>, intent: &AddIntent) -> Result<()> {
        let cached = self
            .music
            .as_ref()
            .and_then(|state| state.cached(&intent.music))
            .ok_or(crate::Error::Invalid("music cache"))?;
        let media = self.music_media_path(port);
        let outcome = libraries::music::add_to_timeline(
            &mut *port.session,
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
            port.select(std::collections::BTreeSet::from([item]));
            port.reset_selection(false);
        }
        port.sync();
        let added = port.tr(crate::i18n::Text::MusicAdded);
        port.toast(added, None);
        Ok(())
    }

    fn music_media_path(&self, port: &Port<'_>) -> PathBuf {
        if let Some(directory) = &self.music_media_dir {
            return directory.clone();
        }
        project::media_dir(port.session.saved_path())
    }
}

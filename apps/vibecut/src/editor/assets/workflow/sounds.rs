//! Applies the Sounds tab's provider, cache and add workflows.
use crate::editor::{Editor, Result};
use engine::CreatedId;
use libraries::sounds::SoundResult;
use std::path::PathBuf;
use time::Tick;

impl Editor {
    /// Attach the app-shared saved library. One library handle is shared by every
    /// project window; each editor owns only its search/preview presentation.
    pub fn set_sound_library(
        &mut self,
        library: std::sync::Arc<libraries::sounds::SoundLibrary>,
    ) -> Result<()> {
        let sounds =
            super::super::sounds::Sounds::new(library.clone(), self.playback.budget().clone())?;
        self.sound_library = Some(library);
        self.sounds = Some(sounds);
        Ok(())
    }

    /// Injection boundary for tests and audit scenes.
    pub fn set_sounds(&mut self, sounds: super::super::sounds::Sounds) {
        self.sounds = Some(sounds);
    }

    /// Project-owned media directory used when a library sound is added.
    pub fn set_sound_media_dir(&mut self, directory: PathBuf) {
        self.sound_media_dir = Some(directory);
    }

    pub(crate) fn tick_sounds(&mut self) {
        let mut failure = None;
        let ready = if let Some(sounds) = &mut self.sounds {
            if let Err(error) = sounds.tick() {
                failure = Some(error.to_string());
            }
            sounds.take_ready_add()
        } else {
            None
        };
        if let Some(message) = failure {
            self.desk().fail(message);
        }
        let Some(sound) = ready else {
            return;
        };
        if let Err(error) = self.add_sound_to_timeline(&sound) {
            if let Some(sounds) = &mut self.sounds {
                sounds.add_failed(super::super::sounds::SoundsProblem::from_app(&error));
            }
            self.desk().fail(error.to_string());
        }
    }

    pub(crate) fn sounds_set_mode(&mut self, mode: super::super::sounds::SoundsMode) {
        self.assets.sounds_mode = mode;
        self.sounds_search(false);
    }

    pub(crate) fn sounds_set_commercial(&mut self, commercial_only: bool) {
        self.assets.sounds_commercial = commercial_only;
        self.sounds_search(false);
    }

    pub(crate) fn sounds_search(&mut self, more: bool) {
        let mode = self.assets.sounds_mode;
        let commercial = self.assets.sounds_commercial;
        let query = self
            .assets
            .searches
            .get(&super::AssetsTab::Sounds)
            .cloned()
            .unwrap_or_default();
        if let Some(sounds) = &mut self.sounds {
            if more {
                sounds.search_more();
            } else {
                sounds.search(&query, commercial, mode, false);
            }
        }
    }

    pub(crate) fn sounds_retry(&mut self) {
        if let Some(sounds) = &mut self.sounds {
            sounds.clear_problems();
        }
        self.sounds_search(false);
    }

    pub(crate) fn sounds_preview(&mut self, id: u64) {
        if let Some(sounds) = &mut self.sounds {
            sounds.play(id);
        }
    }

    pub(crate) fn sounds_toggle_saved(&mut self, id: u64) {
        if let Some(sounds) = &mut self.sounds {
            sounds.toggle_saved(id);
        }
    }

    pub(crate) fn sounds_cancel_cache(&mut self) {
        if let Some(sounds) = &mut self.sounds {
            sounds.cancel_cache();
        }
    }

    pub(crate) fn sounds_add(&mut self, id: u64) {
        let Some(sound) = self.sounds.as_ref().and_then(|sounds| sounds.sound(id)) else {
            return;
        };
        if self
            .sounds
            .as_ref()
            .is_some_and(|sounds| sounds.cached(&sound).is_some())
        {
            if let Err(error) = self.add_sound_to_timeline(&sound) {
                if let Some(sounds) = &mut self.sounds {
                    sounds.add_failed(super::super::sounds::SoundsProblem::from_app(&error));
                }
                self.desk().fail(error.to_string());
            }
        } else if let Some(sounds) = &mut self.sounds {
            sounds.request_add(sound);
        }
    }

    fn add_sound_to_timeline(&mut self, sound: &SoundResult) -> Result<()> {
        let cached = self
            .sounds
            .as_ref()
            .and_then(|sounds| sounds.cached(sound))
            .ok_or(crate::Error::Invalid("sound cache"))?;
        let media = self.sound_media_path();
        let start = self.playback.position().unwrap_or(Tick::ZERO);
        let outcome = libraries::sounds::add_to_timeline(
            &mut self.session,
            sound,
            &cached,
            &media,
            engine::Placement::Auto,
            start,
        )?;
        if let Some(CreatedId::Item(item)) = outcome
            .created
            .iter()
            .find(|id| matches!(id, CreatedId::Item(_)))
            .copied()
        {
            self.selection.clear();
            self.selection.insert(item);
            self.selection_scopes = Default::default();
        }
        self.desk().sync();
        let added =
            crate::i18n::text(self.preferences.locale, crate::i18n::Text::SoundsAdded).to_owned();
        self.set_toast(&added, None);
        Ok(())
    }

    fn sound_media_path(&self) -> PathBuf {
        if let Some(directory) = &self.sound_media_dir {
            return directory.clone();
        }
        if let crate::lifecycle::EditorSession::Persistent(autosave) = &self.session
            && let Some(path) = autosave.saved_path()
        {
            let mut media = path.as_os_str().to_os_string();
            media.push(".media");
            return PathBuf::from(media);
        }
        std::env::temp_dir().join("vibecut-sounds-media")
    }
}

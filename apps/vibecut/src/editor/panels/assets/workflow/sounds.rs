//! Applies the Sounds tab's provider, cache and add workflows.
use super::super::sounds::{LibraryStore, Sounds, SoundsMode, SoundsProblem};
use super::{AssetsTab, State};
use crate::editor::{Port, Result};
use engine::CreatedId;
use libraries::sounds::SoundResult;
use std::path::PathBuf;
use std::sync::Arc;
use time::Tick;

impl State {
    /// Attach the app-shared saved library. One library handle is shared by every
    /// project window; each editor owns only its search/preview presentation.
    pub(in crate::editor) fn set_sound_library(
        &mut self,
        port: &mut Port<'_>,
        library: Arc<libraries::sounds::SoundLibrary>,
    ) -> Result<()> {
        let sounds = Sounds::new(library, port.read.budget.clone())?;
        self.sounds = Some(sounds);
        self.sounds_open = None;
        Ok(())
    }

    /// Injection boundary for tests and audit scenes.
    pub(in crate::editor) fn set_sounds(&mut self, sounds: Sounds) {
        self.sounds = Some(sounds);
    }

    /// Project-owned media directory used when a library sound is added.
    pub(in crate::editor) fn set_sound_media_dir(&mut self, directory: PathBuf) {
        self.sound_media_dir = Some(directory);
    }

    /// A section's Retry asks the lending owner to open this store again; the lending
    /// owner drains the request on the next frame.
    pub(in crate::editor) fn request_library_reopen(&mut self, store: LibraryStore) {
        if !self.library_reopens.contains(&store) {
            self.library_reopens.push(store);
        }
    }

    pub(in crate::editor) fn tick_sounds(&mut self, port: &mut Port<'_>) {
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
            port.fail(message);
        }
        let Some(sound) = ready else {
            return;
        };
        if let Err(error) = self.add_sound_to_timeline(port, &sound) {
            if let Some(sounds) = &mut self.sounds {
                sounds.add_failed(SoundsProblem::from_app(&error));
            }
            port.fail(error.to_string());
        }
    }

    pub(in crate::editor) fn sounds_set_mode(&mut self, mode: SoundsMode) {
        self.sounds_mode = mode;
        self.sounds_search(false);
    }

    pub(in crate::editor) fn sounds_set_commercial(&mut self, commercial_only: bool) {
        self.sounds_commercial = commercial_only;
        self.sounds_search(false);
    }

    pub(in crate::editor) fn sounds_search(&mut self, more: bool) {
        let mode = self.sounds_mode;
        let commercial = self.sounds_commercial;
        let query = self
            .searches
            .get(&AssetsTab::Sounds)
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

    pub(in crate::editor) fn sounds_retry(&mut self) {
        if let Some(sounds) = &mut self.sounds {
            sounds.clear_problems();
        }
        self.sounds_search(false);
    }

    pub(in crate::editor) fn sounds_preview(&mut self, id: u64) {
        if let Some(sounds) = &mut self.sounds {
            sounds.play(id);
        }
    }

    pub(in crate::editor) fn sounds_play(&mut self, sound: SoundResult) {
        if let Some(sounds) = &mut self.sounds {
            sounds.play_sound(sound);
        }
    }

    pub(in crate::editor) fn sounds_toggle_saved(&mut self, id: u64) {
        if let Some(sounds) = &mut self.sounds {
            sounds.toggle_saved(id);
        }
    }

    pub(in crate::editor) fn sounds_cancel_cache(&mut self) {
        if let Some(sounds) = &mut self.sounds {
            sounds.cancel_cache();
        }
    }

    pub(in crate::editor) fn sounds_add(&mut self, port: &mut Port<'_>, id: u64) {
        let Some(sound) = self.sounds.as_ref().and_then(|sounds| sounds.sound(id)) else {
            return;
        };
        if self
            .sounds
            .as_ref()
            .is_some_and(|sounds| sounds.cached(&sound).is_some())
        {
            if let Err(error) = self.add_sound_to_timeline(port, &sound) {
                if let Some(sounds) = &mut self.sounds {
                    sounds.add_failed(SoundsProblem::from_app(&error));
                }
                port.fail(error.to_string());
            }
        } else if let Some(sounds) = &mut self.sounds {
            sounds.request_add(sound);
        }
    }

    fn add_sound_to_timeline(&mut self, port: &mut Port<'_>, sound: &SoundResult) -> Result<()> {
        let cached = self
            .sounds
            .as_ref()
            .and_then(|sounds| sounds.cached(sound))
            .ok_or(crate::Error::Invalid("sound cache"))?;
        let media = self.sound_media_path(port);
        let start = port.read.playhead.unwrap_or(Tick::ZERO);
        let outcome = libraries::sounds::add_to_timeline(
            &mut *port.session,
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
            port.select(std::collections::BTreeSet::from([item]));
            port.reset_selection(false);
        }
        port.sync();
        let added = port.tr(crate::i18n::Text::SoundsAdded);
        port.toast(added, None);
        Ok(())
    }

    fn sound_media_path(&self, port: &Port<'_>) -> PathBuf {
        if let Some(directory) = &self.sound_media_dir {
            return directory.clone();
        }
        project::media_dir(port.session.saved_path())
    }
}

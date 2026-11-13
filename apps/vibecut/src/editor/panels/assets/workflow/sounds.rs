//! Applies the Sounds tab's provider, cache and add workflows.
use super::super::sounds::{Sounds, SoundsMode, SoundsProblem};
use super::{AssetsTab, State};
use crate::editor::{Desk, Editor, Result};
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
        desk: &mut Desk<'_>,
        library: Arc<libraries::sounds::SoundLibrary>,
    ) -> Result<()> {
        let sounds = Sounds::new(library, desk.editor().playback.budget().clone())?;
        self.sounds = Some(sounds);
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

    pub(in crate::editor) fn tick_sounds(&mut self, desk: &mut Desk<'_>) {
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
            desk.fail(message);
        }
        let Some(sound) = ready else {
            return;
        };
        if let Err(error) = self.add_sound_to_timeline(desk, &sound) {
            if let Some(sounds) = &mut self.sounds {
                sounds.add_failed(SoundsProblem::from_app(&error));
            }
            desk.fail(error.to_string());
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

    pub(in crate::editor) fn sounds_add(&mut self, desk: &mut Desk<'_>, id: u64) {
        let Some(sound) = self.sounds.as_ref().and_then(|sounds| sounds.sound(id)) else {
            return;
        };
        if self
            .sounds
            .as_ref()
            .is_some_and(|sounds| sounds.cached(&sound).is_some())
        {
            if let Err(error) = self.add_sound_to_timeline(desk, &sound) {
                if let Some(sounds) = &mut self.sounds {
                    sounds.add_failed(SoundsProblem::from_app(&error));
                }
                desk.fail(error.to_string());
            }
        } else if let Some(sounds) = &mut self.sounds {
            sounds.request_add(sound);
        }
    }

    fn add_sound_to_timeline(&mut self, desk: &mut Desk<'_>, sound: &SoundResult) -> Result<()> {
        let cached = self
            .sounds
            .as_ref()
            .and_then(|sounds| sounds.cached(sound))
            .ok_or(crate::Error::Invalid("sound cache"))?;
        let media = self.sound_media_path(desk.editor());
        let start = desk.editor().playback.position().unwrap_or(Tick::ZERO);
        let outcome = libraries::sounds::add_to_timeline(
            &mut desk.editor().session,
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
            desk.select(std::collections::BTreeSet::from([item]));
            desk.editor().selection_scopes = Default::default();
        }
        desk.sync();
        let added = crate::i18n::text(
            desk.editor().preferences.locale,
            crate::i18n::Text::SoundsAdded,
        )
        .to_owned();
        desk.toast(&added, None);
        Ok(())
    }

    fn sound_media_path(&self, editor: &Editor) -> PathBuf {
        if let Some(directory) = &self.sound_media_dir {
            return directory.clone();
        }
        editor.session.media_directory()
    }
}

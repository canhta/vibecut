//! The one lending point: the process stores and the model service every editor borrows.
use super::window::EditorWindow;
use crate::{Editor, Error, model_manager::ModelManager};
use engine::{ByteBudget, control::MediaAccess};
use libraries::{brand_kit::BrandKit, sounds::SoundLibrary};
use std::{
    path::Path,
    sync::{Arc, Mutex},
};

/// Stores opened once per process and lent to each editor; a second open of the
/// same root is refused by its writer lease, so no window opens one itself.
pub(super) struct Lending {
    sounds: Option<Arc<SoundLibrary>>,
    /// One process music library and session credential shared by every window.
    music: Option<Arc<crate::services::music::MusicSession>>,
    /// One process media cache instance, lent to every editor's timeline and to
    /// the desktop MCP inbox. A second open of the same root is never attempted.
    media: Option<Arc<Mutex<MediaAccess>>>,
    /// The user's brand kits. No editor or control consumer reads it yet.
    brand: Option<Arc<BrandKit>>,
    /// The model service follows the focused editor: it is lent for a frame and
    /// taken back, so downloads outlive any one project window.
    models: Option<ModelManager>,
}

impl Lending {
    /// Open every store beside `root`. A store that cannot open is simply not
    /// lent; its commands refuse as unavailable.
    pub(super) fn open(root: &Path, budget: &ByteBudget) -> Self {
        let sounds = SoundLibrary::open(&root.with_file_name("sounds"))
            .ok()
            .map(Arc::new);
        let music = libraries::music::MusicLibrary::open(&root.with_file_name("music"))
            .ok()
            .map(|library| Arc::new(crate::services::music::MusicSession::new(Arc::new(library))));
        // The workspace root keeps test runs isolated and never races the CLI's cache root.
        let media = MediaAccess::open_in(&root.with_file_name("media"), budget.clone())
            .ok()
            .map(|access| Arc::new(Mutex::new(access)));
        let brand = BrandKit::open(&root.with_file_name("brand_kits"))
            .ok()
            .map(Arc::new);
        Self {
            sounds,
            music,
            media,
            brand,
            models: None,
        }
    }
    pub(super) fn media(&self) -> Option<Arc<Mutex<MediaAccess>>> {
        self.media.clone()
    }
    pub(super) fn brand(&self) -> Option<Arc<BrandKit>> {
        self.brand.clone()
    }
    /// Give a newly registered editor the shared stores.
    pub(super) fn lend_stores(&self, editor: &mut Editor) {
        if let Some(sounds) = &self.sounds {
            let _ = editor.set_sound_library(sounds.clone());
        }
        if let Some(music) = &self.music {
            let _ = editor.set_music_session(music.clone());
        }
        if let Some(media) = &self.media {
            editor.set_media(media.clone());
        }
    }
    /// Lend the model service to an editor, when the lender holds it. Background
    /// workflow completions can request it before the next UI pass.
    pub(super) fn lend_models(&mut self, editor: &mut Editor) {
        if let Some(models) = self.models.take() {
            editor.set_model_manager(models);
        }
    }
    /// Take the model service back from a window, whether its editor still holds
    /// it or a save carried it off with the editor.
    pub(super) fn reclaim_models(&mut self, window: &mut EditorWindow) {
        if let Some(models) = window.retained_models.take() {
            self.models = Some(models);
        } else if let Some(editor) = &mut window.editor
            && let Some(models) = editor.take_model_manager()
        {
            self.models = Some(models);
        }
    }
    /// Advance the held model service; a failure is the app-level error.
    pub(super) fn tick_models(&mut self) -> Option<Error> {
        self.models.as_mut()?.tick().err()
    }
}

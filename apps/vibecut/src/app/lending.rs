//! The one lending point: the process stores and the model service every editor borrows.
use super::window::EditorWindow;
use crate::{
    Editor, Error, editor::LibraryStore, model_manager::ModelManager, services::music::MusicSession,
};
use engine::{ByteBudget, MusicError, SoundError, control::MediaAccess};
use libraries::{brand_kit::BrandKit, music::MusicLibrary, sounds::SoundLibrary};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

/// Stores opened once per process and lent to each editor; a second open of the
/// same root is refused by its writer lease, so no window opens one itself.
pub(super) struct Lending {
    /// The workspace root the stores open beside; a Retry reopens from it.
    root: PathBuf,
    /// The Sounds library, or the typed error its open returned.
    sounds: Result<Arc<SoundLibrary>, SoundError>,
    /// One process music library and session credential shared by every window,
    /// or the typed error its open returned.
    music: Result<Arc<MusicSession>, MusicError>,
    /// One process media cache instance, lent to every editor's timeline and to
    /// the desktop MCP inbox. A second open of the same root is never attempted.
    media: Option<Arc<Mutex<MediaAccess>>>,
    /// The user's brand kits. No editor or control consumer reads it yet.
    brand: Option<Arc<BrandKit>>,
    /// The model service follows the focused editor: it is lent for a frame and
    /// taken back, so downloads outlive any one project window.
    models: Option<ModelManager>,
}

fn open_sounds(root: &Path) -> Result<Arc<SoundLibrary>, SoundError> {
    SoundLibrary::open(&root.with_file_name("sounds")).map(Arc::new)
}

fn open_music(root: &Path) -> Result<Arc<MusicSession>, MusicError> {
    MusicLibrary::open(&root.with_file_name("music"))
        .map(|library| Arc::new(MusicSession::new(Arc::new(library))))
}

impl Lending {
    /// Open every store beside `root`. A store that cannot open keeps its typed
    /// error here and its section shows the reason; its commands refuse as unavailable.
    pub(super) fn open(root: &Path, budget: &ByteBudget) -> Self {
        // The workspace root keeps test runs isolated and never races the CLI's cache root.
        let media = MediaAccess::open_in(&root.with_file_name("media"), budget.clone())
            .ok()
            .map(|access| Arc::new(Mutex::new(access)));
        let brand = BrandKit::open(&root.with_file_name("brand_kits"))
            .ok()
            .map(Arc::new);
        Self {
            root: root.to_path_buf(),
            sounds: open_sounds(root),
            music: open_music(root),
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
    /// Give a newly registered editor the shared stores, or the reason each did not open.
    pub(super) fn lend_stores(&self, editor: &mut Editor) {
        self.lend_sounds(editor);
        self.lend_music(editor);
        if let Some(media) = &self.media {
            editor.set_media(media.clone());
        }
    }
    fn lend_sounds(&self, editor: &mut Editor) {
        match &self.sounds {
            Ok(sounds) => {
                let _ = editor.set_sound_library(sounds.clone());
            }
            Err(error) => editor.set_sound_open_error(error),
        }
    }
    fn lend_music(&self, editor: &mut Editor) {
        match &self.music {
            Ok(music) => {
                let _ = editor.set_music_session(music.clone());
            }
            Err(error) => editor.set_music_open_error(error),
        }
    }
    /// Serve the stores a section asked to open again. A store that opened before
    /// is lent as it stands; one that failed is opened again here, by its only owner.
    pub(super) fn reopen_requested(&mut self, editor: &mut Editor) {
        for store in editor.take_library_reopens() {
            match store {
                LibraryStore::Sounds => {
                    if self.sounds.is_err() {
                        self.sounds = open_sounds(&self.root);
                    }
                    self.lend_sounds(editor);
                }
                LibraryStore::Music => {
                    if self.music.is_err() {
                        self.music = open_music(&self.root);
                    }
                    self.lend_music(editor);
                }
            }
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

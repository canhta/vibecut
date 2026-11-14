//! The one lending point: the process stores and the model service every editor borrows.
use super::window::EditorWindow;
use crate::{
    Editor, Error,
    editor::LibraryStore,
    model_manager::ModelManager,
    services::{ai_access::ModelCache, assets::music::MusicSession},
};
use engine::{
    ByteBudget, MusicError, SoundError,
    control::{MediaAccess, VisionAccess},
};
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
    /// The installed local face detector behind the shorts review's shot layouts. It reads the
    /// shared verified model cache on each load and never downloads.
    vision: Arc<VisionAccess>,
    /// The one models cache the model manager and the face detector both read.
    model_cache: Arc<ModelCache>,
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
        let model_cache = Arc::new(ModelCache::new(budget.clone()));
        // The workspace root keeps test runs isolated and never races the CLI's cache root.
        // Background removal loads its segmenter through the same verified models cache.
        let media = MediaAccess::open_in(&root.with_file_name("media"), budget.clone())
            .ok()
            .map(|mut access| {
                access.set_segmentation(::shorts::installed_person_matte(model_cache.lookup()));
                Arc::new(Mutex::new(access))
            });
        let brand = BrandKit::open(&root.with_file_name("brand_kits"))
            .ok()
            .map(Arc::new);
        Self {
            root: root.to_path_buf(),
            sounds: open_sounds(root),
            music: open_music(root),
            media,
            brand,
            vision: Arc::new(::shorts::installed_faces(model_cache.lookup())),
            model_cache,
            models: None,
        }
    }
    pub(super) fn media(&self) -> Option<Arc<Mutex<MediaAccess>>> {
        self.media.clone()
    }
    /// The footage lookup the stores draw cover images with, over the shared media cache; a
    /// process whose cache did not open captures covers uncorrected.
    pub(super) fn footage(&self) -> engine::footage::Lookup {
        match self.media.clone() {
            Some(media) => MediaAccess::footage_lookup(media),
            None => engine::footage::uncorrected(),
        }
    }
    pub(super) fn vision(&self) -> Arc<VisionAccess> {
        self.vision.clone()
    }
    pub(super) fn brand(&self) -> Option<Arc<BrandKit>> {
        self.brand.clone()
    }
    /// Give a newly registered editor the shared stores, or the reason each did not open.
    pub(super) fn lend_stores(&self, editor: &mut Editor) {
        self.lend_sounds(editor);
        self.lend_music(editor);
        editor.set_model_cache(self.model_cache.clone());
        editor.set_agent_history(self.root.with_file_name("agent_history"));
        if let Some(media) = &self.media {
            editor.set_media(media.clone());
        }
        editor.offer_link_media();
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
    /// Serve the stores any open editor asked to open again. A store that opened
    /// before is lent as it stands to the editors that asked; one that failed is
    /// opened again here, by its only owner, once per request batch. A store that
    /// opens now is lent to every open editor, since each showed the same failure.
    pub(super) fn reopen_requested(&mut self, editors: &mut [&mut Editor]) {
        let asked: Vec<Vec<LibraryStore>> = editors
            .iter_mut()
            .map(|editor| editor.take_library_reopens())
            .collect();
        let wanted = |store| asked.iter().any(|stores| stores.contains(&store));
        let mut reopened = Vec::new();
        if wanted(LibraryStore::Sounds) && self.sounds.is_err() {
            self.sounds = open_sounds(&self.root);
            if self.sounds.is_ok() {
                reopened.push(LibraryStore::Sounds);
            }
        }
        if wanted(LibraryStore::Music) && self.music.is_err() {
            self.music = open_music(&self.root);
            if self.music.is_ok() {
                reopened.push(LibraryStore::Music);
            }
        }
        for (editor, asked) in editors.iter_mut().zip(&asked) {
            for store in [LibraryStore::Sounds, LibraryStore::Music] {
                if !reopened.contains(&store) && !asked.contains(&store) {
                    continue;
                }
                match store {
                    LibraryStore::Sounds => self.lend_sounds(editor),
                    LibraryStore::Music => self.lend_music(editor),
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

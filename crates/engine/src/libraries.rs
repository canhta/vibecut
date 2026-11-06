//! Provides external assets for project use.
mod font_catalog;
pub mod fonts {
    //! Font catalog: bundled, system and curated Google families, lazy loading into `text`, and an
    pub use super::font_catalog::{
        FontEntry, FontError, FontLoad, FontLoadState, FontOrigin, FontState, FontStore,
        GoogleFamily, GoogleFile, MAX_SCANNED_FACES, ScannedFace, curated_google_families,
        scan_directory, scan_system,
    };
}
mod sound_library;
pub mod sounds {
    //! Bring-your-own-credential sound search over the public Freesound APIv2, plus a
    pub use super::sound_library::{
        ADD_COMMAND, CachedSound, Error, PreviewClip, PreviewSink, SAVE_COMMAND, SEARCH_COMMAND,
        SavedSound, SearchPage, SearchRequest, SearchSource, Sound, SoundClient, SoundDownloads,
        SoundLibrary, SoundPreview, SoundResult, Token, add_saved, add_to_timeline, commercial,
    };
}

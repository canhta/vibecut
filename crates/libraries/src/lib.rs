//! Provides external assets for project use.
mod font_catalog;
pub mod fonts {
    //! Resolves font assets for project rendering.
    pub use super::font_catalog::{
        FontStore, GoogleFamily, GoogleFile, MAX_SCANNED_FACES, ScannedFace,
        curated_google_families, scan_directory, scan_system,
    };
}
mod sound_library;
pub mod sounds {
    //! Provides licensed sound assets for projects.
    pub use super::sound_library::{
        ADD_COMMAND, CachedSound, PreviewClip, PreviewSink, SAVE_COMMAND, SEARCH_COMMAND,
        SavedSound, SearchPage, SearchRequest, SearchSource, Sound, SoundClient, SoundDownloads,
        SoundLibrary, SoundPreview, SoundResult, Token, add_saved, add_to_timeline, commercial,
    };
}

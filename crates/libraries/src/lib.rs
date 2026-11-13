//! Provides external assets for project use.
pub mod brand_kit;
mod font_catalog;
pub mod fonts {
    //! Resolves font assets for project rendering.
    pub use super::font_catalog::{
        FontFile, FontStore, GoogleFamily, GoogleFile, LocalFontFile, MAX_SCANNED_FACES,
        ScannedFace, curated_google_families, scan_directory, scan_system,
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
mod sticker_library;
pub mod stickers {
    //! Provides bundled and downloadable stickers for the Stickers panel.
    pub use super::sticker_library::{
        Category, Collection, CollectionState, DEFAULT_SEARCH_LIMIT, Entry, FETCHED_SIZE,
        FLAG_SIZE, FLAGS_PROVIDER, FlagMatch, FlagPage, Item, MAX_RECENT_STICKERS, MAX_SVG_BYTES,
        Page, Recents, StickerStore, Unavailable, build_sticker_id, check_svg, curated_collections,
        insert_sticker, is_offline, parse_sticker_id, resolve_sticker, search_catalog,
        search_flags,
    };
}
mod music_library;
pub mod music {
    //! Provides openly licensed music for projects.
    pub use super::music_library::{
        ADD_COMMAND, CHIPS, CachedMusic, Music, MusicClient, MusicDownloads, MusicLibrary,
        MusicPreview, MusicResult, MusicToken, PreviewClip, PreviewSink, SAVE_COMMAND,
        SEARCH_COMMAND, SavedMusic, SearchPage, SearchRequest, SearchSource, add_saved,
        add_to_timeline,
    };
}

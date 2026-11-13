//! Validated project, scene, track and item snapshots with whole-project invariants.

macro_rules! id {
    ($($name:ident),+) => {$ (
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, schemars::JsonSchema)]
        #[serde(transparent)]
        pub struct $name(pub u64);
    )+};
}
mod audio_cleanup;
mod item;
mod narration;
mod project;
mod scene;
mod settings;
mod shorts_run;
mod track;

pub use audio_cleanup::{
    AudioCleanup, CleanupEffect, CleanupKind, CleanupPreset, EqBand, EqBandKind,
    GRAPHIC_EQ_FREQUENCIES_HZ, MAX_CLEANUP_EFFECTS, MAX_EQ_BANDS, MAX_HUM_HARMONICS,
    MainsFrequency, NOISE_REDUCTION_DEFAULT, VOICE_ENHANCEMENT_EFFECTS, voice_enhancement_effects,
};
pub use item::{
    AudioParams, AudioSource, BackgroundRemoval, CaptionParams, CaptionWordWarning,
    CaptionWordWarnings, ClipBackground, EffectParams, GraphicParams, Item, ItemId, ItemParams,
    LibraryMusic, LibrarySound, MusicLicence, OverlayParams, StabilizeLevel, StickerParams,
    TextAlign, TextBackground, TextCase, TextDecoration, TextParams, TextShadow, TextStroke,
    TextStyle, VOLUME_DB_RANGE, VideoParams, WordSpan, valid_volume,
};
pub use narration::{
    MAX_DUCK_MS, MAX_DUCK_TENTHS_DB, Narration, NarrationConsent, NarrationCost, NarrationDucking,
    PROVIDER_SPEED_THOUSANDTHS,
};
pub use project::{
    CaptionPresetId, MAX_CAPTION_PRESETS, MAX_RECENT_COLORS, MAX_TEXT_PRESETS, Project,
    ProjectData, ProjectId, SavedCaptionPreset, SavedTextPreset, TextPresetId,
};
pub use scene::{Bookmark, BookmarkId, Scene, SceneId, SceneScope, Scenes};
pub use settings::{
    AspectPreset, Canvas, CanvasSize, Color, Loudness, LoudnessGains, LoudnessScope,
    LoudnessTarget, Settings,
};
pub use shorts_run::{
    MAX_RUN_CLIPS, MAX_RUN_ENERGY, MAX_RUN_PIECES, ShortsClip, ShortsEnergy, ShortsPiece,
    ShortsRequest, ShortsRole, ShortsRun, ShortsSegment, ShortsSource,
};
pub use track::{Timeline, Track, TrackId, TrackKind};

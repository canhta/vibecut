//! Validated project, scene, track and item snapshots with whole-project invariants.

macro_rules! id {
    ($($name:ident),+) => {$ (
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, schemars::JsonSchema)]
        #[serde(transparent)]
        pub struct $name(pub u64);
    )+};
}
mod item;
mod project;
mod scene;
mod settings;
mod track;

pub use item::{
    AudioParams, AudioSource, CaptionParams, Item, ItemId, ItemParams, LibrarySound, OverlayParams,
    TextAlign, TextBackground, TextDecoration, TextParams, TextPosition, VideoParams, valid_volume,
};
pub use project::{Project, ProjectData, ProjectId};
pub use scene::{Bookmark, BookmarkId, Scene, SceneId, SceneScope, Scenes};
pub use settings::{
    AspectPreset, Canvas, CanvasSize, Color, Loudness, LoudnessGains, LoudnessScope,
    LoudnessTarget, Settings,
};
pub use track::{Timeline, Track, TrackId, TrackKind};

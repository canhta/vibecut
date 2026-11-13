//! The cache-only clip display contract areas read through [`Context`](super::Context).

use crate::editor::{AssetId, ItemId, egui};

/// One ready waveform column, linear amplitudes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct WaveBar {
    pub(in crate::editor) min: f32,
    pub(in crate::editor) max: f32,
    /// Any sample crossed full scale, so the display draws a `> 0 dBFS` tip.
    pub(in crate::editor) over: bool,
}

/// Cache-only waveform state for one clip's visible range.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum WaveState {
    Ready(Vec<WaveBar>),
    Pending,
    Failed,
    Unsupported,
}

/// One cached strip tile: the output-time slice it fills and its frame texture.
#[derive(Clone)]
pub(crate) struct StripTile {
    pub(in crate::editor) output: time::TimeRange,
    pub(in crate::editor) texture: egui::TextureHandle,
}

/// Cache-only thumbnail strip state for one clip's visible range.
#[derive(Clone)]
pub(crate) enum StripState {
    Ready(Vec<StripTile>),
    Pending,
    Failed,
    Unsupported,
}

/// A generation command the panel asks the frame to run.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum VisualRequest {
    Waveform {
        asset: AssetId,
    },
    Thumbnails {
        item: ItemId,
        visible: time::TimeRange,
        pixels_per_second: f64,
    },
}
impl VisualRequest {
    /// The cache family one in-flight job serves.
    pub(in crate::editor) fn key(&self) -> VisualKey {
        match self {
            Self::Waveform { asset } => VisualKey::Waveform(*asset),
            Self::Thumbnails { item, .. } => VisualKey::Thumbnails(*item),
        }
    }
    /// The registered `JobControl` command.
    pub(in crate::editor) fn command(&self) -> engine::Command {
        match self {
            Self::Waveform { asset } => engine::Command::GenerateWaveform { asset: *asset },
            Self::Thumbnails {
                item,
                visible,
                pixels_per_second,
            } => engine::Command::GenerateClipThumbnails {
                item: *item,
                visible: *visible,
                pixels_per_second: *pixels_per_second,
            },
        }
    }
}

/// The cache family one in-flight job serves.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(in crate::editor) enum VisualKey {
    Waveform(AssetId),
    Thumbnails(ItemId),
}

/// Read-only clip visual queries shared by every timeline area.
pub(crate) trait VisualQuery {
    fn strip(
        &self,
        ctx: &egui::Context,
        asset: &media::SourceAsset,
        item: &project::Item,
        visible: time::TimeRange,
        pixels_per_second: f64,
    ) -> StripState;
    fn waveform(
        &self,
        asset: &media::SourceAsset,
        item: &project::Item,
        visible: time::TimeRange,
        columns: usize,
    ) -> WaveState;
    /// Whether the asset's source file is still on disk, as the engine's media
    /// status reports it. A missing source keeps its asset record (the relink
    /// state) and asks for no strip or waveform.
    fn availability(&self, asset: &media::SourceAsset) -> media::AssetAvailability;
    /// Drain the generation commands the last paint asked for.
    fn take_requests(&self) -> Vec<VisualRequest>;
}

//! Shared CPU pixels, absolute-sample audio, and finalized export execution.
mod audio;
mod export;
mod image;
mod scene;
mod sources;

pub use audio::{AudioBlock, AudioMixer, LoudnessReport, Measurement, measure_loudness, mix_audio};
pub use export::{
    ExportMode, ExportReceipt, ExportSettings, HdrPolicy, OutputOptions, RangeChoice, SceneChoice,
    VideoCodec, export, export_with_sources, validate_output_path,
};
pub use image::Image;
use media::AssetId;
pub use scene::effects::lut;
pub use scene::{
    CaptionBackground, CaptionOutline, CaptionPosition, CaptionStyle, FLAGS_PROVIDER, Fit,
    MAX_STICKER_RASTER_SIDE, Placement, RasterCacheStats, Rect, RenderSettings, Renderer,
    SHAPES_PROVIDER, StickerError, StickerRaster, rasterize_sticker, style_from_cue,
};
pub use scene::{
    arrange, blend, color, effects, mask, transition_thumbnail, transitions, viewport,
};
pub use sources::{FileSources, SourceProvider};
use time::SourceRange;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemoryKind {
    Decoded,
    Compositor,
    Export,
}
pub trait Reservation: Send + Sync {}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Admit,
    Render,
    Mix,
    Encode,
    Finalize,
    Finalized,
    Publish,
}
/// Caller/engine owns cancellation, scheduling and the aggregate byte policy.
/// Reserve before allocating and retain the returned lease for its entire lifetime.
pub trait ExecutionHooks: Sync {
    fn cancelled(&self) -> bool;
    /// Bounded pixel/conversion concurrency admitted by the scheduling owner.
    /// Providers without a scheduler remain sequential; values are clamped to 1..=8.
    fn parallelism(&self) -> usize {
        1
    }
    /// Execute disjoint borrowed kernels and join every task before returning,
    /// including cancellation, failure and unwind. At most eight tasks are submitted.
    fn run_tasks(&self, tasks: &mut [&mut (dyn FnMut() -> Result<()> + Send)]) -> Result<()> {
        for task in tasks {
            task()?;
        }
        Ok(())
    }
    fn reserve(&self, kind: MemoryKind, bytes: u64) -> Result<Box<dyn Reservation>>;
    /// Concurrent cancellation owners fence cancellation before irreversible publication.
    fn begin_publish(&self) -> Result<()> {
        if self.cancelled() {
            Err(Error::Cancelled)
        } else {
            Ok(())
        }
    }
    fn progress(&self, _stage: Stage, _fraction: f32) -> Result<()> {
        Ok(())
    }
}
pub(crate) fn conversion_reservation(
    hooks: &dyn ExecutionHooks,
    kind: MemoryKind,
    base: u64,
) -> Result<(u8, Box<dyn Reservation>)> {
    let mut threads = hooks.parallelism().clamp(1, 8) as u8;
    loop {
        let bytes = base
            .checked_add(u64::from(threads - 1) * (8 << 20))
            .ok_or(Error::Invalid("conversion memory bound"))?;
        match hooks.reserve(kind, bytes) {
            Err(Error::Budget { .. }) if threads > 1 => threads -= 1,
            result => return result.map(|lease| (threads, lease)),
        }
    }
}
pub(crate) fn check(hooks: &dyn ExecutionHooks) -> Result<()> {
    if hooks.cancelled() {
        Err(Error::Cancelled)
    } else {
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnavailableReason {
    Missing,
    Changed,
    Metadata,
    Video,
    Audio,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unavailable {
    pub asset: AssetId,
    pub item: project::ItemId,
    pub range: SourceRange,
    pub reason: UnavailableReason,
}
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid render input: {0}")]
    Invalid(&'static str),
    #[error("unsupported render capability: {0}")]
    Unsupported(&'static str),
    #[error("pixel worker unwound")]
    WorkerPanicked,
    #[error("render cancelled")]
    Cancelled,
    #[error("render allocation of {requested} bytes exceeds available budget {limit}")]
    Budget { requested: u64, limit: u64 },
    #[error("render allocation failed")]
    Allocation(#[from] std::collections::TryReserveError),
    #[error("required sources unavailable")]
    Unavailable(Vec<Unavailable>),
    #[error("audio mix peak {peak} exceeds full scale; reduce gain before export")]
    AudioOverload { peak: f32 },
    #[error("invalid cube LUT at line {line}")]
    InvalidLut {
        line: u32,
        reason: lut::LutInvalidReason,
    },
    #[error("required LUT unavailable")]
    LutUnavailable {
        id: project::LutId,
        reason: lut::LutUnavailableReason,
    },
    #[error("render IO failed")]
    Io(#[from] std::io::Error),
    #[error("render media failed")]
    Media(#[from] media::MediaError),
    #[error("sticker unavailable")]
    Sticker(#[from] StickerError),
    #[error("render project failed")]
    Project(#[from] project::Error),
    #[error("render time failed")]
    Time(#[from] time::TimeError),
    #[error("text layout failed")]
    Text(#[from] text::TextError),
    #[error("text raster failed")]
    Raster(#[from] text::RenderError),
}
pub type Result<T> = std::result::Result<T, Error>;
pub(crate) fn vector<T: Clone>(len: usize, value: T) -> Result<Vec<T>> {
    let mut output = Vec::new();
    output.try_reserve_exact(len)?;
    output.resize(len, value);
    Ok(output)
}

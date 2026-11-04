//! Shared CPU pixels, absolute-sample audio, and finalized export execution.
mod audio;
mod caption;
pub use caption::{CaptionBackground, CaptionOutline, CaptionPosition, CaptionStyle};
mod export;
mod image;
mod scene;
mod sources;
pub use audio::{AudioBlock, mix_audio};
pub use export::{
    ExportMode, ExportReceipt, ExportSettings, HdrPolicy, export, export_with_sources,
    validate_output_path,
};
pub use image::Image;
use media::AssetId;
pub use scene::{Fit, Placement, RasterCacheStats, Rect, RenderSettings, Renderer};
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
    #[error("render IO failed")]
    Io(#[from] std::io::Error),
    #[error("render media failed")]
    Media(#[from] media::MediaError),
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

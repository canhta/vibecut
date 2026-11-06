//! Identifies a source, describes its streams and summarizes what it contains.

mod anchors;
mod asset;
mod model;
mod probe;
mod summary;

pub(crate) use anchors::{MfccAnchor, MfccAnchors};
pub use asset::{
    AssetAvailability, AssetData, AssetId, ContentHash, SourceAsset, SourceProbe, SourceStream,
};
pub use model::{CodecParameters, Packet, TimeBase};
pub use probe::{AudioInfo, ColorInfo, Probe, StreamInfo, StreamKind, VideoInfo};
pub use summary::{
    Decodability, DisplayFrameRate, ImportedSource, MediaKind, MediaMetadata, ProbeCapabilities,
    StreamCapability, UnsupportedReason,
};

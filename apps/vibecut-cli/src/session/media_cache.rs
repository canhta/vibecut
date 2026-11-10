//! Opens the artifact caches media generation commands publish through.
use crate::args::CliError;
use engine::ByteBudget;
use engine::cache::ArtifactCache;
use engine::thumbnails::ThumbnailCache;

const MAX_ENTRIES: usize = 128;

/// Source peak summaries, keyed by content and audio layout.
pub(crate) fn waveform_cache(budget: ByteBudget) -> Result<ArtifactCache, CliError> {
    let root = engine::providers::default_cache_root()?.join("waveform");
    Ok(ArtifactCache::open(&root, budget, MAX_ENTRIES)?.0)
}

/// Sampled clip frames, keyed by content, source position and edge.
pub(crate) fn thumbnail_cache(budget: ByteBudget) -> Result<ThumbnailCache, CliError> {
    let root = engine::providers::default_cache_root()?.join("thumbnails");
    Ok(ThumbnailCache::open(&root, budget, MAX_ENTRIES)?)
}

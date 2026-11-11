//! Content-keyed thumbnail artifacts.
//!
//! Thumbnails belong to the engine's one artifact cache policy: the key binds
//! the source content hash, the requested frame position and the output edge,
//! so changed content or a different frame invalidates it. A decode failure is
//! never a hit: corrupt bytes fall back to regeneration.
use super::{Error as ArtifactError, Result as ArtifactResult};
use crate::ByteBudget;
use crate::cache::{ArtifactCache, ArtifactKind, CacheKey, KeyInputs};
use media::{ContentHash, SourceAsset, THUMBNAIL_MAX_EDGE, Thumbnail};
use std::path::Path;
use time::Tick;

const PROVIDER: &str = "vibecut";
const MODEL: &str = "thumbnail";
const VERSION: &str = "3";

/// Canonical key for one decoded frame. `at == Tick::ZERO` means the first
/// decodable frame.
pub fn thumbnail_key(content: ContentHash, at: Tick, max_edge: u32) -> ArtifactResult<CacheKey> {
    if max_edge == 0 || max_edge > THUMBNAIL_MAX_EDGE {
        return Err(ArtifactError::Edit(crate::session::EditError::Invalid(
            "thumbnail max edge",
        )));
    }
    let mut frame = blake3::Hasher::new();
    frame.update(b"vibecut-thumbnail-frame-v1");
    frame.update(&at.0.to_le_bytes());
    let settings = max_edge.to_le_bytes();
    CacheKey::new(KeyInputs {
        kind: ArtifactKind::Thumbnail,
        source: content.0,
        provider: PROVIDER,
        model: MODEL,
        version: VERSION,
        settings: &settings,
        transcript: [0; 32],
        cues: [0; 32],
        clip_range: *frame.finalize().as_bytes(),
        translation: [0; 32],
    })
    .map_err(ArtifactError::from)
}

/// Shared retained pixels and allowance for CPU image, upload staging and GPU
/// texture. Cloning the handle never clones pixels or loses their accounting.
#[derive(Clone)]
pub struct RetainedThumbnail(std::sync::Arc<RetainedImage>);
struct RetainedImage {
    image: Thumbnail,
    probe: media::SourceProbe,
    _lease: crate::ByteLease,
}
impl std::fmt::Debug for RetainedThumbnail {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.image.fmt(f)
    }
}
impl std::ops::Deref for RetainedThumbnail {
    type Target = Thumbnail;
    fn deref(&self) -> &Thumbnail {
        &self.0.image
    }
}
impl RetainedThumbnail {
    /// True for clones of the same retained image, including its upload lease.
    pub fn shares_pixels(&self, other: &Self) -> bool {
        std::sync::Arc::ptr_eq(&self.0, &other.0)
    }
    /// Geometry follows pixels through memory-cache eviction, without host state.
    pub fn probe(&self) -> &media::SourceProbe {
        &self.0.probe
    }
    fn probe_bytes(probe: &media::SourceProbe) -> u64 {
        (std::mem::size_of::<media::SourceProbe>()
            + probe.format.capacity()
            + probe.streams.capacity() * std::mem::size_of::<media::SourceStream>()
            + probe
                .streams
                .iter()
                .map(|stream| stream.codec.capacity())
                .sum::<usize>()) as u64
    }
    /// The generator's temporary allocation belongs to its working allowance;
    /// transfer to this lease before releasing that allowance.
    pub fn new(image: Thumbnail, asset: &SourceAsset, budget: &ByteBudget) -> ArtifactResult<Self> {
        if image.width == 0
            || image.height == 0
            || image.width > THUMBNAIL_MAX_EDGE
            || image.height > THUMBNAIL_MAX_EDGE
            || u64::from(image.width) * u64::from(image.height) * 4 != image.pixels.len() as u64
        {
            return Err(ArtifactError::Edit(crate::session::EditError::Invalid(
                "retained thumbnail geometry",
            )));
        }
        let lease = budget.reserve(
            crate::ByteOwner::DecodedFrames,
            image.pixels.capacity() as u64
                + image.pixels.len() as u64 * 3
                + Self::probe_bytes(asset.probe()),
        )?;
        Ok(Self(std::sync::Arc::new(RetainedImage {
            image,
            probe: asset.probe().clone(),
            _lease: lease,
        })))
    }
}

/// One cache read: a decoded hit, an absent entry or an entry that must be
/// regenerated.
pub(crate) enum ThumbnailLookup {
    Hit(RetainedThumbnail),
    Missing,
    Corrupt,
}

/// One exclusive thumbnail cache directory, reusing the engine artifact policy.
pub struct ThumbnailCache {
    cache: ArtifactCache,
    budget: ByteBudget,
}

impl ThumbnailCache {
    /// Registered disk artifacts only; does not count retained decoded images.
    pub fn maintenance(&self) -> crate::runtime::Result<crate::cache::CacheMaintenance> {
        self.cache.maintenance()
    }

    pub fn open(
        root: &Path,
        budget: ByteBudget,
        max_entries: usize,
    ) -> crate::runtime::Result<Self> {
        let (cache, _diagnostics) = ArtifactCache::open(root, budget.clone(), max_entries)?;
        Ok(Self { cache, budget })
    }

    // Geometry and pixels share an envelope and eviction lifetime: a surviving
    // frame hit must never lose the metadata that prevents recovery re-decode.
    fn unpack(bytes: &[u8], content: ContentHash) -> Option<(media::SourceProbe, &[u8])> {
        if bytes.get(..8)? != b"VTCIMG03" {
            return None;
        }
        let length = u32::from_le_bytes(bytes.get(8..12)?.try_into().ok()?) as usize;
        if length > 1 << 20 {
            return None;
        }
        let end = 12_usize.checked_add(length)?;
        let asset: SourceAsset = serde_json::from_slice(bytes.get(12..end)?).ok()?;
        if asset.content_hash() != content {
            return None;
        }
        Some((media::AssetData::from(asset).probe, bytes.get(end..)?))
    }
    /// Validated metadata from the default first-frame thumbnail entry, without
    /// decoding pixels. Host capabilities are always reopened on this machine.
    pub fn get_probe(&mut self, content: ContentHash) -> Option<media::SourceProbe> {
        let key = thumbnail_key(content, Tick::ZERO, THUMBNAIL_MAX_EDGE).ok()?;
        let lease = self.cache.get(key).ok()??;
        Self::unpack(lease.bytes(), content).map(|(probe, _)| probe)
    }

    /// A missing, corrupt or undecodable entry is `None`, never a partial image.
    pub fn get(
        &mut self,
        asset: &SourceAsset,
        at: Tick,
        max_edge: u32,
    ) -> ArtifactResult<Option<RetainedThumbnail>> {
        Ok(match self.lookup(asset, at, max_edge)? {
            ThumbnailLookup::Hit(image) => Some(image),
            ThumbnailLookup::Missing | ThumbnailLookup::Corrupt => None,
        })
    }

    /// Cache read that keeps a corrupt entry distinguishable from a miss, so a
    /// display query can report `Failed` while generation treats both as misses.
    pub(crate) fn lookup(
        &mut self,
        asset: &SourceAsset,
        at: Tick,
        max_edge: u32,
    ) -> ArtifactResult<ThumbnailLookup> {
        let key = thumbnail_key(asset.content_hash(), at, max_edge)?;
        let lease = match self.cache.get(key) {
            Ok(lease) => lease,
            Err(crate::RuntimeError::CacheCorrupt) => {
                return Ok(ThumbnailLookup::Corrupt);
            }
            Err(error) => return Err(error.into()),
        };
        let Some(lease) = lease else {
            return Ok(ThumbnailLookup::Missing);
        };
        // Reserve the decoded image, CPU upload and GPU allowance before the
        // payload decoder allocates pixels. The cache read lease stays pinned.
        let Some((probe, payload)) = Self::unpack(lease.bytes(), asset.content_hash()) else {
            return Ok(ThumbnailLookup::Corrupt);
        };
        let pixels = payload.len().saturating_sub(24) as u64;
        let image_lease = self.budget.reserve(
            crate::ByteOwner::DecodedFrames,
            pixels * 4 + RetainedThumbnail::probe_bytes(&probe),
        )?;
        let decoded = Thumbnail::decode(payload);
        drop(lease);
        match decoded {
            Ok(image) if image.width <= max_edge && image.height <= max_edge => Ok(
                ThumbnailLookup::Hit(RetainedThumbnail(std::sync::Arc::new(RetainedImage {
                    image,
                    probe,
                    _lease: image_lease,
                }))),
            ),
            // Replaced on the next successful put.
            Ok(_) | Err(_) => Ok(ThumbnailLookup::Corrupt),
        }
    }

    pub fn put(
        &mut self,
        asset: &SourceAsset,
        at: Tick,
        max_edge: u32,
        thumbnail: &Thumbnail,
    ) -> ArtifactResult<()> {
        let key = thumbnail_key(asset.content_hash(), at, max_edge)?;
        let metadata = serde_json::to_vec(asset).map_err(|_| {
            ArtifactError::Edit(crate::session::EditError::Invalid(
                "thumbnail probe metadata",
            ))
        })?;
        if metadata.len() > 1 << 20 {
            return Err(crate::RuntimeError::ArtifactTooLarge.into());
        }
        let image = thumbnail.encode();
        let mut payload = Vec::with_capacity(12 + metadata.len() + image.len());
        payload.extend_from_slice(b"VTCIMG03");
        payload.extend_from_slice(&(metadata.len() as u32).to_le_bytes());
        payload.extend_from_slice(&metadata);
        payload.extend_from_slice(&image);
        self.cache.put(key, &payload).map_err(ArtifactError::from)
    }

    /// Cache hit without decoding; otherwise decode once and publish. The
    /// generation closure is only called on a miss or a corrupt entry.
    pub fn get_or_generate(
        &mut self,
        asset: &SourceAsset,
        at: Tick,
        max_edge: u32,
        generate: impl FnOnce() -> media::Result<Thumbnail>,
    ) -> ArtifactResult<RetainedThumbnail> {
        if let Some(thumbnail) = self.get(asset, at, max_edge)? {
            return Ok(thumbnail);
        }
        let thumbnail = generate()?;
        self.put(asset, at, max_edge, &thumbnail)?;
        RetainedThumbnail::new(thumbnail, asset, &self.budget)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;

    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    fn sample_asset(id: u64, hash: u8) -> SourceAsset {
        testkit::source::asset(id, std::env::temp_dir().join("fixture.mp4"))
            .format("mov,mp4")
            .duration(Tick(1_000))
            .hash([hash; 32])
            .stream(testkit::source::video().duration(Tick(1_000)).build())
            .build()
    }

    fn thumbnail() -> Thumbnail {
        Thumbnail {
            width: 2,
            height: 1,
            frame: Tick(0),
            pixels: vec![9, 8, 7, 6, 5, 4, 3, 2],
        }
    }

    #[test]
    fn retained_pixels_share_accounting_and_release_only_after_the_last_owner() {
        let budget = ByteBudget::new(1 << 20).unwrap();
        let asset = sample_asset(1, 7);
        let image = RetainedThumbnail::new(thumbnail(), &asset, &budget).unwrap();
        let charge = budget.snapshot().unwrap().used;
        assert!(charge >= 32);
        assert_eq!(image.probe(), asset.probe());
        let ui = image.clone();
        drop(image);
        assert_eq!(budget.snapshot().unwrap().used, charge);
        let pressure = budget.reserve(crate::ByteOwner::JobResults, (1 << 20) - charge + 1);
        assert!(matches!(pressure, Err(crate::RuntimeError::BudgetFull)));
        drop(ui);
        assert_eq!(budget.snapshot().unwrap().used, 0);
        assert!(
            budget
                .reserve(crate::ByteOwner::JobResults, 1 << 20)
                .is_ok()
        );
    }

    #[test]
    fn cached_probe_keeps_metadata_and_excludes_host_capabilities() {
        let directory = tempfile::tempdir().unwrap();
        let mut cache =
            ThumbnailCache::open(directory.path(), ByteBudget::new(1 << 20).unwrap(), 16).unwrap();
        let asset = sample_asset(1, 7);
        cache
            .put(&asset, Tick::ZERO, THUMBNAIL_MAX_EDGE, &thumbnail())
            .unwrap();
        assert_eq!(
            std::fs::read_dir(directory.path())
                .unwrap()
                .filter_map(|e| e.ok())
                .filter(|e| e.file_name() != ".cache.lock")
                .count(),
            1,
            "probe geometry and image must have one eviction lifetime"
        );
        assert_eq!(
            cache.get_probe(asset.content_hash()),
            Some(asset.probe().clone())
        );
        assert!(cache.get_probe(ContentHash([8; 32])).is_none());
    }

    #[test]
    fn key_binds_content_position_and_edge() {
        let content = ContentHash([1; 32]);
        let base = thumbnail_key(content, Tick::ZERO, 320).unwrap();
        assert_eq!(base, thumbnail_key(content, Tick::ZERO, 320).unwrap());
        assert_ne!(base, thumbnail_key(content, Tick(1), 320).unwrap());
        assert_ne!(
            base,
            thumbnail_key(ContentHash([2; 32]), Tick::ZERO, 320).unwrap()
        );
        assert_ne!(base, thumbnail_key(content, Tick::ZERO, 160).unwrap());
        assert!(thumbnail_key(content, Tick::ZERO, 0).is_err());
    }

    #[test]
    fn hit_does_not_decode_and_content_change_misses() {
        let directory = tempfile::tempdir().unwrap();
        let mut cache =
            ThumbnailCache::open(directory.path(), ByteBudget::new(8 << 20).unwrap(), 16).unwrap();
        let asset = sample_asset(1, 7);
        let decodes = Arc::new(AtomicUsize::new(0));
        let generate = |decodes: Arc<AtomicUsize>| {
            decodes.fetch_add(1, Ordering::SeqCst);
            Ok(thumbnail())
        };
        let first = cache
            .get_or_generate(&asset, Tick::ZERO, 320, || generate(decodes.clone()))
            .unwrap();
        assert_eq!(&*first, &thumbnail());
        let second = cache
            .get_or_generate(&asset, Tick::ZERO, 320, || generate(decodes.clone()))
            .unwrap();
        assert_eq!(&*second, &thumbnail());
        assert_eq!(decodes.load(Ordering::SeqCst), 1);
        let changed = sample_asset(1, 8);
        cache
            .get_or_generate(&changed, Tick::ZERO, 320, || generate(decodes.clone()))
            .unwrap();
        assert_eq!(decodes.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn corrupt_cache_falls_back_to_regeneration() {
        let directory = tempfile::tempdir().unwrap();
        let asset = sample_asset(1, 7);
        let mut cache =
            ThumbnailCache::open(directory.path(), ByteBudget::new(8 << 20).unwrap(), 16).unwrap();
        cache
            .get_or_generate(&asset, Tick::ZERO, 320, || Ok(thumbnail()))
            .unwrap();
        let file = std::fs::read_dir(directory.path())
            .unwrap()
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .find(|path| {
                path.file_name()
                    .is_some_and(|name| name != std::ffi::OsStr::new(".cache.lock"))
            })
            .unwrap();
        std::fs::write(&file, b"not a cache envelope").unwrap();
        let decodes = Arc::new(AtomicUsize::new(0));
        let recovered = cache
            .get_or_generate(&asset, Tick::ZERO, 320, || {
                decodes.fetch_add(1, Ordering::SeqCst);
                Ok(thumbnail())
            })
            .unwrap();
        assert_eq!(&*recovered, &thumbnail());
        assert_eq!(decodes.load(Ordering::SeqCst), 1);
    }
}

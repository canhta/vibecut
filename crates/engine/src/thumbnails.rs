//! Content-keyed thumbnail artifacts.
//!
//! Thumbnails belong to the engine's one artifact cache policy: the key binds
//! the source content hash, the requested frame position and the output edge,
//! so changed content or a different frame invalidates it. A decode failure is
//! never a hit: corrupt bytes fall back to regeneration.
use crate::cache::{ArtifactCache, ArtifactKind, CacheKey, KeyInputs};
use crate::{ByteBudget, EditError, Result};
use media::{ContentHash, SourceAsset, THUMBNAIL_MAX_EDGE, Thumbnail};
use std::path::Path;
use time::Tick;

const PROVIDER: &str = "vibecut";
const MODEL: &str = "thumbnail";
const VERSION: &str = "1";

/// Canonical key for one decoded frame. `at == Tick::ZERO` means the first
/// decodable frame.
pub fn thumbnail_key(content: ContentHash, at: Tick, max_edge: u32) -> Result<CacheKey> {
    if max_edge == 0 || max_edge > THUMBNAIL_MAX_EDGE {
        return Err(EditError::Invalid("thumbnail max edge"));
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
}

/// One exclusive thumbnail cache directory, reusing the engine artifact policy.
pub struct ThumbnailCache {
    cache: ArtifactCache,
}

impl ThumbnailCache {
    pub fn open(root: &Path, budget: ByteBudget, max_entries: usize) -> Result<Self> {
        let (cache, _diagnostics) = ArtifactCache::open(root, budget, max_entries)?;
        Ok(Self { cache })
    }

    /// A missing, corrupt or undecodable entry is `None`, never a partial image.
    pub fn get(
        &mut self,
        asset: &SourceAsset,
        at: Tick,
        max_edge: u32,
    ) -> Result<Option<Thumbnail>> {
        let key = thumbnail_key(asset.content_hash(), at, max_edge)?;
        let lease = match self.cache.get(key) {
            Ok(lease) => lease,
            Err(EditError::CacheCorrupt) => return Ok(None),
            Err(error) => return Err(error),
        };
        let Some(lease) = lease else {
            return Ok(None);
        };
        let decoded = Thumbnail::decode(lease.bytes());
        drop(lease);
        match decoded {
            Ok(thumbnail) => Ok(Some(thumbnail)),
            // Replaced on the next successful put.
            Err(_) => Ok(None),
        }
    }

    pub fn put(
        &mut self,
        asset: &SourceAsset,
        at: Tick,
        max_edge: u32,
        thumbnail: &Thumbnail,
    ) -> Result<()> {
        let key = thumbnail_key(asset.content_hash(), at, max_edge)?;
        self.cache.put(key, &thumbnail.encode())
    }

    /// Cache hit without decoding; otherwise decode once and publish. The
    /// generation closure is only called on a miss or a corrupt entry.
    pub fn get_or_generate(
        &mut self,
        asset: &SourceAsset,
        at: Tick,
        max_edge: u32,
        generate: impl FnOnce() -> media::Result<Thumbnail>,
    ) -> Result<Thumbnail> {
        if let Some(thumbnail) = self.get(asset, at, max_edge)? {
            return Ok(thumbnail);
        }
        let thumbnail = generate()?;
        self.put(asset, at, max_edge, &thumbnail)?;
        Ok(thumbnail)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    use media::{AssetData, AssetId, SourceProbe, SourceStream, StreamKind, TimeBase, VideoInfo};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    fn sample_asset(id: u64, hash: u8) -> SourceAsset {
        SourceAsset::try_from(AssetData {
            id: AssetId(id),
            path: std::path::PathBuf::from("C:/media/fixture.mp4"),
            content_hash: ContentHash([hash; 32]),
            probe: SourceProbe {
                format: "mov,mp4".into(),
                duration: Some(Tick(1_000)),
                streams: vec![SourceStream {
                    index: 0,
                    kind: StreamKind::Video,
                    codec: "h264".into(),
                    time_base: TimeBase::new(1, 1_000).unwrap(),
                    duration: Some(Tick(1_000)),
                    start: None,
                    frame_rate: Some(time::FrameRate::FPS_30),
                    frame_count: None,
                    variable_frame_rate: None,
                    video: Some(VideoInfo {
                        width: 1920,
                        height: 1080,
                        color: media::ColorInfo::default(),
                        bit_depth: Some(8),
                    }),
                    audio: None,
                }],
            },
        })
        .unwrap()
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
        assert_eq!(first, thumbnail());
        let second = cache
            .get_or_generate(&asset, Tick::ZERO, 320, || generate(decodes.clone()))
            .unwrap();
        assert_eq!(second, thumbnail());
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
        assert_eq!(recovered, thumbnail());
        assert_eq!(decodes.load(Ordering::SeqCst), 1);
    }
}

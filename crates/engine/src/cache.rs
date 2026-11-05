//! Content-addressed artifacts and independently committed ASR chunk checkpoints.
mod native;
use crate::{ByteBudget, ByteLease, ByteOwner, EditError};
pub use native::ModelLease;
pub(crate) use native::{NativeEdit, strong_etag};
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArtifactKind {
    SpeechAudio,
    AsrChunk,
    Transcript,
    Translation,
    Cues,
    Preview,
    ModelBundle,
    ShotAnalysis,
}
/// Canonical semantic inputs, in addition to source content and adapter identity.
/// Hash normalized owner serialization, never Debug/display strings or file paths.
pub struct KeyInputs<'a> {
    pub kind: ArtifactKind,
    pub source: [u8; 32],
    pub provider: &'a str,
    pub model: &'a str,
    pub version: &'a str,
    pub settings: &'a [u8],
    pub transcript: [u8; 32],
    pub cues: [u8; 32],
    pub clip_range: [u8; 32],
    pub translation: [u8; 32],
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct CacheKey([u8; 32]);
impl CacheKey {
    pub fn new(inputs: KeyInputs<'_>) -> Result<Self, EditError> {
        if [inputs.provider, inputs.model, inputs.version]
            .iter()
            .any(|s| s.is_empty() || s.len() > 256)
            || inputs.settings.len() > 1024 * 1024
        {
            return Err(EditError::Limit);
        }
        let kind: &[u8] = match inputs.kind {
            ArtifactKind::SpeechAudio => b"speech-audio",
            ArtifactKind::AsrChunk => b"asr-chunk",
            ArtifactKind::Transcript => b"transcript",
            ArtifactKind::Translation => b"translation",
            ArtifactKind::Cues => b"cues",
            ArtifactKind::Preview => b"preview",
            ArtifactKind::ModelBundle => b"model-bundle",
            ArtifactKind::ShotAnalysis => b"shot-analysis",
        };
        let mut hash = blake3::Hasher::new();
        hash.update(b"vibecut-artifact-v1");
        for field in [
            kind,
            &inputs.source,
            inputs.provider.as_bytes(),
            inputs.model.as_bytes(),
            inputs.version.as_bytes(),
            inputs.settings,
            &inputs.transcript,
            &inputs.cues,
            &inputs.clip_range,
            &inputs.translation,
        ] {
            hash.update(&(field.len() as u64).to_le_bytes());
            hash.update(field);
        }
        Ok(Self(*hash.finalize().as_bytes()))
    }
    pub fn bytes(self) -> [u8; 32] {
        self.0
    }
    pub fn filename(self) -> String {
        blake3::Hash::from(self.0).to_hex().to_string()
    }
}
const HEADER: usize = 72;
const EVICTING: usize = usize::MAX;
const REMOVED: usize = usize::MAX - 1;
/// An engine-owned persistent artifact registered weakly with the aggregate
/// budget. Atomic pins serialize readers against reclaimers without an IO lock.
pub(crate) struct StoredArtifact {
    path: PathBuf,
    native: bool,
    charge: Mutex<Option<ByteLease>>,
    len: u64,
    pins: AtomicUsize,
    stamp: AtomicU64,
}
impl StoredArtifact {
    pub(crate) fn removed(&self) -> bool {
        self.pins.load(Ordering::Acquire) == REMOVED
    }
    pub(crate) fn stamp(&self) -> u64 {
        self.stamp.load(Ordering::Relaxed)
    }
    fn pinned(&self) -> bool {
        self.native || self.pins.load(Ordering::Acquire) != 0
    }
    fn pin(self: &Arc<Self>) -> Result<Option<ArtifactPin>, EditError> {
        let mut pins = self.pins.load(Ordering::Acquire);
        loop {
            if pins == REMOVED {
                return Ok(None);
            }
            if pins >= REMOVED - 1 {
                return Err(EditError::BudgetFull);
            }
            match self.pins.compare_exchange_weak(
                pins,
                pins + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Ok(Some(ArtifactPin(self.clone()))),
                Err(current) => pins = current,
            }
        }
    }
    /// Only the successful 0 -> EVICTING owner performs IO and releases accounting.
    /// A concurrent read/reclaim sees temporary pressure, never a missing live pin.
    pub(crate) fn evict(&self) -> Result<bool, EditError> {
        self.evict_inner(false)
    }
    fn evict_inner(&self, explicit: bool) -> Result<bool, EditError> {
        if self.native && !explicit {
            return Ok(false);
        }
        if self
            .pins
            .compare_exchange(0, EVICTING, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Ok(false);
        }
        let removed = if self.native {
            fs::remove_dir_all(&self.path)
        } else {
            fs::remove_file(&self.path)
        };
        match removed {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                self.pins.store(0, Ordering::Release);
                return Err(e.into());
            }
        }
        let charge = {
            self.charge
                .lock()
                .map_err(|_| EditError::JobPoisoned)?
                .take()
        };
        self.pins.store(REMOVED, Ordering::Release);
        drop(charge);
        Ok(true)
    }
}
struct ArtifactPin(Arc<StoredArtifact>);
impl Drop for ArtifactPin {
    fn drop(&mut self) {
        self.0.pins.fetch_sub(1, Ordering::AcqRel);
    }
}
/// Single writer, exclusively owned directory. Methods serialize IO through &mut self.
/// Files are untrusted, size capped before reads and digest checked before reuse.
pub struct ArtifactCache {
    root: PathBuf,
    root_lock: Arc<fs::File>,
    budget: ByteBudget,
    entries: BTreeMap<CacheKey, Arc<StoredArtifact>>,
    max_entries: usize,
}
/// Explicit pin: eviction/clear skips this entry until all leases are dropped.
/// Bytes remain charged separately from disk for the entire reader lifetime.
pub struct ArtifactLease {
    bytes: Vec<u8>,
    _pin: ArtifactPin,
    _charge: ByteLease,
}
impl ArtifactLease {
    pub fn bytes(&self) -> &[u8] {
        self.bytes.get(HEADER..).unwrap_or_default()
    }
}
impl ArtifactCache {
    pub(crate) fn byte_budget(&self) -> ByteBudget {
        self.budget.clone()
    }
    pub(crate) fn root_path(&self) -> &Path {
        &self.root
    }

    /// Rebuild bounded metadata after a crash. Excess entries are removed, and files
    /// with malformed names/header are skipped with a diagnostic count. Payloads
    /// are verified on get. LRU order restarts at directory scan.
    pub fn open(
        root: &Path,
        budget: ByteBudget,
        max_entries: usize,
    ) -> Result<(Self, usize), EditError> {
        if !(1..=100_000).contains(&max_entries) {
            return Err(EditError::Limit);
        }
        fs::create_dir_all(root)?;
        if !fs::symlink_metadata(root)?.file_type().is_dir() {
            return Err(EditError::CacheCorrupt);
        }
        let root = fs::canonicalize(root)?;
        let lock_path = root.join(".cache.lock");
        if fs::symlink_metadata(&lock_path).is_ok_and(|m| !m.file_type().is_file()) {
            return Err(EditError::CacheCorrupt);
        }
        let mut options = fs::File::options();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let root_lock = Arc::new(options.open(lock_path)?);
        root_lock.try_lock().map_err(|e| match e {
            fs::TryLockError::WouldBlock => EditError::CacheBusy,
            fs::TryLockError::Error(e) => EditError::Io(e),
        })?;
        let mut cache = Self {
            root: root.clone(),
            root_lock,
            budget,
            entries: BTreeMap::new(),
            max_entries,
        };
        let mut corrupt = 0usize;
        for file in fs::read_dir(&root)? {
            let file = file?;
            let kind = file.file_type()?;
            if kind.is_dir() {
                match cache.open_native(&file.path()) {
                    Ok(true) => {}
                    Ok(false) => {
                        corrupt = corrupt.saturating_add(1);
                    }
                    Err(e) => return Err(e),
                }
                continue;
            }
            if kind.is_symlink() || !kind.is_file() {
                corrupt = corrupt.saturating_add(1);
                continue;
            }
            let Some(name) = file.file_name().to_str().map(str::to_owned) else {
                corrupt = corrupt.saturating_add(1);
                continue;
            };
            if name == ".cache.lock" {
                continue;
            }
            // Atomic writer debris is never considered a checkpoint.
            if name.starts_with(".vibecut-") {
                fs::remove_file(file.path())?;
                continue;
            }
            let Ok(hash) = blake3::Hash::from_hex(&name) else {
                corrupt = corrupt.saturating_add(1);
                continue;
            };
            let key = CacheKey(*hash.as_bytes());
            let len = file.metadata()?.len();
            let mut header = [0u8; HEADER];
            if name != key.filename()
                || len < HEADER as u64
                || fs::File::open(file.path())?
                    .read_exact(&mut header)
                    .is_err()
                || header.get(..8) != Some(b"VCACHE01")
                || header.get(8..40) != Some(key.0.as_slice())
            {
                corrupt = corrupt.saturating_add(1);
                fs::remove_file(file.path())?;
                continue;
            }
            cache.prune();
            while cache.entries.len() >= max_entries {
                cache.evict_one()?;
            }
            let charge = match cache.budget.reserve(ByteOwner::PersistentCache, len) {
                Ok(charge) => charge,
                Err(EditError::ArtifactTooLarge | EditError::BudgetFull) => {
                    fs::remove_file(file.path())?;
                    continue;
                }
                Err(e) => return Err(e),
            };
            cache.insert(key, charge, len)?;
        }
        Ok((cache, corrupt))
    }
    pub fn len(&self) -> usize {
        self.entries
            .values()
            .filter(|entry| !entry.removed())
            .count()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    fn prune(&mut self) {
        self.entries.retain(|_, entry| !entry.removed());
    }
    fn insert(&mut self, key: CacheKey, charge: ByteLease, len: u64) -> Result<(), EditError> {
        self.prune();
        let artifact = Arc::new(StoredArtifact {
            path: self.path(key),
            native: false,
            charge: Mutex::new(Some(charge)),
            len,
            pins: AtomicUsize::new(0),
            stamp: AtomicU64::new(self.budget.stamp()),
        });
        self.budget.register(&artifact)?;
        self.entries.insert(key, artifact);
        Ok(())
    }
    fn path(&self, key: CacheKey) -> PathBuf {
        self.root.join(key.filename())
    }
    fn remove(&mut self, key: CacheKey) -> Result<(), EditError> {
        if let Some(entry) = self.entries.get(&key)
            && !entry.removed()
            && !entry.evict()?
        {
            return Err(EditError::BudgetFull);
        }
        self.entries.remove(&key);
        Ok(())
    }
    fn evict_one(&mut self) -> Result<(), EditError> {
        self.prune();
        let key = self
            .entries
            .iter()
            .filter(|(_, e)| !e.pinned())
            .min_by_key(|(_, e)| e.stamp())
            .map(|(key, _)| *key)
            .ok_or(EditError::BudgetFull)?;
        self.remove(key)
    }
    /// Commit only a complete artifact. Jobs call this only after successful finish;
    /// ASR may commit each complete chunk separately, keyed by its exact source range.
    pub fn put(&mut self, key: CacheKey, bytes: &[u8]) -> Result<(), EditError> {
        let len = bytes
            .len()
            .checked_add(HEADER)
            .ok_or(EditError::ArtifactTooLarge)? as u64;
        // Charge staging memory before building the envelope. Admission may evict disk.
        if len > self.budget.snapshot()?.limit / 2 {
            return Err(EditError::ArtifactTooLarge);
        }
        self.prune();
        if let Some(entry) = self.entries.get(&key) {
            if entry.pinned() {
                return Err(EditError::BudgetFull);
            }
            self.remove(key)?;
        }
        while self.entries.len() >= self.max_entries {
            self.evict_one()?;
        }
        let disk = self.budget.reserve(ByteOwner::PersistentCache, len)?;
        let _staging = self.budget.reserve(ByteOwner::CacheReads, len)?;
        let mut envelope =
            Vec::with_capacity(usize::try_from(len).map_err(|_| EditError::ArtifactTooLarge)?);
        envelope.extend_from_slice(b"VCACHE01");
        envelope.extend_from_slice(&key.0);
        envelope.extend_from_slice(blake3::hash(bytes).as_bytes());
        envelope.extend_from_slice(bytes);
        project::atomic_write(&self.path(key), &envelope)?;
        self.insert(key, disk, len)
    }
    /// Corrupt/missing entries are never hits. Removal of corrupt files waits
    /// for any earlier reader pins; callers receive CacheCorrupt on each failed read.
    pub fn get(&mut self, key: CacheKey) -> Result<Option<ArtifactLease>, EditError> {
        self.prune();
        let Some(entry) = self.entries.get(&key) else {
            return Ok(None);
        };
        if entry.native {
            return Err(EditError::Unsupported("native bundle requires model lease"));
        }
        let Some(pin) = entry.pin()? else {
            self.entries.remove(&key);
            return Ok(None);
        };
        let expected_len = entry.len;
        let path = self.path(key);
        let result = (|| {
            let meta = fs::symlink_metadata(&path)?;
            if !meta.is_file() || meta.file_type().is_symlink() || meta.len() != expected_len {
                return Err(EditError::CacheCorrupt);
            }
            let _charge = self.budget.reserve(ByteOwner::CacheReads, meta.len())?;
            let len = usize::try_from(meta.len()).map_err(|_| EditError::ArtifactTooLarge)?;
            let mut bytes = vec![0; len];
            let mut file = fs::File::open(&path)?;
            if let Err(e) = file.read_exact(&mut bytes) {
                if e.kind() == std::io::ErrorKind::UnexpectedEof {
                    return Err(EditError::CacheCorrupt);
                }
                return Err(e.into());
            }
            let mut extra = [0u8; 1];
            if file.read(&mut extra)? != 0 {
                return Err(EditError::CacheCorrupt);
            }
            if bytes.len() as u64 != meta.len()
                || bytes.get(..8) != Some(b"VCACHE01")
                || bytes.get(8..40) != Some(key.0.as_slice())
                || bytes.get(40..HEADER)
                    != bytes
                        .get(HEADER..)
                        .map(blake3::hash)
                        .as_ref()
                        .map(|h| h.as_bytes().as_slice())
            {
                return Err(EditError::CacheCorrupt);
            }
            // Keep the envelope allocation: no second payload copy.
            Ok(ArtifactLease {
                bytes,
                _pin: pin,
                _charge,
            })
        })();
        match result {
            Ok(lease) => {
                if let Some(e) = self.entries.get(&key) {
                    e.stamp.store(self.budget.stamp(), Ordering::Relaxed);
                }
                Ok(Some(lease))
            }
            Err(EditError::CacheCorrupt) => {
                // Existing reader pins retain their immutable bytes. Reclaim the
                // corrupt file after their release; preserve the corruption diagnostic.
                match self.remove(key) {
                    Ok(()) | Err(EditError::BudgetFull) => {}
                    Err(e) => return Err(e),
                }
                Err(EditError::CacheCorrupt)
            }
            Err(EditError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {
                self.entries.remove(&key);
                Err(EditError::CacheCorrupt)
            }
            Err(e) => Err(e),
        }
    }
    /// Remove unpinned entries; return how many are still explicitly in use.
    pub fn clear(&mut self) -> Result<usize, EditError> {
        self.prune();
        let keys: Vec<_> = self
            .entries
            .iter()
            .filter(|(_, e)| !e.pinned())
            .map(|(k, _)| *k)
            .collect();
        for key in keys {
            self.remove(key)?;
        }
        Ok(self.entries.len())
    }
}

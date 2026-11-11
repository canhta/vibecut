//! Retains content-addressed execution artifacts.
mod native;
use super::{Error as RuntimeError, Result as RuntimeResult};
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
    Thumbnail,
    ModelBundle,
    ShotAnalysis,
    SceneDetection,
    SpeechActivity,
    Waveform,
    PauseCandidates,
    Chapters,
    Fillers,
    SoundAudio,
    FontDownload,
    Loudness,
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
            ArtifactKind::Thumbnail => b"thumbnail",
            ArtifactKind::ModelBundle => b"model-bundle",
            ArtifactKind::ShotAnalysis => b"shot-analysis",
            ArtifactKind::SceneDetection => b"scene-detection",
            ArtifactKind::SpeechActivity => b"speech-activity",
            ArtifactKind::Waveform => b"waveform",
            ArtifactKind::PauseCandidates => b"pause-candidates",
            ArtifactKind::Chapters => b"chapters",
            ArtifactKind::Fillers => b"fillers",
            ArtifactKind::SoundAudio => b"sound-audio",
            ArtifactKind::FontDownload => b"font-download",
            ArtifactKind::Loudness => b"loudness",
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
    fn pin(self: &Arc<Self>) -> RuntimeResult<Option<ArtifactPin>> {
        let mut pins = self.pins.load(Ordering::Acquire);
        loop {
            if pins == REMOVED {
                return Ok(None);
            }
            if pins >= REMOVED - 1 {
                return Err(RuntimeError::BudgetFull);
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
    pub(crate) fn evict(&self) -> RuntimeResult<bool> {
        self.evict_inner(false)
    }
    fn evict_inner(&self, explicit: bool) -> RuntimeResult<bool> {
        self.evict_with(explicit, || {
            if self.native {
                fs::remove_dir_all(&self.path)
            } else {
                fs::remove_file(&self.path)
            }
        })
    }
    fn evict_with(
        &self,
        explicit: bool,
        unlink: impl FnOnce() -> std::io::Result<()>,
    ) -> RuntimeResult<bool> {
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
        match unlink() {
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
                .map_err(|_| RuntimeError::Poisoned)?
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
    root_lock: Arc<CacheLock>,
    budget: ByteBudget,
    entries: BTreeMap<CacheKey, Arc<StoredArtifact>>,
    max_entries: usize,
}

/// Shared by the writer and native model leases; the final logical owner unlocks.
struct CacheLock(fs::File, fs::Metadata);
impl Drop for CacheLock {
    fn drop(&mut self) {
        // Closing alone can retain flock through a concurrently spawned child's
        // inherited descriptor. Match the lock lifetime to our logical owners.
        let _ = self.0.unlock();
    }
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
    fn check_root(&self) -> Result<(), EditError> {
        check_root(&self.root, &self.root_lock)
    }
    /// Capture registered ordinary artifacts without IO or reader pins. The
    /// desktop's two stores admit at most 256 entries in total. Larger snapshots
    /// refuse rather than truncate a disk-size claim.
    pub fn maintenance(&self) -> Result<CacheMaintenance, EditError> {
        let entries: Vec<_> = self
            .entries
            .iter()
            .filter(|(_, entry)| !entry.native && !entry.removed())
            .take(257)
            .map(|(key, entry)| (*key, entry.clone()))
            .collect();
        if entries.len() > 256 {
            return Err(EditError::Limit);
        }
        Ok(CacheMaintenance {
            root: self.root.clone(),
            root_lock: self.root_lock.clone(),
            entries,
        })
    }
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
            return Err(EditError::Runtime(crate::RuntimeError::CacheCorrupt));
        }
        let root = fs::canonicalize(root)?;
        let lock_path = root.join(".cache.lock");
        if fs::symlink_metadata(&lock_path).is_ok_and(|m| !m.file_type().is_file()) {
            return Err(EditError::Runtime(crate::RuntimeError::CacheCorrupt));
        }
        let mut options = fs::File::options();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let root_lock = Arc::new(CacheLock(options.open(lock_path)?, fs::metadata(&root)?));
        root_lock.0.try_lock().map_err(|e| match e {
            fs::TryLockError::WouldBlock => EditError::Runtime(crate::RuntimeError::CacheBusy),
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
                Err(RuntimeError::ArtifactTooLarge | RuntimeError::BudgetFull) => {
                    fs::remove_file(file.path())?;
                    continue;
                }
                Err(e) => return Err(e.into()),
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
            return Err(EditError::Runtime(RuntimeError::BudgetFull));
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
            .ok_or(EditError::Runtime(RuntimeError::BudgetFull))?;
        self.remove(key)
    }
    /// Commit only a complete artifact. Jobs call this only after successful finish;
    /// ASR may commit each complete chunk separately, keyed by its exact source range.
    pub fn put(&mut self, key: CacheKey, bytes: &[u8]) -> Result<(), EditError> {
        self.check_root()?;
        let len = bytes
            .len()
            .checked_add(HEADER)
            .ok_or(EditError::Runtime(RuntimeError::ArtifactTooLarge))? as u64;
        // Charge staging memory before building the envelope. Admission may evict disk.
        if len > self.budget.snapshot()?.limit / 2 {
            return Err(EditError::Runtime(RuntimeError::ArtifactTooLarge));
        }
        self.prune();
        if let Some(entry) = self.entries.get(&key) {
            if entry.pinned() {
                return Err(EditError::Runtime(RuntimeError::BudgetFull));
            }
            self.remove(key)?;
        }
        while self.entries.len() >= self.max_entries {
            self.evict_one()?;
        }
        let disk = self.budget.reserve(ByteOwner::PersistentCache, len)?;
        let _staging = self.budget.reserve(ByteOwner::CacheReads, len)?;
        let mut envelope = Vec::with_capacity(
            usize::try_from(len).map_err(|_| EditError::Runtime(RuntimeError::ArtifactTooLarge))?,
        );
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
        self.check_root()?;
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
                return Err(EditError::Runtime(crate::RuntimeError::CacheCorrupt));
            }
            let _charge = self.budget.reserve(ByteOwner::CacheReads, meta.len())?;
            let len = usize::try_from(meta.len())
                .map_err(|_| EditError::Runtime(RuntimeError::ArtifactTooLarge))?;
            let mut bytes = vec![0; len];
            let mut file = fs::File::open(&path)?;
            if let Err(e) = file.read_exact(&mut bytes) {
                if e.kind() == std::io::ErrorKind::UnexpectedEof {
                    return Err(EditError::Runtime(crate::RuntimeError::CacheCorrupt));
                }
                return Err(e.into());
            }
            let mut extra = [0u8; 1];
            if file.read(&mut extra)? != 0 {
                return Err(EditError::Runtime(crate::RuntimeError::CacheCorrupt));
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
                return Err(EditError::Runtime(crate::RuntimeError::CacheCorrupt));
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
            Err(EditError::Runtime(crate::RuntimeError::CacheCorrupt)) => {
                // Existing reader pins retain their immutable bytes. Reclaim the
                // corrupt file after their release; preserve the corruption diagnostic.
                match self.remove(key) {
                    Ok(()) | Err(EditError::Runtime(RuntimeError::BudgetFull)) => {}
                    Err(e) => return Err(e),
                }
                Err(EditError::Runtime(crate::RuntimeError::CacheCorrupt))
            }
            Err(EditError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {
                // Retire disk accounting through the same eviction state; a
                // reader lease must retain the missing entry until release.
                match self.remove(key) {
                    Ok(()) | Err(EditError::Runtime(RuntimeError::BudgetFull)) => {}
                    Err(error) => return Err(error),
                }
                Err(EditError::Runtime(crate::RuntimeError::CacheCorrupt))
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

/// Human desktop maintenance; no path or project edit is accepted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CacheOperation {
    Inspect,
    Clear,
}
/// Snapshot observations in physical file bytes (including the 72-byte envelope).
/// Counts describe captured entries only; concurrent generation may repopulate.
#[derive(Default, Debug)]
pub struct CacheMaintenanceReport {
    pub disk_bytes: u64,
    pub in_use_bytes: u64,
    pub removed_bytes: u64,
    pub entries: u64,
    pub in_use: u64,
    pub missing: u64,
    pub failed: u64,
    pub root_unavailable: bool,
    pub cause: Option<EditError>,
}
impl CacheMaintenanceReport {
    /// Combine the desktop's two disjoint snapshots without hiding partial work.
    pub fn merge(&mut self, other: Self) {
        for (a, b) in [
            (&mut self.disk_bytes, other.disk_bytes),
            (&mut self.in_use_bytes, other.in_use_bytes),
            (&mut self.removed_bytes, other.removed_bytes),
            (&mut self.entries, other.entries),
            (&mut self.in_use, other.in_use),
            (&mut self.missing, other.missing),
            (&mut self.failed, other.failed),
        ] {
            if let Some(sum) = a.checked_add(b) {
                *a = sum;
            } else {
                self.cause.get_or_insert(EditError::Limit);
            }
        }
        self.root_unavailable |= other.root_unavailable;
        if self.cause.is_none() {
            self.cause = other.cause;
        }
    }
    fn fail(&mut self, error: EditError) {
        self.failed += 1; // snapshots are bounded to 256
        self.cause.get_or_insert(error);
    }
}
/// Retains registered entries and the directory's exclusive lock through IO.
/// It is not a reader pin or a second cache/budget.
pub struct CacheMaintenance {
    root: PathBuf,
    root_lock: Arc<CacheLock>,
    entries: Vec<(CacheKey, Arc<StoredArtifact>)>,
}
fn corrupt() -> EditError {
    RuntimeError::CacheCorrupt.into()
}
fn same_file(a: &fs::Metadata, b: &fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        a.dev() == b.dev() && a.ino() == b.ino()
    }
    #[cfg(not(unix))]
    {
        // Best-effort settled replacement detection on platforms without std
        // file identity. This is not protection against hostile ancestor races.
        a.created()
            .ok()
            .zip(b.created().ok())
            .is_some_and(|(a, b)| a == b)
    }
}
fn check_root(root: &Path, lock: &CacheLock) -> Result<(), EditError> {
    let valid = || -> std::io::Result<bool> {
        let directory = fs::symlink_metadata(root)?;
        let named_lock = fs::symlink_metadata(root.join(".cache.lock"))?;
        Ok(directory.file_type().is_dir()
            && same_file(&directory, &lock.1)
            && named_lock.file_type().is_file()
            && same_file(&named_lock, &lock.0.metadata()?))
    };
    match valid() {
        Ok(true) => Ok(()),
        Ok(false) => Err(corrupt()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Err(corrupt()),
        Err(error) => Err(error.into()),
    }
}
impl CacheMaintenance {
    pub(crate) fn entry_count(&self) -> usize {
        self.entries.len()
    }
    /// Inspect headers/metadata only; clear uses the existing atomic pin/evict
    /// state. Cancellation stops further unlinks, preserving prior removals and
    /// a typed cause. No cache/budget lock is held during IO. Root checks detect
    /// settled replacement, not concurrent hostile ancestor replacement.
    pub fn run(
        self,
        operation: CacheOperation,
        cancelled: impl Fn() -> bool,
    ) -> CacheMaintenanceReport {
        self.run_using(operation, cancelled, StoredArtifact::evict)
    }
    fn run_using(
        self,
        operation: CacheOperation,
        cancelled: impl Fn() -> bool,
        mut evict: impl FnMut(&StoredArtifact) -> RuntimeResult<bool>,
    ) -> CacheMaintenanceReport {
        let mut report = CacheMaintenanceReport::default();
        let mut stopped = false;
        for (key, entry) in &self.entries {
            // Revalidate for every action, including snapshots that have gone
            // stale while another editor was generating or reclaiming.
            if let Err(error) = check_root(&self.root, &self.root_lock) {
                report.root_unavailable =
                    matches!(error, EditError::Runtime(RuntimeError::CacheCorrupt));
                report.fail(error);
                return report;
            }
            if entry.removed() {
                continue;
            }
            if !stopped && cancelled() {
                report
                    .cause
                    .get_or_insert(EditError::Runtime(crate::RuntimeError::Cancelled));
                stopped = true;
            }
            let measurement = (|| -> Result<Option<u64>, EditError> {
                let meta = match fs::symlink_metadata(&entry.path) {
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                    result => result?,
                };
                if !meta.file_type().is_file() || meta.len() != entry.len {
                    return Err(corrupt());
                }
                let mut header = [0; HEADER];
                let mut file = fs::File::open(&entry.path)?;
                if let Err(error) = file.read_exact(&mut header) {
                    return Err(if error.kind() == std::io::ErrorKind::UnexpectedEof {
                        corrupt()
                    } else {
                        error.into()
                    });
                }
                if !same_file(&meta, &file.metadata()?)
                    || header.get(..8) != Some(b"VCACHE01")
                    || header.get(8..40) != Some(key.0.as_slice())
                {
                    return Err(corrupt());
                }
                Ok(Some(meta.len()))
            })();
            let len = match measurement {
                Ok(Some(len)) => len,
                Ok(None) => {
                    report.missing += 1;
                    report.cause.get_or_insert_with(corrupt);
                    // Missing files have zero physical bytes. Pins keep their
                    // disk charge until a later pass after release.
                    if !stopped && let Err(error) = evict(entry) {
                        report.fail(error.into());
                    }
                    if entry.pinned() && !entry.removed() {
                        report.in_use += 1;
                        report.in_use_bytes += entry.len;
                    }
                    continue;
                }
                Err(error) => {
                    report.entries += 1;
                    report.fail(error);
                    continue;
                }
            };
            if operation == CacheOperation::Clear && !stopped {
                match evict(entry) {
                    Ok(true) => {
                        report.removed_bytes += len;
                        continue;
                    }
                    Ok(false) => {}
                    Err(error) => report.fail(error.into()),
                }
            }
            report.entries += 1;
            report.disk_bytes += len;
            if entry.pinned() && !entry.removed() {
                report.in_use += 1;
                report.in_use_bytes += len;
            }
        }
        // Empty/deleted stores must not masquerade as a successful zero.
        if let Err(error) = check_root(&self.root, &self.root_lock) {
            report.root_unavailable =
                matches!(error, EditError::Runtime(RuntimeError::CacheCorrupt));
            report.fail(error);
        }
        report
    }
}

#[cfg(all(test, unix))]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::{ArtifactCache, ByteBudget, EditError};

    #[test]
    fn cache_maintenance_partial_unlink_failure_retains_charge_and_retries() {
        use super::{CacheOperation, RuntimeError};
        let dir = tempfile::tempdir().unwrap();
        let budget = ByteBudget::new(8192).unwrap();
        let (mut cache, _) = ArtifactCache::open(dir.path(), budget.clone(), 8).unwrap();
        for n in 1..=3 {
            let key = super::CacheKey::new(super::KeyInputs {
                kind: super::ArtifactKind::Waveform,
                source: [n; 32],
                provider: "test",
                model: "test",
                version: "1",
                settings: &[],
                transcript: [0; 32],
                cues: [0; 32],
                clip_range: [0; 32],
                translation: [0; 32],
            })
            .unwrap();
            cache.put(key, &[n; 28]).unwrap();
        }
        let model = ai::models::catalog().unwrap().remove(0);
        let native = cache.edit_model(&model).unwrap();
        let marker = native.directory.join(".bundle");
        let model_bytes = std::fs::read(&marker).unwrap();
        drop(native);
        let native_charge = budget.snapshot().unwrap().used - 300;
        let mut calls = 0;
        let report = cache.maintenance().unwrap().run_using(
            CacheOperation::Clear,
            || false,
            |entry| {
                calls += 1;
                if calls == 2 {
                    entry.evict_with(false, || {
                        Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied))
                    })
                } else {
                    entry.evict()
                }
            },
        );
        assert_eq!(
            (
                report.removed_bytes,
                report.disk_bytes,
                report.failed,
                report.entries
            ),
            (200, 100, 1, 1)
        );
        assert!(
            matches!(report.cause, Some(EditError::Runtime(RuntimeError::Io(ref error))) if error.kind() == std::io::ErrorKind::PermissionDenied)
        );
        assert_eq!(budget.snapshot().unwrap().used, native_charge + 100);
        let retry = cache
            .maintenance()
            .unwrap()
            .run(CacheOperation::Clear, || false);
        assert_eq!((retry.removed_bytes, retry.disk_bytes), (100, 0));
        assert!(retry.cause.is_none());
        assert_eq!(budget.snapshot().unwrap().used, native_charge);
        assert_eq!(std::fs::read(marker).unwrap(), model_bytes);
    }

    #[test]
    fn cache_reopen_waits_for_last_owner_but_not_retained_descriptors() {
        let root = tempfile::tempdir().unwrap();
        let budget = ByteBudget::new(1024 * 1024).unwrap();
        let (cache, _) = ArtifactCache::open(root.path(), budget.clone(), 32).unwrap();
        let owner = cache.root_lock.clone();
        // A duplicated open file description pins the same flock, just as a
        // concurrent fork can before its child execs or closes the descriptor.
        let retained = owner.0.try_clone().unwrap();
        drop(cache);
        assert!(matches!(
            ArtifactCache::open(root.path(), budget.clone(), 32),
            Err(EditError::Runtime(crate::RuntimeError::CacheBusy))
        ));
        drop(owner);
        let (reopened, _) = ArtifactCache::open(root.path(), budget.clone(), 32).unwrap();
        drop(retained);
        assert!(matches!(
            ArtifactCache::open(root.path(), budget.clone(), 32),
            Err(EditError::Runtime(crate::RuntimeError::CacheBusy))
        ));
        drop(reopened);
        assert!(ArtifactCache::open(root.path(), budget, 32).is_ok());
    }
}

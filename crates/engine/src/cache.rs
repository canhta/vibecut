//! Content-addressed artifacts and independently committed ASR chunk checkpoints.
use crate::{ByteBudget, ByteLease, ByteOwner, EditError};
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArtifactKind {
    SpeechAudio,
    AsrChunk,
    Transcript,
    Translation,
    Cues,
    Preview,
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
struct Entry {
    charge: Arc<ByteLease>,
    len: u64,
    pin: Arc<()>,
    stamp: u64,
}
/// Single writer, exclusively owned directory. Methods serialize IO through &mut self.
/// Files are untrusted, size capped before reads and digest checked before reuse.
pub struct ArtifactCache {
    root: PathBuf,
    budget: ByteBudget,
    entries: BTreeMap<CacheKey, Entry>,
    max_entries: usize,
    stamp: u64,
}
/// Explicit pin: eviction/clear skips this entry until all leases are dropped.
/// Bytes remain charged separately from disk for the entire reader lifetime.
pub struct ArtifactLease {
    bytes: Vec<u8>,
    _pin: Arc<()>,
    _disk: Arc<ByteLease>,
    _charge: ByteLease,
}
impl ArtifactLease {
    pub fn bytes(&self) -> &[u8] {
        self.bytes.get(HEADER..).unwrap_or_default()
    }
}
impl ArtifactCache {
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
        let mut cache = Self {
            root: root.to_owned(),
            budget,
            entries: BTreeMap::new(),
            max_entries,
            stamp: 0,
        };
        let mut corrupt = 0usize;
        for file in fs::read_dir(root)? {
            let file = file?;
            let kind = file.file_type()?;
            if kind.is_symlink() || !kind.is_file() {
                corrupt = corrupt.saturating_add(1);
                continue;
            }
            let Some(name) = file.file_name().to_str().map(str::to_owned) else {
                corrupt = corrupt.saturating_add(1);
                continue;
            };
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
            while cache.entries.len() >= max_entries {
                cache.evict_one()?;
            }
            let charge = match cache.make_room(len) {
                Ok(charge) => charge,
                Err(EditError::ArtifactTooLarge | EditError::BudgetFull) => {
                    fs::remove_file(file.path())?;
                    continue;
                }
                Err(e) => return Err(e),
            };
            cache.entries.insert(
                key,
                Entry {
                    charge: Arc::new(charge),
                    len,
                    pin: Arc::new(()),
                    stamp: 0,
                },
            );
        }
        Ok((cache, corrupt))
    }
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    fn path(&self, key: CacheKey) -> PathBuf {
        self.root.join(key.filename())
    }
    fn remove(&mut self, key: CacheKey) -> Result<(), EditError> {
        match fs::remove_file(self.path(key)) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        self.entries.remove(&key);
        Ok(())
    }
    fn evict_one(&mut self) -> Result<(), EditError> {
        let key = self
            .entries
            .iter()
            .filter(|(_, e)| Arc::strong_count(&e.pin) == 1)
            .min_by_key(|(_, e)| e.stamp)
            .map(|(key, _)| *key)
            .ok_or(EditError::BudgetFull)?;
        self.remove(key)
    }
    fn make_room(&mut self, bytes: u64) -> Result<ByteLease, EditError> {
        loop {
            match self.budget.reserve(ByteOwner::PersistentCache, bytes) {
                Err(EditError::BudgetFull) if !self.entries.is_empty() => self.evict_one()?,
                result => return result,
            }
        }
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
        if let Some(entry) = self.entries.get(&key) {
            if Arc::strong_count(&entry.pin) > 1 {
                return Err(EditError::BudgetFull);
            }
            self.remove(key)?;
        }
        while self.entries.len() >= self.max_entries {
            self.evict_one()?;
        }
        let disk = self.make_room(len)?;
        let _staging = loop {
            match self.budget.reserve(ByteOwner::CacheReads, len) {
                Ok(v) => break v,
                Err(EditError::BudgetFull) => self.evict_one()?,
                Err(e) => return Err(e),
            }
        };
        let mut envelope =
            Vec::with_capacity(usize::try_from(len).map_err(|_| EditError::ArtifactTooLarge)?);
        envelope.extend_from_slice(b"VCACHE01");
        envelope.extend_from_slice(&key.0);
        envelope.extend_from_slice(blake3::hash(bytes).as_bytes());
        envelope.extend_from_slice(bytes);
        project::atomic_write(&self.path(key), &envelope)?;
        self.stamp = self.stamp.saturating_add(1);
        self.entries.insert(
            key,
            Entry {
                charge: Arc::new(disk),
                len,
                pin: Arc::new(()),
                stamp: self.stamp,
            },
        );
        Ok(())
    }
    /// Corrupt/missing entries are removed and reported once, never returned as hits.
    pub fn get(&mut self, key: CacheKey) -> Result<Option<ArtifactLease>, EditError> {
        let Some(entry) = self.entries.get(&key) else {
            return Ok(None);
        };
        let pin = entry.pin.clone();
        let disk = entry.charge.clone();
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
                _disk: disk,
                _charge,
            })
        })();
        match result {
            Ok(lease) => {
                self.stamp = self.stamp.saturating_add(1);
                if let Some(e) = self.entries.get_mut(&key) {
                    e.stamp = self.stamp;
                }
                Ok(Some(lease))
            }
            Err(EditError::CacheCorrupt) => {
                self.remove(key)?;
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
        let keys: Vec<_> = self
            .entries
            .iter()
            .filter(|(_, e)| Arc::strong_count(&e.pin) == 1)
            .map(|(k, _)| *k)
            .collect();
        for key in keys {
            self.remove(key)?;
        }
        Ok(self.entries.len())
    }
}

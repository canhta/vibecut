//! Native bundles extend the cache protocol; only a verified marker authorizes a lease.
use super::{
    ArtifactCache, ArtifactKind, ArtifactPin, CacheKey, CacheLock, HEADER, KeyInputs,
    StoredArtifact,
};
use crate::ByteOwner;
use crate::runtime::{Error as RuntimeError, Result};
use ai::foundation::models::Model;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
};
const STATE: &str = ".bundle";
pub(crate) const META_LIMIT: u64 = 65536;
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Part {
    pub offset: u64,
    pub etag: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NativeState {
    pub version: u32,
    pub model: Model,
    pub complete: bool,
    pub parts: Vec<Part>,
}
impl NativeState {
    fn validate(&self, key: CacheKey) -> Result<()> {
        self.model.validate().map_err(RuntimeError::Model)?;
        if self.version != 1
            || self.parts.len() != self.model.files.len()
            || model_key(&self.model)? != key
        {
            return Err(RuntimeError::CacheCorrupt);
        }
        for (part, file) in self.parts.iter().zip(&self.model.files) {
            if part.offset > file.size
                || part.etag.as_ref().is_some_and(|tag| !strong_etag(tag))
                || (self.complete && part.offset != file.size)
            {
                return Err(RuntimeError::CacheCorrupt);
            }
        }
        Ok(())
    }
}
pub(crate) fn strong_etag(value: &str) -> bool {
    value.len() >= 2
        && value.len() <= 1024
        && value.starts_with('"')
        && value.ends_with('"')
        && value
            .as_bytes()
            .get(1..value.len() - 1)
            .is_some_and(|s| s.iter().all(|b| matches!(b, 0x21 | 0x23..=0x7e)))
}
pub fn model_key(model: &Model) -> Result<CacheKey> {
    model.validate().map_err(RuntimeError::Model)?;
    let settings = serde_json::to_vec(model).map_err(|_| RuntimeError::Limit)?;
    if settings.len() as u64 > META_LIMIT / 4 {
        return Err(RuntimeError::Limit);
    }
    CacheKey::new(KeyInputs {
        kind: ArtifactKind::ModelBundle,
        source: [0; 32],
        provider: "local-model",
        model: &model.id,
        version: "1",
        settings: &settings,
        transcript: [0; 32],
        cues: [0; 32],
        clip_range: [0; 32],
        translation: [0; 32],
    })
}
/// Keeps immutable named files and the exclusive cache-root lock alive for native loaders.
/// Explicit removal refuses while any lease exists; ordinary LRU/clear never removes models.
pub struct ModelLease {
    directory: PathBuf,
    model: Model,
    _pin: ArtifactPin,
    _root_lock: Arc<CacheLock>,
}
impl ModelLease {
    pub fn directory(&self) -> &Path {
        &self.directory
    }
    pub fn model(&self) -> &Model {
        &self.model
    }
    pub fn file(&self, name: &str) -> Result<PathBuf> {
        if !self.model.files.iter().any(|file| file.name == name) {
            return Err(RuntimeError::Invalid("unknown model file"));
        }
        Ok(self.directory.join(name))
    }
}
pub(crate) struct NativeEdit {
    pub directory: PathBuf,
    pub state: NativeState,
    _pin: ArtifactPin,
    lengths: Vec<u64>,
    metadata_bytes: u64,
}
impl NativeEdit {
    pub fn checkpoint(&mut self) -> Result<()> {
        self.metadata_bytes = write_state(&self.directory, &self.state)?;
        self.report()
    }
    pub fn report_length(&mut self, index: usize, len: u64) -> Result<()> {
        *self
            .lengths
            .get_mut(index)
            .ok_or(RuntimeError::CacheCorrupt)? = len;
        self.report()
    }
    fn report(&self) -> Result<()> {
        let total = self
            .lengths
            .iter()
            .try_fold(self.metadata_bytes, |n, len| {
                n.checked_add(*len).ok_or(RuntimeError::Limit)
            })?;
        self._pin
            .0
            .charge
            .lock()
            .map_err(|_| crate::RuntimeError::Poisoned)?
            .as_mut()
            .ok_or(RuntimeError::CacheCorrupt)?
            .resize_models(total)
    }
}
fn usage(root: &Path, state: &NativeState) -> Result<(u64, Vec<u64>)> {
    let metadata = fs::symlink_metadata(root.join(STATE))?.len();
    let lengths = state
        .model
        .files
        .iter()
        .map(|file| match fs::symlink_metadata(root.join(&file.name)) {
            Ok(m) if m.file_type().is_file() => Ok(m.len()),
            Ok(_) => Ok(0),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(0),
            Err(e) => Err(RuntimeError::Io(e)),
        })
        .collect::<std::result::Result<Vec<_>, _>>()?;
    Ok((metadata, lengths))
}

fn envelope(key: CacheKey, state: &NativeState) -> Result<Vec<u8>> {
    let body = serde_json::to_vec(state).map_err(|_| RuntimeError::CacheCorrupt)?;
    if body.len() as u64 > META_LIMIT - HEADER as u64 {
        return Err(RuntimeError::Limit);
    }
    let mut bytes = Vec::with_capacity(HEADER + body.len());
    bytes.extend_from_slice(b"VCACHE01");
    bytes.extend_from_slice(&key.0);
    bytes.extend_from_slice(blake3::hash(&body).as_bytes());
    bytes.extend_from_slice(&body);
    Ok(bytes)
}
fn write_state(root: &Path, state: &NativeState) -> Result<u64> {
    let key = model_key(&state.model)?;
    state.validate(key)?;
    let bytes = envelope(key, state)?;
    project::atomic_write(&root.join(STATE), &bytes).map_err(|e| match e {
        project::Error::Io(e) if e.kind() == std::io::ErrorKind::StorageFull => {
            RuntimeError::StorageFull
        }
        other => RuntimeError::Edit(crate::EditError::Project(other)),
    })?;
    Ok(bytes.len() as u64)
}
fn read_state(root: &Path, key: CacheKey) -> Result<NativeState> {
    if !fs::symlink_metadata(root)?.file_type().is_dir() {
        return Err(RuntimeError::CacheCorrupt);
    }
    let path = root.join(STATE);
    let meta = fs::symlink_metadata(&path)?;
    if !meta.file_type().is_file() || meta.len() > META_LIMIT {
        return Err(RuntimeError::CacheCorrupt);
    }
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(META_LIMIT + 1)
        .read_to_end(&mut bytes)?;
    let body = bytes.get(HEADER..).ok_or(RuntimeError::CacheCorrupt)?;
    if bytes.len() as u64 > META_LIMIT
        || bytes.get(..8) != Some(b"VCACHE01")
        || bytes.get(8..40) != Some(key.0.as_slice())
        || bytes.get(40..HEADER) != Some(blake3::hash(body).as_bytes().as_slice())
    {
        return Err(RuntimeError::CacheCorrupt);
    }
    let state: NativeState =
        serde_json::from_slice(body).map_err(|_| RuntimeError::CacheCorrupt)?;
    state.validate(key)?;
    Ok(state)
}
fn reserved_bytes(model: &Model) -> Result<u64> {
    model.files.iter().try_fold(META_LIMIT * 2, |n, f| {
        n.checked_add(f.size).ok_or(RuntimeError::Limit)
    })
}
impl ArtifactCache {
    pub(super) fn open_native(&mut self, path: &Path) -> Result<bool> {
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            return Ok(false);
        };
        let Ok(hash) = blake3::Hash::from_hex(name) else {
            return Ok(false);
        };
        let key = CacheKey(*hash.as_bytes());
        if name != key.filename() {
            return Ok(false);
        }
        // Unknown directories are preserved, never adopted or recursively removed.
        let Ok(state) = read_state(path, key) else {
            return Ok(false);
        };
        if self.entries.len() >= self.max_entries {
            return Err(RuntimeError::Limit);
        }
        let (meta, lengths) = usage(path, &state)?;
        let total = lengths.iter().try_fold(meta, |n, len| {
            n.checked_add(*len).ok_or(RuntimeError::Limit)
        })?;
        self.insert_native(key, total)?;
        Ok(true)
    }
    fn insert_native(&mut self, key: CacheKey, len: u64) -> Result<()> {
        let charge = self.budget.reserve(ByteOwner::ModelWeights, len)?;
        let entry = Arc::new(StoredArtifact {
            path: self.path(key),
            native: true,
            charge: Mutex::new(Some(charge)),
            len,
            pins: AtomicUsize::new(0),
            stamp: AtomicU64::new(self.budget.stamp()),
        });
        self.budget.register(&entry)?;
        self.entries.insert(key, entry);
        Ok(())
    }
    pub fn model_key(model: &Model) -> Result<CacheKey> {
        model_key(model)
    }
    /// Verify a hit through ai's streaming SHA/size owner before exposing raw paths.
    pub fn model(
        &mut self,
        model: &Model,
        hooks: &dyn ai::ExecutionHooks,
    ) -> Result<Option<ModelLease>> {
        let key = model_key(model)?;
        self.prune();
        let Some(entry) = self.entries.get(&key) else {
            return Ok(None);
        };
        if !entry.native {
            return Err(RuntimeError::CacheCorrupt);
        }
        let pin = entry.pin()?.ok_or(RuntimeError::CacheBusy)?;
        let state = read_state(&entry.path, key)?;
        if state.model != *model {
            return Err(RuntimeError::CacheCorrupt);
        }
        if !state.complete {
            return Ok(None);
        }
        for file in &model.files {
            let path = entry.path.join(&file.name);
            let meta = fs::symlink_metadata(&path)?;
            if !meta.file_type().is_file() || meta.len() != file.size {
                return Err(RuntimeError::CacheCorrupt);
            }
            ai::foundation::models::verify_file(&mut fs::File::open(path)?, file, hooks)
                .map_err(RuntimeError::Model)?;
        }
        Ok(Some(ModelLease {
            directory: entry.path.clone(),
            model: model.clone(),
            _pin: pin,
            _root_lock: self.root_lock.clone(),
        }))
    }
    pub(crate) fn remaining_model(&self, model: &Model) -> Result<u64> {
        let key = model_key(model)?;
        if let Some(entry) = self.entries.get(&key) {
            if !entry.native {
                return Err(RuntimeError::CacheCorrupt);
            }
            let state = read_state(&entry.path, key)?;
            state.model.files.iter().zip(&state.parts).try_fold(
                META_LIMIT * 2,
                |sum, (file, part)| {
                    let length = match fs::symlink_metadata(entry.path.join(&file.name)) {
                        Ok(meta) if meta.file_type().is_file() => meta.len(),
                        Ok(_) => {
                            return Err(RuntimeError::CacheCorrupt);
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => 0,
                        Err(e) => return Err(RuntimeError::Io(e)),
                    };
                    if length < part.offset {
                        return Err(RuntimeError::CacheCorrupt);
                    }
                    // A crash can leave bytes beyond the durable checkpoint. They
                    // are reclaimed before resumption, so credit their disk space.
                    sum.checked_add(file.size.saturating_sub(length))
                        .ok_or(RuntimeError::Limit)
                },
            )
        } else {
            reserved_bytes(model)
        }
    }
    pub(crate) fn edit_model(&mut self, model: &Model) -> Result<NativeEdit> {
        let key = model_key(model)?;
        self.prune();
        if !self.entries.contains_key(&key) {
            while self.entries.len() >= self.max_entries {
                self.evict_one()?;
            }
            let directory = self.path(key);
            // Never overwrite/adopt a foreign directory at this content address.
            if directory.try_exists()? {
                return Err(RuntimeError::CacheCorrupt);
            }
            fs::create_dir(&directory)?;
            let state = NativeState {
                version: 1,
                model: model.clone(),
                complete: false,
                parts: model
                    .files
                    .iter()
                    .map(|_| Part {
                        offset: 0,
                        etag: None,
                    })
                    .collect(),
            };
            if let Err(e) = write_state(&directory, &state) {
                let _ = fs::remove_dir(&directory);
                return Err(e);
            }
            self.insert_native(key, fs::symlink_metadata(directory.join(STATE))?.len())?;
        }
        let entry = self.entries.get(&key).ok_or(RuntimeError::CacheCorrupt)?;
        if !entry.native || entry.pins.load(Ordering::Acquire) != 0 {
            return Err(RuntimeError::CacheBusy);
        }
        let pin = entry.pin()?.ok_or(RuntimeError::CacheBusy)?;
        let state = read_state(&entry.path, key)?;
        if state.model != *model || state.complete {
            return Err(RuntimeError::CacheBusy);
        }
        let (metadata_bytes, lengths) = usage(&entry.path, &state)?;
        Ok(NativeEdit {
            directory: entry.path.clone(),
            state,
            _pin: pin,
            metadata_bytes,
            lengths,
        })
    }
    /// Explicit user removal; never called by LRU, generic clear or download fallback.
    pub fn remove_model(&mut self, model: &Model) -> Result<()> {
        let key = model_key(model)?;
        let Some(entry) = self.entries.get(&key) else {
            return Ok(());
        };
        if !entry.native {
            return Err(RuntimeError::CacheCorrupt);
        }
        if !entry.evict_inner(true)? {
            return Err(RuntimeError::CacheBusy);
        }
        self.entries.remove(&key);
        Ok(())
    }
}

//! Publishes durable copied LUT bundles.
use crate::{Error as EngineError, Result, jobs::JobContext};
use project::{LutId, LutRecord, ProjectId};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

const QUOTA: u64 = 256 << 20;
const MAX_BUNDLE: u64 = (24 + 16) * 1024 * 1024 + 16384;
const FILES: [&str; 3] = ["canonical.cube", "original.cube", "provenance.json"];
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Provenance {
    version: u32,
    id: LutId,
    name: captions::NfcText,
    licence: Option<captions::NfcText>,
    source_name: captions::NfcText,
    source_hash: media::ContentHash,
    copied_source_hash: media::ContentHash,
}
impl Provenance {
    fn record(self, directory: PathBuf) -> Result<LutRecord> {
        if self.version != 1 {
            return Err(EngineError::Edit(crate::EditError::Invalid(
                "LUT bundle version",
            )));
        }
        let record = LutRecord {
            id: self.id,
            name: self.name,
            licence: self.licence,
            source_name: self.source_name,
            source_hash: self.source_hash,
            directory,
        };
        record.validate()?;
        Ok(record)
    }
}
struct Quota {
    bytes: u64,
    entries: usize,
    pending: u64,
}
struct Store {
    root: PathBuf,
    quota: Mutex<Quota>,
    lease: project::WriterLease,
}
/// Process-owned durable source assets, disjoint from evictable artifacts.
#[derive(Clone)]
pub struct LutStore(Arc<Store>);
struct Pending {
    store: Arc<Store>,
    bytes: u64,
    committed: bool,
}
impl Drop for Pending {
    fn drop(&mut self) {
        if let Ok(mut state) = self.store.quota.lock() {
            state.pending = state.pending.saturating_sub(MAX_BUNDLE);
            if self.committed {
                state.bytes = state.bytes.saturating_add(self.bytes);
                state.entries = state.entries.saturating_add(1);
            }
        }
    }
}
fn plain(path: &Path) -> Result<fs::Metadata> {
    let meta = fs::symlink_metadata(path)?;
    if meta.file_type().is_symlink() {
        return Err(EngineError::Edit(crate::EditError::Invalid(
            "LUT path is a symlink",
        )));
    }
    Ok(meta)
}
fn parents(path: &Path) -> Result<()> {
    for ancestor in path.ancestors() {
        match fs::symlink_metadata(ancestor) {
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(EngineError::Edit(crate::EditError::Invalid(
                    "LUT parent is a symlink",
                )));
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}
fn usage(root: &Path) -> Result<(u64, usize, usize)> {
    let mut bytes = 0u64;
    let mut count = 0usize;
    let mut bundles = 0usize;
    let mut dirs = vec![(root.to_owned(), 0)];
    while let Some((dir, depth)) = dirs.pop() {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            count += 1;
            if count > 4096 {
                return Err(EngineError::Control(
                    crate::session::wire::ControlError::Limit,
                ));
            }
            let meta = plain(&entry.path())?;
            if meta.is_dir() {
                if depth >= 2 {
                    return Err(EngineError::Edit(crate::EditError::Invalid(
                        "LUT store nesting",
                    )));
                }
                if depth == 1 {
                    bundles += 1;
                }
                dirs.push((entry.path(), depth + 1));
            } else if meta.is_file() {
                bytes = bytes.checked_add(meta.len()).ok_or(EngineError::Control(
                    crate::session::wire::ControlError::Limit,
                ))?;
            } else {
                return Err(EngineError::Edit(crate::EditError::Invalid(
                    "LUT store entry",
                )));
            }
        }
    }
    Ok((bytes, count, bundles))
}
impl LutStore {
    /// Open one exclusive source store. Admission enumerates at most 4096 entries;
    /// foreign entries are retained and counted, never interpreted as disposable.
    pub fn open(root: &Path) -> Result<Self> {
        parents(root)?;
        fs::create_dir_all(root)?;
        parents(root)?;
        let root = fs::canonicalize(root)?;
        let lease = project::WriterLease::acquire(&root.join("store.vibecut"))?;
        let (bytes, _, bundles) = usage(&root)?;
        if bytes > QUOTA || bundles > 1024 {
            return Err(EngineError::Control(
                crate::session::wire::ControlError::Limit,
            ));
        }
        Ok(Self(Arc::new(Store {
            root,
            quota: Mutex::new(Quota {
                bytes,
                entries: bundles,
                pending: 0,
            }),
            lease,
        })))
    }
    fn reserve(&self) -> Result<Pending> {
        let (bytes, entries, bundles) = usage(&self.0.root)?;
        let mut state = self
            .0
            .quota
            .lock()
            .map_err(|_| crate::RuntimeError::Poisoned)?;
        // Include persistent publication lock files, project directories and all
        // staging entries. Concurrent pending work is charged conservatively twice
        // if enumeration already observed it, so reopening never exceeds the cap.
        let pending = state.pending / MAX_BUNDLE;
        if entries.saturating_add((pending as usize + 1) * 10) > 4096
            || state.entries.max(bundles) + pending as usize >= 1024
            || state
                .bytes
                .max(bytes)
                .saturating_add(state.pending)
                .saturating_add(MAX_BUNDLE)
                > QUOTA
        {
            return Err(EngineError::Control(
                crate::session::wire::ControlError::Limit,
            ));
        }
        state.pending += MAX_BUNDLE;
        Ok(Pending {
            store: self.0.clone(),
            bytes: 0,
            committed: false,
        })
    }
    /// Copy/validate first, seal cancellation only at complete-directory publication.
    /// Relink accepts the exact existing record, never an arbitrary output path.
    pub(crate) fn import(
        &self,
        project: ProjectId,
        source: &Path,
        name: Option<captions::NfcText>,
        licence: Option<captions::NfcText>,
        expected: Option<LutRecord>,
        context: &JobContext,
    ) -> Result<LutRecord> {
        context.check_cancelled()?;
        if !source
            .extension()
            .is_some_and(|s| s.eq_ignore_ascii_case("cube"))
        {
            return Err(render::Error::Unsupported("only 3D cube LUTs are supported").into());
        }
        parents(source)?;
        let metadata = plain(source)?;
        if !metadata.is_file() || metadata.len() > render::lut::MAX_CUBE_SOURCE {
            return Err(EngineError::Control(
                crate::session::wire::ControlError::Limit,
            ));
        }
        let source_name = source
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or(EngineError::Edit(crate::EditError::Invalid(
                "LUT source filename",
            )))?;
        if source_name.len() > 256
            || name
                .as_ref()
                .is_some_and(|s| s.as_str().is_empty() || s.as_str().len() > 256)
            || licence.as_ref().is_some_and(|s| s.as_str().len() > 1024)
        {
            return Err(EngineError::Control(
                crate::session::wire::ControlError::Limit,
            ));
        }
        let mut pending = self.reserve()?;
        let parent = self.0.root.join(project.0.to_string());
        parents(&parent)?;
        fs::create_dir_all(&parent)?;
        let stage = super::projects::staging_directory(&parent, "lut-import")?;
        let _stage = super::projects::DirectoryGuard(Some(stage.clone()));
        let original = stage.as_path().join("original.cube");
        let _copy = context
            .memory_budget()
            .reserve(crate::ByteOwner::PersistentCache, 256 << 10)?;
        let mut input = fs::File::open(source)?.take(render::lut::MAX_CUBE_SOURCE + 1);
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&original)?;
        let mut copied = 0u64;
        let mut buffer = [0u8; 65536];
        loop {
            context.check_cancelled()?;
            let n = input.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            copied = copied.checked_add(n as u64).ok_or(EngineError::Control(
                crate::session::wire::ControlError::Limit,
            ))?;
            if copied > metadata.len() {
                return Err(EngineError::Edit(crate::EditError::Invalid(
                    "LUT source changed",
                )));
            }
            output.write_all(buffer.get(..n).ok_or(EngineError::Control(
                crate::session::wire::ControlError::Limit,
            ))?)?;
        }
        output.sync_all()?;
        drop(output);
        if copied != metadata.len()
            || plain(source)?.len() != metadata.len()
            || plain(source)?.modified()? != metadata.modified()?
        {
            return Err(EngineError::Edit(crate::EditError::Invalid(
                "LUT source changed",
            )));
        }
        let (cube, source_hash) =
            render::lut::read_cube(&original, render::lut::MAX_CUBE_SOURCE, context)?;
        let canonical = stage.as_path().join("canonical.cube");
        let file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&canonical)?;
        let mut output = std::io::BufWriter::with_capacity(65536, file);
        cube.write_cube(&mut output, context)?;
        output.flush()?;
        output.get_ref().sync_all()?;
        drop(output);
        drop(cube);
        if plain(&canonical)?.len() > render::lut::MAX_CUBE_CANONICAL {
            return Err(EngineError::Control(
                crate::session::wire::ControlError::Limit,
            ));
        }
        let id = LutId::new(hash_file(
            &canonical,
            render::lut::MAX_CUBE_CANONICAL,
            &|| context.cancelled(),
        )?);
        let destination = parent.join(id.hex());
        let mut record = LutRecord {
            id,
            name: name.clone().unwrap_or_else(|| source_name.into()),
            licence: licence.clone(),
            source_name: source_name.into(),
            source_hash,
            directory: destination.clone(),
        };
        let repairing = expected.is_some();
        if let Some(expected) = expected {
            expected.validate()?;
            if expected.id != id {
                return Err(EngineError::Edit(crate::EditError::Invalid(
                    "LUT relink content differs",
                )));
            }
            // Managed copies keep their original path; only a matching persisted
            // provenance proves that a damaged directory belongs to this record.
            let owned_store = expected.directory == destination;
            if !owned_store && read_record(&expected.directory)? != expected {
                return Err(EngineError::Edit(crate::EditError::Invalid(
                    "LUT relink destination",
                )));
            }
            if name.as_ref().is_some_and(|name| *name != expected.name)
                || licence
                    .as_ref()
                    .is_some_and(|licence| Some(licence) != expected.licence.as_ref())
            {
                return Err(EngineError::Edit(crate::EditError::Invalid(
                    "conflicting LUT provenance",
                )));
            }
            record = expected;
        }
        record.validate()?;
        let provenance = Provenance {
            version: 1,
            id,
            name: record.name.clone(),
            licence: record.licence.clone(),
            source_name: record.source_name.clone(),
            source_hash: record.source_hash,
            copied_source_hash: source_hash,
        };
        let _metadata = context
            .memory_budget()
            .reserve(crate::ByteOwner::PersistentCache, 256 << 10)?;
        let encoded = crate::session::wire::bounded_json(&provenance, 16384)?;
        crate::session::wire::bounded_json(&record, 16384)?;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(stage.as_path().join("provenance.json"))?;
        file.write_all(&encoded)?;
        file.sync_all()?;
        drop(file);
        pending.bytes = copied + plain(&canonical)?.len() + encoded.len() as u64;
        context.check_cancelled()?;
        parents(&record.directory)?;
        let publication = project::WriterLease::acquire_wait(
            &record
                .directory
                .parent()
                .ok_or(EngineError::Edit(crate::EditError::Invalid("LUT parent")))?
                .join(format!("{}.bundle", record.id.hex())),
            std::time::Duration::from_secs(5),
            &|| context.cancelled(),
        )
        .map_err(|error| {
            if context.cancelled() {
                EngineError::from(crate::RuntimeError::Cancelled)
            } else {
                error.into()
            }
        })?;
        context.check_cancelled()?;
        if record.directory.exists() {
            let existing = read_record(&record.directory)?;
            if existing.id != record.id {
                return Err(EngineError::Edit(crate::EditError::Invalid(
                    "foreign LUT destination",
                )));
            }
            if (name.is_some() && existing.name != record.name)
                || (licence.is_some() && existing.licence != record.licence)
            {
                return Err(EngineError::Edit(crate::EditError::Invalid(
                    "conflicting LUT provenance",
                )));
            }
            match verify_bundle(&existing, || context.cancelled()) {
                Ok(()) => return Ok(existing),
                Err(
                    EngineError::Edit(crate::EditError::Invalid(_))
                    | EngineError::Control(crate::session::wire::ControlError::Limit),
                ) => {}
                Err(EngineError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
            if !repairing {
                return Err(EngineError::Edit(crate::EditError::Invalid(
                    "damaged LUT needs matching relink",
                )));
            }
            if existing != record {
                return Err(EngineError::Edit(crate::EditError::Invalid(
                    "LUT relink provenance",
                )));
            }
            let backup = super::projects::staging_directory(
                record
                    .directory
                    .parent()
                    .ok_or(EngineError::Edit(crate::EditError::Invalid("LUT parent")))?,
                "lut-repair",
            )?;
            let _backup = super::projects::DirectoryGuard(Some(backup.clone()));
            let old = backup.as_path().join("old");
            context.commit_publication(|| -> Result<()> {
                self.0.lease.validate()?;
                publication.validate()?;
                fs::rename(&record.directory, &old)?;
                if let Err(error) = fs::rename(stage.as_path(), &record.directory) {
                    fs::rename(&old, &record.directory)?;
                    return Err(error.into());
                }
                Ok(())
            })?;
            // Conservative disk accounting includes the replacement; no free-budget
            // assumption if another reader still has the prior files open.
        } else {
            context.commit_publication(|| -> Result<()> {
                self.0.lease.validate()?;
                publication.validate()?;
                parents(&record.directory)?;
                fs::rename(stage.as_path(), &record.directory)?;
                Ok(())
            })?;
        }
        pending.committed = true;
        Ok(record)
    }
}
fn hash_file(path: &Path, limit: u64, cancelled: &dyn Fn() -> bool) -> Result<media::ContentHash> {
    let mut file = fs::File::open(path)?.take(limit + 1);
    let mut hash = blake3::Hasher::new();
    let mut buffer = [0u8; 65536];
    let mut count = 0u64;
    loop {
        if cancelled() {
            return Err(crate::RuntimeError::Cancelled.into());
        }
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        count += n as u64;
        if count > limit {
            return Err(EngineError::Control(
                crate::session::wire::ControlError::Limit,
            ));
        }
        hash.update(buffer.get(..n).ok_or(EngineError::Control(
            crate::session::wire::ControlError::Limit,
        ))?);
    }
    Ok(media::ContentHash(*hash.finalize().as_bytes()))
}
fn read_provenance(directory: &Path) -> Result<Provenance> {
    parents(directory)?;
    if !plain(directory)?.is_dir() {
        return Err(EngineError::Edit(crate::EditError::Invalid("LUT bundle")));
    }
    for (index, entry) in fs::read_dir(directory)?.enumerate() {
        let entry = entry?;
        if index >= 3
            || !FILES.iter().any(|name| entry.file_name() == *name)
            || !plain(&entry.path())?.is_file()
        {
            return Err(EngineError::Edit(crate::EditError::Invalid(
                "foreign LUT bundle entry",
            )));
        }
    }
    let path = directory.join("provenance.json");
    let meta = plain(&path)?;
    if !meta.is_file() || meta.len() > 16384 {
        return Err(EngineError::Edit(crate::EditError::Invalid(
            "LUT provenance",
        )));
    }
    let mut file = fs::File::open(path)?.take(16385);
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    if bytes.len() > 16384 {
        return Err(EngineError::Control(
            crate::session::wire::ControlError::Limit,
        ));
    }
    serde_json::from_slice(&bytes)
        .map_err(|_| EngineError::Edit(crate::EditError::Invalid("LUT provenance")))
}
fn read_record(directory: &Path) -> Result<LutRecord> {
    read_provenance(directory)?.record(directory.to_owned())
}
/// Verify exactly the immutable files a persisted record names, without parsing
/// or changing project/history. Used by managed copy/history and import reuse.
pub(crate) fn verify_bundle(record: &LutRecord, cancelled: impl Fn() -> bool) -> Result<()> {
    let provenance = read_provenance(&record.directory)?;
    let actual_hash = provenance.copied_source_hash;
    if provenance.record(record.directory.clone())? != *record {
        return Err(EngineError::Edit(crate::EditError::Invalid(
            "LUT provenance differs",
        )));
    }
    for (name, limit, hash) in [
        (FILES[0], render::lut::MAX_CUBE_CANONICAL, record.id),
        (
            FILES[1],
            render::lut::MAX_CUBE_SOURCE,
            LutId::new(actual_hash),
        ),
    ] {
        let path = record.directory.join(name);
        let meta = plain(&path)?;
        if !meta.is_file() || meta.len() > limit {
            return Err(EngineError::Edit(crate::EditError::Invalid(
                "LUT bundle file",
            )));
        }
        if LutId::new(hash_file(&path, limit, &cancelled)?) != hash {
            return Err(EngineError::Edit(crate::EditError::Invalid(
                "LUT bundle changed",
            )));
        }
    }
    Ok(())
}

/// Copy the complete verified bundle into a private managed-project stage.
pub(crate) fn copy_bundle(
    record: &LutRecord,
    destination: &Path,
    budget: &crate::ByteBudget,
    cancelled: &dyn Fn() -> bool,
) -> Result<()> {
    let _lease = budget.reserve(crate::ByteOwner::PersistentCache, 256 << 10)?;
    verify_bundle(record, cancelled)?;
    parents(destination)?;
    fs::create_dir(destination)?;
    let mut buffer = [0u8; 65536];
    for name in FILES {
        let mut input = fs::File::open(record.directory.join(name))?.take(MAX_BUNDLE + 1);
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(destination.join(name))?;
        let mut count = 0u64;
        loop {
            if cancelled() {
                return Err(crate::RuntimeError::Cancelled.into());
            }
            let n = input.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            count += n as u64;
            if count > MAX_BUNDLE {
                return Err(EngineError::Control(
                    crate::session::wire::ControlError::Limit,
                ));
            }
            output.write_all(buffer.get(..n).ok_or(EngineError::Control(
                crate::session::wire::ControlError::Limit,
            ))?)?;
        }
        output.sync_all()?;
    }
    let mut copied = record.clone();
    copied.directory = destination.to_owned();
    verify_bundle(&copied, cancelled)
}

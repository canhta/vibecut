//! Stores, hashes and verifies kit files and the index that lists them.
use super::Result;
use super::record::{Index, MAX_FILE_BYTES, MAX_RECORD_BYTES, Media, valid_sha256};
use engine::BrandKitError;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub(super) const INDEX: &str = "brand_kits.json";
pub(super) const LEASE: &str = "brand_kits.vibecut";
const MEDIA: &str = "media";
const BUFFER: usize = 64 * 1024;

/// Whether a stored file is present and matches its recorded hash and size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FileState {
    Available,
    Missing,
    Corrupt,
}

pub(super) fn media_dir(root: &Path) -> PathBuf {
    root.join(MEDIA)
}

pub(super) fn media_path(root: &Path, sha256: &str) -> PathBuf {
    root.join(MEDIA).join(sha256)
}

/// Reads the index; a store with no index yet starts empty. A damaged index is `Integrity`.
pub(super) fn load_index(root: &Path) -> Result<Index> {
    let path = root.join(INDEX);
    if !path.exists() {
        return Ok(Index::empty());
    }
    let index: Index =
        serde_json::from_slice(&read_bounded(&path)?).map_err(|_| BrandKitError::Integrity)?;
    index.validate().map_err(|_| BrandKitError::Integrity)?;
    Ok(index)
}

pub(super) fn persist(root: &Path, index: &Index) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(index).map_err(|_| BrandKitError::Integrity)?;
    write_atomic(&root.join(INDEX), &bytes)
}

/// Writes through a `.partial` sibling and renames it into place, so a reader sees the old or
/// the new file, never a prefix.
pub(super) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let partial = partial_path(path);
    {
        let mut file = File::create(&partial)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    fs::rename(&partial, path)?;
    Ok(())
}

fn partial_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".partial");
    path.with_file_name(name)
}

fn read_bounded(path: &Path) -> Result<Vec<u8>> {
    let meta = fs::metadata(path)?;
    if !meta.is_file() {
        return Err(BrandKitError::Integrity);
    }
    if meta.len() > MAX_RECORD_BYTES {
        return Err(BrandKitError::Limit);
    }
    Ok(fs::read(path)?)
}

/// Copies `source` into the store while hashing it, verifies the stored copy by hash and
/// returns its SHA-256 and size. An identical file already stored is kept as it is.
pub(super) fn import_media(root: &Path, source: &Path) -> Result<(String, u64)> {
    let meta = fs::metadata(source)?;
    if !meta.is_file() {
        return Err(BrandKitError::Invalid("media source"));
    }
    if meta.len() > MAX_FILE_BYTES {
        return Err(BrandKitError::Limit);
    }
    let dir = media_dir(root);
    fs::create_dir_all(&dir)?;
    let partial = dir.join(format!(".partial-{}-{}", std::process::id(), nanos()));
    let copied = copy_hashed(source, &partial);
    let (sha256, size) = match copied {
        Ok(copied) => copied,
        Err(error) => {
            let _ = fs::remove_file(&partial);
            return Err(error.into());
        }
    };
    let target = media_path(root, &sha256);
    let intact =
        target.is_file() && hash_file(&target).is_ok_and(|found| found == (sha256.clone(), size));
    if intact {
        let _ = fs::remove_file(&partial);
    } else {
        let _ = fs::remove_file(&target);
        fs::rename(&partial, &target)?;
    }
    if hash_file(&target)? != (sha256.clone(), size) {
        let _ = fs::remove_file(&target);
        return Err(BrandKitError::Integrity);
    }
    Ok((sha256, size))
}

/// Whether `path` is a regular file whose bytes hash to `sha256` at exactly `size` bytes.
pub(super) fn source_matches(path: &Path, size: u64, sha256: &str) -> bool {
    match fs::metadata(path) {
        Ok(meta) if meta.is_file() && meta.len() == size => {
            hash_file(path).is_ok_and(|(found, _)| found == sha256)
        }
        _ => false,
    }
}

/// Verification state of one media record against its stored file.
pub(super) fn file_state(root: &Path, media: &Media) -> FileState {
    let path = media_path(root, &media.sha256);
    let Ok(meta) = fs::metadata(&path) else {
        return FileState::Missing;
    };
    if !meta.is_file() {
        return FileState::Missing;
    }
    if meta.len() != media.size || !valid_sha256(&media.sha256) {
        return FileState::Corrupt;
    }
    match hash_file(&path) {
        Ok((sha256, size)) if sha256 == media.sha256 && size == media.size => FileState::Available,
        _ => FileState::Corrupt,
    }
}

/// Removes stored media that no kit cites. Partial copies and foreign names are left alone.
pub(super) fn prune_media(root: &Path, referenced: &BTreeSet<String>) {
    let Ok(entries) = fs::read_dir(media_dir(root)) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if valid_sha256(name) && !referenced.contains(name) {
            let _ = fs::remove_file(entry.path());
        }
    }
}

fn copy_hashed(source: &Path, dest: &Path) -> io::Result<(String, u64)> {
    let mut input = File::open(source)?;
    let mut output = File::create(dest)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; BUFFER];
    let mut total = 0u64;
    loop {
        let read = input.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        total += read as u64;
        if total > MAX_FILE_BYTES {
            return Err(io::Error::other("media file grew past its limit"));
        }
        hasher.update(buffer.get(..read).unwrap_or_default());
        output.write_all(buffer.get(..read).unwrap_or_default())?;
    }
    output.sync_all()?;
    Ok((hex(&hasher.finalize()), total))
}

fn hash_file(path: &Path) -> io::Result<(String, u64)> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; BUFFER];
    let mut total = 0u64;
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        total += read as u64;
        hasher.update(buffer.get(..read).unwrap_or_default());
    }
    Ok((hex(&hasher.finalize()), total))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos())
}

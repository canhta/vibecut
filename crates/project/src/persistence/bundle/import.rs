//! Validates a bundle directory and extracts it into a new project directory.
use super::files::{self, copy_verified, open_regular, plain_directory, read_required, write_new};
use super::manifest::{
    BUNDLE_FORMAT_ID, MANIFEST_FILE, MAX_BUNDLE_BYTES, MAX_MANIFEST_BYTES, MAX_MEDIA_BYTES,
    MEDIA_DIR, Manifest, PROJECT_FILE, parse_hex, stored_name, valid_name,
};
use crate::persistence::bounds::MAX_BYTES;
use crate::{Error, Project, Result, SCHEMA_VERSION};
use media::{AssetData, AssetId, SourceAsset};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

/// A published project directory.
#[derive(Debug)]
pub struct ImportedBundle {
    project_path: PathBuf,
    project: Project,
    missing: Vec<AssetId>,
}
impl ImportedBundle {
    /// The extracted `.vibecut` file.
    pub fn project_path(&self) -> &Path {
        &self.project_path
    }
    /// The extracted project: a fresh identity, asset paths inside the new directory.
    pub fn project(&self) -> &Project {
        &self.project
    }
    /// Assets with no media in the bundle. They keep their identity and recorded
    /// hash, so a later relink accepts only identical content.
    pub fn missing(&self) -> &[AssetId] {
        &self.missing
    }
}

/// Extract `bundle` into the new directory `destination`.
///
/// Nothing in the bundle names a path: stored files are derived from each hash and
/// a name the closed rule accepts, every opened file must be a regular file (links
/// are refused, including `media/` itself), declared sizes must match, and each
/// copy is hashed against both the manifest and the asset. Duplicate entries,
/// entries that do not match the project's assets one to one, an unknown format
/// and any size beyond [`MAX_MEDIA_BYTES`] / [`MAX_BUNDLE_BYTES`] are refused. A
/// newer schema is `TooNew`, an older one `TooOld`. Media absent from the bundle
/// is reported in `missing`, not refused. The directory is staged beside the
/// destination and published by one rename; the project gets a fresh identity.
pub fn import(bundle: &Path, destination: &Path) -> Result<ImportedBundle> {
    if !plain_directory(bundle)? {
        return Err(Error::Invalid("bundle directory missing"));
    }
    let manifest: Manifest = serde_json::from_slice(&read_required(
        &bundle.join(MANIFEST_FILE),
        MAX_MANIFEST_BYTES,
    )?)?;
    if manifest.format != BUNDLE_FORMAT_ID {
        return Err(Error::Invalid("bundle format identifier"));
    }
    if manifest.schema_version < SCHEMA_VERSION {
        return Err(Error::TooOld {
            found: manifest.schema_version,
            supported: SCHEMA_VERSION,
        });
    }
    if manifest.schema_version > SCHEMA_VERSION {
        return Err(Error::TooNew {
            found: manifest.schema_version,
            supported: SCHEMA_VERSION,
        });
    }
    let loaded = crate::decode(&read_required(
        &bundle.join(PROJECT_FILE),
        MAX_BYTES as u64,
    )?)?;
    let hashes = verified_hashes(&manifest, &loaded.project)?;
    let source_dir = bundle.join(MEDIA_DIR);
    let source_dir = plain_directory(&source_dir)?.then_some(source_dir);

    let target = files::target(destination)?;
    let media_dir = target.staging.path().join(MEDIA_DIR);
    std::fs::create_dir(&media_dir)?;
    let inside = target.destination.join(MEDIA_DIR);
    let mut present: BTreeMap<String, bool> = BTreeMap::new();
    let mut paths: BTreeMap<AssetId, PathBuf> = BTreeMap::new();
    let mut missing = Vec::new();
    let mut total = 0u64;
    for entry in &manifest.media {
        let digest = *hashes
            .get(&entry.asset)
            .ok_or(Error::Invalid("bundle entry without asset"))?;
        let file = stored_name(&entry.hash, &entry.name);
        let copied = match present.get(&file) {
            Some(copied) => *copied,
            None => {
                let input = match &source_dir {
                    Some(dir) => open_regular(&dir.join(&file))?,
                    None => None,
                };
                let copied = match input {
                    Some((input, length)) => {
                        if length != entry.bytes {
                            return Err(Error::Invalid("bundle entry size differs from manifest"));
                        }
                        let bytes = copy_verified(
                            input,
                            &media_dir.join(&file),
                            media::ContentHash(digest),
                        )?;
                        if bytes != length {
                            return Err(Error::Invalid("bundle entry changed while reading"));
                        }
                        total = total
                            .checked_add(bytes)
                            .filter(|total| *total <= MAX_BUNDLE_BYTES)
                            .ok_or(Error::TooLarge("bundle size"))?;
                        true
                    }
                    None => false,
                };
                present.insert(file.clone(), copied);
                copied
            }
        };
        if !copied {
            missing.push(entry.asset);
        }
        paths.insert(entry.asset, inside.join(&file));
    }

    let mut data = loaded.project.into_data();
    let mut assets = Vec::with_capacity(data.assets.len());
    for asset in &data.assets {
        let mut relocated = AssetData::from(asset.clone());
        relocated.path = paths
            .remove(&asset.id())
            .ok_or(Error::Invalid("bundle entry without asset"))?;
        assets.push(SourceAsset::try_from(relocated)?);
    }
    data.assets = assets;
    let project = Project::new(data)?.fork()?;
    write_new(
        &target.staging.path().join(PROJECT_FILE),
        &crate::encode(&project, true)?,
    )?;
    let directory = target.publish()?;
    Ok(ImportedBundle {
        project_path: directory.join(PROJECT_FILE),
        project,
        missing,
    })
}

/// Check the manifest against the project before touching the destination:
/// one entry per asset, no duplicate ids, valid names, matching recorded hashes
/// and bounded sizes. Returns each asset's content hash.
fn verified_hashes(manifest: &Manifest, project: &Project) -> Result<BTreeMap<AssetId, [u8; 32]>> {
    let assets = &project.data().assets;
    if manifest.media.len() != assets.len() {
        return Err(Error::Invalid("bundle entries differ from project assets"));
    }
    let expected: BTreeMap<AssetId, [u8; 32]> = assets
        .iter()
        .map(|asset| (asset.id(), asset.content_hash().0))
        .collect();
    let mut seen = BTreeSet::new();
    let mut sizes: BTreeMap<String, u64> = BTreeMap::new();
    let mut hashes = BTreeMap::new();
    for entry in &manifest.media {
        if !seen.insert(entry.asset) {
            return Err(Error::Invalid("duplicate bundle entry"));
        }
        let recorded = parse_hex(&entry.hash).ok_or(Error::Invalid("bundle entry hash"))?;
        if expected.get(&entry.asset) != Some(&recorded) {
            return Err(Error::Invalid("bundle entry does not match its asset"));
        }
        if !valid_name(&entry.name) {
            return Err(Error::Invalid("bundle entry name"));
        }
        if entry.bytes > MAX_MEDIA_BYTES {
            return Err(Error::TooLarge("bundle media"));
        }
        if *sizes
            .entry(stored_name(&entry.hash, &entry.name))
            .or_insert(entry.bytes)
            != entry.bytes
        {
            return Err(Error::Invalid("bundle entry sizes disagree"));
        }
        hashes.insert(entry.asset, recorded);
    }
    Ok(hashes)
}

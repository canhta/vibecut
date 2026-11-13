//! Writes a project and its media as a new bundle directory.
use super::files::{self, copy_verified, write_new};
use super::manifest::{
    BUNDLE_FORMAT_ID, Entry, MANIFEST_FILE, MAX_BUNDLE_BYTES, MEDIA_DIR, Manifest, PROJECT_FILE,
    hex, sanitize, stored_name,
};
use crate::{Error, Project, Result, SCHEMA_VERSION};
use media::{AssetData, AssetId, SourceAsset};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    path::{Path, PathBuf},
};

/// A published bundle.
#[derive(Debug)]
pub struct ExportedBundle {
    path: PathBuf,
    copied: usize,
    missing: Vec<AssetId>,
}
impl ExportedBundle {
    pub fn path(&self) -> &Path {
        &self.path
    }
    /// Distinct media files copied into the bundle.
    pub fn copied(&self) -> usize {
        self.copied
    }
    /// Assets whose media was absent at export; importing them enters relink.
    pub fn missing(&self) -> &[AssetId] {
        &self.missing
    }
}

/// Write `project` and every present media file to a new directory `destination`.
///
/// The directory is staged beside the destination, synced, then published by one
/// rename, so a failure or crash never leaves a partial bundle at the destination.
/// An existing destination is refused. Missing media is recorded, not an error;
/// media whose bytes no longer match the asset's recorded hash is refused. Source
/// paths are not written: the envelope's asset paths point inside the bundle.
pub fn export(project: &Project, destination: &Path) -> Result<ExportedBundle> {
    let target = files::target(destination)?;
    let media_dir = target.staging.path().join(MEDIA_DIR);
    fs::create_dir(&media_dir)?;
    let inside = target.destination.join(MEDIA_DIR);
    let mut data = project.data().clone();
    let mut assets = Vec::with_capacity(data.assets.len());
    let mut media = Vec::with_capacity(data.assets.len());
    let mut copies: BTreeMap<String, u64> = BTreeMap::new();
    let mut missing = Vec::new();
    let mut total = 0u64;
    for asset in &data.assets {
        let name = sanitize(
            &asset
                .path()
                .file_name()
                .unwrap_or_default()
                .to_string_lossy(),
        );
        let hash = hex(&asset.content_hash().0);
        let file = stored_name(&hash, &name);
        let copied = match copies.get(&file) {
            Some(bytes) => Some(*bytes),
            None => match source(asset)? {
                Some(input) => {
                    let bytes = copy_verified(input, &media_dir.join(&file), asset.content_hash())?;
                    total = total
                        .checked_add(bytes)
                        .filter(|total| *total <= MAX_BUNDLE_BYTES)
                        .ok_or(Error::TooLarge("bundle size"))?;
                    copies.insert(file.clone(), bytes);
                    Some(bytes)
                }
                None => None,
            },
        };
        if copied.is_none() {
            missing.push(asset.id());
        }
        let bytes = copied.unwrap_or(0);
        let mut relocated = AssetData::from(asset.clone());
        relocated.path = inside.join(&file);
        assets.push(SourceAsset::try_from(relocated)?);
        media.push(Entry {
            asset: asset.id(),
            name,
            hash,
            bytes,
        });
    }
    data.assets = assets;
    let bundled = Project::new(data)?;
    write_new(
        &target.staging.path().join(PROJECT_FILE),
        &crate::encode(&bundled, true)?,
    )?;
    let manifest = Manifest {
        format: BUNDLE_FORMAT_ID.to_owned(),
        schema_version: SCHEMA_VERSION,
        generator: format!("VibeCut {}", env!("CARGO_PKG_VERSION")),
        media,
    };
    write_new(
        &target.staging.path().join(MANIFEST_FILE),
        &serde_json::to_vec_pretty(&manifest)?,
    )?;
    let copied = copies.len();
    let path = target.publish()?;
    Ok(ExportedBundle {
        path,
        copied,
        missing,
    })
}

/// The asset's media when it is a present regular file.
fn source(asset: &SourceAsset) -> Result<Option<File>> {
    match fs::metadata(asset.path()) {
        Ok(meta) if meta.is_file() => Ok(Some(File::open(asset.path())?)),
        Ok(_) => Ok(None),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

//! Exports a kit to one folder and imports such a folder, verifying every file it names.
use super::Result;
use super::files::{self, FileState};
use super::record::{Content, MAX_RECORD_BYTES, SCHEMA};
use super::store::BrandKit;
use engine::BrandKitError;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

const RECORD: &str = "kit.json";
const MEDIA: &str = "media";

/// The archive record: schema plus the kit contents, with no local identity or paths.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Archive {
    schema: u32,
    kit: Content,
}

impl BrandKit {
    /// Writes kit `id` into the empty folder `dest` as `kit.json` plus `media/<sha256>` files.
    /// Refuses when a stored file is missing or corrupt, so an export is never partial.
    pub fn export_kit(&self, id: u64, dest: &Path) -> Result<()> {
        let content = self.content(id)?;
        if dest.exists() && fs::read_dir(dest)?.next().is_some() {
            return Err(BrandKitError::Invalid("archive destination"));
        }
        fs::create_dir_all(dest.join(MEDIA))?;
        for media in &content.media {
            if files::file_state(self.root(), media) != FileState::Available {
                return Err(BrandKitError::Integrity);
            }
            fs::copy(
                files::media_path(self.root(), &media.sha256),
                dest.join(MEDIA).join(&media.sha256),
            )?;
        }
        let bytes = serde_json::to_vec_pretty(&Archive {
            schema: SCHEMA,
            kit: content,
        })
        .map_err(|_| BrandKitError::Integrity)?;
        files::write_atomic(&dest.join(RECORD), &bytes)
    }

    /// Reads an archive folder written by `export_kit`, verifies each named file against its
    /// recorded hash and size, and adds the kit under a new local identity.
    pub fn import_kit(&self, src: &Path) -> Result<u64> {
        let record = src.join(RECORD);
        let meta = fs::metadata(&record)?;
        if !meta.is_file() || meta.len() > MAX_RECORD_BYTES {
            return Err(BrandKitError::Invalid("archive record"));
        }
        let archive: Archive = serde_json::from_slice(&fs::read(&record)?)
            .map_err(|_| BrandKitError::Invalid("archive record"))?;
        if archive.schema != SCHEMA {
            return Err(BrandKitError::Invalid("archive schema"));
        }
        // Content::validate admits only lowercase SHA-256 names, so no media path can leave
        // the archive folder or the store's media directory.
        archive.kit.validate()?;
        let mut state = self.lock()?;
        for media in &archive.kit.media {
            let source = src.join(MEDIA).join(&media.sha256);
            if !files::source_matches(&source, media.size, &media.sha256) {
                return Err(BrandKitError::Integrity);
            }
            files::import_media(self.root(), &source)?;
        }
        self.adopt(&mut state, archive.kit)
    }
}

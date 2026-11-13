//! Persisted brand kit records, their limits and admission rules.
use super::Result;
use engine::BrandKitError;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Most kits the store holds.
pub const MAX_KITS: usize = 16;
/// Most entries of each kind one kit holds.
pub const MAX_ENTRIES: usize = 64;
/// Largest stored media file, in bytes.
pub const MAX_FILE_BYTES: u64 = 256 << 20;
/// Longest kit, colour, font, media or preset name, in UTF-8 bytes.
pub const MAX_NAME_BYTES: usize = 128;
/// Schema of the store index and of archive records.
pub(super) const SCHEMA: u32 = 1;
/// Largest index or archive record read from disk.
pub(super) const MAX_RECORD_BYTES: u64 = 4 << 20;

/// The whole store as persisted: every kit and the one active selection.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Index {
    pub schema: u32,
    pub next_id: u64,
    pub active: Option<u64>,
    pub kits: Vec<Kit>,
}

/// A kit with its local identity. The identity never leaves the store.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Kit {
    pub id: u64,
    pub content: Content,
}

/// Kit contents without local identity, so an exported archive is the same wherever it is imported.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Content {
    pub name: String,
    pub colors: Vec<BrandColor>,
    pub fonts: Vec<String>,
    pub media: Vec<Media>,
    pub text_presets: Vec<project::SavedTextPreset>,
    pub caption_presets: Vec<project::SavedCaptionPreset>,
}

/// One named colour, ordered by insertion.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct BrandColor {
    pub name: String,
    pub rgba: [u8; 4],
}

/// One media file. The stored file is named by its SHA-256, so names never reach the filesystem.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Media {
    pub id: u64,
    pub name: String,
    pub sha256: String,
    pub size: u64,
}

impl Index {
    pub(super) fn empty() -> Self {
        Self {
            schema: SCHEMA,
            next_id: 1,
            active: None,
            kits: Vec::new(),
        }
    }

    pub(super) fn kit(&self, id: u64) -> Result<&Kit> {
        self.kits
            .iter()
            .find(|kit| kit.id == id)
            .ok_or(BrandKitError::NotFound)
    }

    pub(super) fn kit_mut(&mut self, id: u64) -> Result<&mut Kit> {
        self.kits
            .iter_mut()
            .find(|kit| kit.id == id)
            .ok_or(BrandKitError::NotFound)
    }

    /// Every media hash any kit cites; files outside this set are unreferenced.
    pub(super) fn referenced_media(&self) -> BTreeSet<String> {
        self.kits
            .iter()
            .flat_map(|kit| kit.content.media.iter().map(|media| media.sha256.clone()))
            .collect()
    }

    pub(super) fn validate(&self) -> Result<()> {
        if self.schema != SCHEMA || self.kits.len() > MAX_KITS {
            return Err(invalid("brand kit index"));
        }
        check_unique(
            self.kits.iter().map(|kit| kit.content.name.as_str()),
            "duplicate kit name",
        )?;
        let mut ids = BTreeSet::new();
        for kit in &self.kits {
            if !ids.insert(kit.id) || kit.id >= self.next_id {
                return Err(invalid("kit identity"));
            }
            kit.content.validate()?;
        }
        if let Some(active) = self.active
            && !ids.contains(&active)
        {
            return Err(invalid("active kit"));
        }
        Ok(())
    }
}

impl Content {
    pub(super) fn named(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            colors: Vec::new(),
            fonts: Vec::new(),
            media: Vec::new(),
            text_presets: Vec::new(),
            caption_presets: Vec::new(),
        }
    }

    /// Structural admission for a whole kit: names, bounds, unique names per kind and
    /// well-formed media records. Files are verified separately, against these records.
    pub(super) fn validate(&self) -> Result<()> {
        check_name(&self.name)?;
        check_count(self.colors.len())?;
        check_count(self.fonts.len())?;
        check_count(self.media.len())?;
        check_count(self.text_presets.len())?;
        check_count(self.caption_presets.len())?;
        for color in &self.colors {
            check_name(&color.name)?;
        }
        check_unique(
            self.colors.iter().map(|c| c.name.as_str()),
            "duplicate colour",
        )?;
        for family in &self.fonts {
            check_name(family)?;
        }
        check_unique(self.fonts.iter().map(String::as_str), "duplicate font")?;
        for media in &self.media {
            check_name(&media.name)?;
            if !valid_sha256(&media.sha256) || media.size > MAX_FILE_BYTES {
                return Err(invalid("media record"));
            }
        }
        check_unique(
            self.media.iter().map(|m| m.name.as_str()),
            "duplicate media",
        )?;
        check_unique_ids(self.media.iter().map(|m| m.id))?;
        for preset in &self.text_presets {
            if !project::SavedTextPreset::valid_name(&preset.name) {
                return Err(invalid("text preset name"));
            }
        }
        check_unique(
            self.text_presets.iter().map(|p| p.name.as_str()),
            "duplicate text preset",
        )?;
        for preset in &self.caption_presets {
            if !project::SavedCaptionPreset::valid_name(&preset.name) {
                return Err(invalid("caption preset name"));
            }
        }
        check_unique(
            self.caption_presets.iter().map(|p| p.name.as_str()),
            "duplicate caption preset",
        )
    }
}

/// Name rule shared by kits and entries: 1 to `MAX_NAME_BYTES` bytes, no controls, no
/// path separators, no leading or trailing whitespace.
pub(super) fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_NAME_BYTES
        && name.trim() == name
        && !name
            .chars()
            .any(|c| c.is_control() || c == '/' || c == '\\')
}

pub(super) fn check_name(name: &str) -> Result<()> {
    if valid_name(name) {
        Ok(())
    } else {
        Err(invalid("name"))
    }
}

pub(super) fn check_count(len: usize) -> Result<()> {
    if len > MAX_ENTRIES {
        Err(BrandKitError::Limit)
    } else {
        Ok(())
    }
}

pub(super) fn check_unique<'a>(
    names: impl IntoIterator<Item = &'a str>,
    what: &'static str,
) -> Result<()> {
    let mut seen = BTreeSet::new();
    for name in names {
        if !seen.insert(name) {
            return Err(invalid(what));
        }
    }
    Ok(())
}

fn check_unique_ids(ids: impl IntoIterator<Item = u64>) -> Result<()> {
    let mut seen = BTreeSet::new();
    for id in ids {
        if !seen.insert(id) {
            return Err(invalid("duplicate media identity"));
        }
    }
    Ok(())
}

/// A lowercase hexadecimal SHA-256, the only form accepted as a stored file name.
pub(super) fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn invalid(what: &'static str) -> BrandKitError {
    BrandKitError::Invalid(what)
}

//! Optional presentation metadata; never part of editable project snapshots.
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::{
    io::Read,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use time::Tick;

pub const METADATA_VERSION: u32 = 2;
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ViewState {
    /// Timeline scale multiplier, greater than zero and at most 1000.
    pub zoom: f64,
    /// Horizontal scroll in logical pixels, 0..=1e9.
    pub scroll: f64,
    pub playhead: Tick,
}
impl Default for ViewState {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            scroll: 0.0,
            playhead: Tick::ZERO,
        }
    }
}
impl ViewState {
    pub fn validate(&self) -> Result<()> {
        if !self.zoom.is_finite()
            || self.zoom <= 0.0
            || self.zoom > 1000.0
            || !self.scroll.is_finite()
            || !(0.0..=1e9).contains(&self.scroll)
            || self.playhead < Tick::ZERO
        {
            return Err(Error::Invalid("view state"));
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Metadata {
    pub version: u32,
    pub view: ViewState,
    pub scene_views: std::collections::BTreeMap<crate::SceneId, ViewState>,
    pub created_unix_ms: u64,
    pub modified_unix_ms: u64,
}
impl Default for Metadata {
    fn default() -> Self {
        let now = now_ms();
        Self {
            version: METADATA_VERSION,
            view: ViewState::default(),
            scene_views: Default::default(),
            created_unix_ms: now,
            modified_unix_ms: now,
        }
    }
}
impl Metadata {
    pub fn validate(&self) -> Result<()> {
        if self.version != METADATA_VERSION || self.modified_unix_ms < self.created_unix_ms {
            return Err(Error::Invalid("metadata version or timestamps"));
        }
        if self.scene_views.len() > 256 {
            return Err(Error::TooLarge("scene views"));
        }
        for view in self.scene_views.values() {
            view.validate()?;
        }
        self.view.validate()
    }
    pub fn touch(&mut self) {
        self.modified_unix_ms = now_ms().max(self.created_unix_ms);
    }
}
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| u64::try_from(d.as_millis()).ok())
        .unwrap_or(0)
}
pub(crate) fn path(project: &Path) -> Result<PathBuf> {
    let mut name = project
        .file_name()
        .ok_or(Error::Invalid("metadata path"))?
        .to_os_string();
    name.push(".meta.json");
    Ok(project.with_file_name(name))
}
/// Optional, bounded metadata. Missing, corrupt, unsupported or unreadable metadata
/// falls back to defaults and never prevents the project from opening.
pub(crate) fn load(project: &Path) -> Metadata {
    fn read(project: &Path) -> Result<Metadata> {
        let path = path(project)?;
        if !std::fs::symlink_metadata(&path)?.file_type().is_file() {
            return Err(Error::Invalid("metadata file"));
        }
        let mut bytes = Vec::new();
        std::fs::File::open(path)?
            .take(65537)
            .read_to_end(&mut bytes)?;
        if bytes.len() > 65536 {
            return Err(Error::TooLarge("metadata"));
        }
        let value: Metadata = serde_json::from_slice(&bytes)?;
        value.validate()?;
        Ok(value)
    }
    read(project).unwrap_or_default()
}
pub(crate) fn save(
    project: &Path,
    value: &Metadata,
    check: impl FnOnce() -> Result<()>,
) -> Result<()> {
    value.validate()?;
    let bytes = serde_json::to_vec(value)?;
    if bytes.len() > 65536 {
        return Err(Error::TooLarge("metadata"));
    }
    crate::atomic::atomic_write_checked(&path(project)?, &bytes, check)
}

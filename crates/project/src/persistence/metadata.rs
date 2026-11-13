//! Optional presentation metadata; never part of editable project snapshots.
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use time::Tick;

pub const METADATA_VERSION: u32 = 4;
/// Small uncompressed sRGB RGBA thumbnail; consumers never decode untrusted images.
#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum Thumbnail {
    #[default]
    Missing,
    Unavailable,
    Ready {
        width: u32,
        height: u32,
        rgba: Vec<u8>,
    },
}
impl Thumbnail {
    pub fn validate(&self) -> Result<()> {
        if let Self::Ready {
            width,
            height,
            rgba,
        } = self
            && (!(1..=128).contains(width)
                || !(1..=128).contains(height)
                || u64::from(*width) * u64::from(*height) * 4 != rgba.len() as u64)
        {
            return Err(Error::Invalid("thumbnail dimensions or bytes"));
        }
        Ok(())
    }
}
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
    pub thumbnail: Thumbnail,
    pub view: ViewState,
    pub scene_views: std::collections::BTreeMap<crate::SceneId, ViewState>,
    /// Preview reads each source's cached proxy while on. A presentation choice
    /// like the view; the project file never names a proxy and export reads sources.
    pub proxy_preview: bool,
    pub created_unix_ms: u64,
    pub modified_unix_ms: u64,
}
impl Default for Metadata {
    fn default() -> Self {
        let now = now_ms();
        Self {
            version: METADATA_VERSION,
            thumbnail: Thumbnail::Missing,
            view: ViewState::default(),
            scene_views: Default::default(),
            proxy_preview: false,
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
        self.thumbnail.validate()?;
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
/// Native metadata sidecar path for a project; refuses a missing filename.
pub fn path(project: &Path) -> Result<PathBuf> {
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
            .take(524289)
            .read_to_end(&mut bytes)?;
        if bytes.len() > 524288 {
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
    if bytes.len() > 524288 {
        return Err(Error::TooLarge("metadata"));
    }
    super::atomic::stage_validated(
        &path(project)?,
        |file| file.write_all(&bytes).map_err(Error::from),
        |_| check(),
    )
}

impl std::fmt::Debug for Thumbnail {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing => f.write_str("Missing"),
            Self::Unavailable => f.write_str("Unavailable"),
            Self::Ready {
                width,
                height,
                rgba,
            } => f
                .debug_struct("Ready")
                .field("width", width)
                .field("height", height)
                .field("bytes", &rgba.len())
                .finish(),
        }
    }
}

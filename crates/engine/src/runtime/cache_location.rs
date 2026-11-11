//! Discovers the per-user cache location.
use crate::{Error as EngineError, Result};
use std::path::PathBuf;

/// Per-user cache paths, discovered without filesystem IO.
/// Explicitly injected artifact roots remain independent of this location.
pub struct CacheRoot {
    root: PathBuf,
}

impl CacheRoot {
    /// Select the native OS cache base without creating, reading or removing files.
    /// Native environment values retain their path representation.
    ///
    /// # Errors
    /// Returns [`crate::EditError::Unsupported`] composed through [`crate::Error::Edit`] when the selected base is absent or
    /// nonabsolute, including a present empty or relative `XDG_CACHE_HOME`.
    pub fn discover() -> Result<Self> {
        #[cfg(target_os = "macos")]
        let base = std::env::var_os("HOME").map(|p| PathBuf::from(p).join("Library/Caches"));
        #[cfg(target_os = "windows")]
        let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        let base = std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".cache")));
        let base = base.filter(|p| p.is_absolute()).ok_or(EngineError::Edit(
            crate::EditError::Unsupported("OS cache location unavailable"),
        ))?;
        Ok(Self {
            root: base.join("VibeCut"),
        })
    }

    /// Model bundles and recognition checkpoints: `<base>/VibeCut/models`.
    pub fn models(&self) -> PathBuf {
        self.root.join("models")
    }

    /// Downloaded fonts: `<base>/VibeCut/fonts`.
    pub fn fonts(&self) -> PathBuf {
        self.root.join("fonts")
    }

    /// Waveform artifacts: `<base>/VibeCut/models/waveform`.
    pub fn waveform(&self) -> PathBuf {
        self.models().join("waveform")
    }

    /// Thumbnail artifacts: `<base>/VibeCut/models/thumbnails`.
    pub fn thumbnails(&self) -> PathBuf {
        self.models().join("thumbnails")
    }
}

impl std::fmt::Debug for CacheRoot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CacheRoot").finish_non_exhaustive()
    }
}

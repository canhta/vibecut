//! Provides openly licensed music tracks for projects.
//!
//! The source is an open aggregator that answers anonymous queries; a user key
//! only raises its rate limit. Admission keeps public-domain-dedication and
//! attribution-only licences and drops every other licence even when the server
//! ignores the filter.
use engine::cache::{ArtifactKind, CacheKey, KeyInputs};
use engine::jobs::{
    CancelToken, JobBytes, JobId, JobKind, JobRunner, JobSpec, JobStage, ResourceClass,
};
use engine::{Placement, Session, control::Error as EngineError, control::Result};
use media::{AssetData, ContentHash, SourceAsset, SourceProbe};
use project::atomic::OwnedDirectory;
use serde::{Deserialize, Serialize};
use std::{
    fmt, fs,
    io::{Read, Write},
    ops::Deref,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};
use time::Tick;
use transport::{DownloadTransport, HttpRequest, HttpTransport, download_redirect};

pub const SEARCH_COMMAND: &str = "music.search";
pub const SAVE_COMMAND: &str = "music.save";
pub const ADD_COMMAND: &str = "music.add_to_timeline";

const API: &str = "https://api.openverse.org/v1/audio/";
/// Only these two licence families are admitted: a public-domain dedication and
/// attribution-only. NC, share-alike and no-derivatives values never pass.
const LICENCES: &str = "cc0,by";
/// Anonymous requests are capped at twenty results per page; keeping the bound
/// also keeps a user key from changing the page contract under the UI.
const MAX_PAGE_SIZE: u32 = 20;
const MAX_SEARCH_BYTES: usize = 1 << 20;
const MAX_LIBRARY_BYTES: u64 = 1 << 20;
const MAX_SAVED: usize = 512;
const MAX_CACHE_ENTRIES: usize = 10_000;
const MAX_AUDIO_BYTES: u64 = 32 << 20;
const MAX_QUERY: usize = 200;
const MAX_URL: usize = 8192;
const LIBRARY_FORMAT: &str = "vibecut.music-library";
const LIBRARY_VERSION: u32 = 1;

use engine::MusicError as Error;
pub type Music<T> = std::result::Result<T, Error>;

/// Mood and genre chips, each a tag the source catalogue carries. The list is
/// provider data, not UI copy; the panel labels the chip with the tag itself.
pub const CHIPS: [&str; 12] = [
    "acoustic",
    "ambient",
    "chill",
    "cinematic",
    "electronic",
    "hip hop",
    "jazz",
    "lo-fi",
    "piano",
    "pop",
    "rock",
    "upbeat",
];

/// User-supplied key held only for this process. Debug is always redacted.
#[derive(Clone)]
pub struct MusicToken(String);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchRequest {
    /// Free text; mutually exclusive with `tag`.
    pub query: String,
    /// A source tag the catalogue carries.
    pub tag: Option<String>,
    pub page: u32,
    pub page_size: u32,
}

/// Provider-neutral result. Attribution is precomputed so every surface shows
/// the same author/licence text and exports retain it verbatim.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MusicResult {
    pub id: String,
    pub title: String,
    pub author: String,
    /// Short licence name shown beside the author, e.g. `CC BY 4.0`.
    pub licence: String,
    pub licence_url: String,
    pub attribution: String,
    pub source_url: String,
    pub audio_url: String,
    pub duration_ms: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchSource {
    Remote,
    Local,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchPage {
    pub count: u64,
    pub page: u32,
    pub page_size: u32,
    pub results: Vec<MusicResult>,
    pub has_next: bool,
    pub source: SearchSource,
}

/// Direct provider client. Redirects are disabled on the JSON transport so a
/// user key can never follow a redirect to another host.
pub struct MusicClient {
    token: Option<MusicToken>,
    transport: Arc<dyn HttpTransport>,
}
impl Clone for MusicClient {
    fn clone(&self) -> Self {
        Self {
            token: self.token.clone(),
            transport: self.transport.clone(),
        }
    }
}

#[derive(Deserialize)]
struct RawPage {
    #[serde(default)]
    result_count: u64,
    #[serde(default)]
    page_count: u64,
    results: Vec<serde_json::Value>,
}
#[derive(Deserialize)]
struct RawTrack {
    id: Option<String>,
    title: Option<String>,
    creator: Option<String>,
    url: Option<String>,
    foreign_landing_url: Option<String>,
    license: Option<String>,
    license_version: Option<String>,
    license_url: Option<String>,
    attribution: Option<String>,
    duration: Option<i64>,
}

/// One saved track and its local save time. The provider result is immutable so a
/// cached file and its attribution always describe the same representation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SavedMusic {
    pub music: MusicResult,
    pub saved_at_unix_ms: u64,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredLibrary {
    format: String,
    version: u32,
    tracks: Vec<SavedMusic>,
}
/// Durable local library: `library.json` plus content-addressed audio bytes.
/// It never stores a credential or an absolute project path.
pub struct MusicLibrary {
    root: OwnedDirectory,
    audio: OwnedDirectory,
    lease: project::WriterLease,
    state: Mutex<Vec<SavedMusic>>,
    /// Bumped on every committed Saved change, so another handle on this shared
    /// library can tell its copy of Saved is stale without re-reading it.
    revision: AtomicU64,
    operation: AtomicBool,
}

struct Operation<T: Deref<Target = MusicLibrary>>(T);
impl<T: Deref<Target = MusicLibrary>> Drop for Operation<T> {
    fn drop(&mut self) {
        self.0.operation.store(false, Ordering::Release);
    }
}
impl MusicLibrary {
    fn validate(&self) -> project::Result<()> {
        self.root.validate()?;
        self.audio.validate()?;
        self.lease.validate()?;
        Ok(())
    }
    fn operation(&self) -> Music<Operation<&Self>> {
        Operation::acquire(self)
    }
}
impl<T: Deref<Target = MusicLibrary>> Operation<T> {
    fn acquire(library: T) -> Music<Self> {
        library
            .operation
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| Error::Project(project::Error::Busy))?;
        let operation = Self(library);
        operation.0.validate()?;
        Ok(operation)
    }
}
static NEXT: AtomicU64 = AtomicU64::new(0);

/// A durable cached file with the probed media identity needed for a project asset.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CachedMusic {
    pub path: PathBuf,
    pub content_hash: ContentHash,
    pub probe: SourceProbe,
}

/// Streaming music downloader. It performs no retries; the shared runner owns the
/// attempt budget. No credential is sent to audio/CDN URLs.
pub struct MusicDownloads {
    library: Arc<MusicLibrary>,
    transport: Arc<dyn DownloadTransport>,
}

/// One preview at a time. Stop cancels an in-flight download and delegates to the
/// app-owned sink; a missing device is a typed `NoAudioDevice` from the sink.
pub struct MusicPreview {
    downloads: Arc<MusicDownloads>,
    sink: Arc<dyn crate::sound_library::PreviewSink>,
    active: Mutex<Option<Arc<AtomicBool>>>,
}
mod search;

mod saved;

mod cache;

mod downloads;

mod preview;

mod timeline;

mod control;

use search::{hex, url};

use saved::{private_directory, validate_result};

use cache::cached_from_path;

pub use timeline::{add_saved, add_to_timeline};

pub use crate::sound_library::{PreviewClip, PreviewSink};

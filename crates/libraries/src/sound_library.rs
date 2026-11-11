//! Provides licensed sound assets for projects.
use ai::transport::{DownloadTransport, HttpRequest, HttpTransport, download_redirect};
use engine::cache::{ArtifactKind, CacheKey, KeyInputs};
use engine::jobs::{
    CancelToken, JobBytes, JobId, JobKind, JobRunner, JobSpec, JobStage, ResourceClass,
};
use engine::{Placement, Session, control::Error as EngineError, control::Result};
use media::{AssetData, ContentHash, SourceAsset, SourceProbe};
use project::LibrarySound;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fmt, fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};
use time::Tick;

pub const SEARCH_COMMAND: &str = "sounds.search";
pub const SAVE_COMMAND: &str = "sounds.save";
pub const ADD_COMMAND: &str = "sounds.add_to_timeline";

const API: &str = "https://freesound.org/apiv2/search/";
const FIELDS: &str = "id,name,username,license,num_downloads,previews,download,duration,url";
/// Commercial use is already licensed for CC0 and CC BY. NC, Sampling+ and any
/// undocumented values are excluded rather than guessed.
const COMMERCIAL_FILTER: &str = r#"license:("Creative Commons 0" OR "Attribution")"#;
const MAX_SEARCH_BYTES: usize = 1 << 20;
const MAX_LIBRARY_BYTES: u64 = 1 << 20;
const MAX_SAVED: usize = 1024;
const MAX_AUDIO_BYTES: u64 = 32 << 20;
const MAX_QUERY: usize = 500;
const MAX_URL: usize = 8192;
const MAX_TEXT: usize = 4096;
const LIBRARY_FORMAT: &str = "vibecut.sound-library";
const LIBRARY_VERSION: u32 = 1;

use engine::SoundError as Error;
pub type Sound<T> = std::result::Result<T, Error>;

/// User-supplied credential held only for this process. Debug is always redacted.
#[derive(Clone)]
pub struct Token(String);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchRequest {
    pub query: String,
    pub page: u32,
    pub page_size: u32,
    /// Default is commercial-only; false includes NC and legacy licences.
    pub commercial_only: bool,
}

/// Provider-neutral result. Attribution is precomputed so every surface shows the
/// same author/licence text and exports retain it verbatim.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SoundResult {
    pub id: u64,
    pub name: String,
    pub author: String,
    pub licence: String,
    pub attribution: String,
    pub source_url: String,
    pub audio_url: String,
    pub duration_ms: u64,
    pub downloads: u64,
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
    pub results: Vec<SoundResult>,
    pub has_next: bool,
    pub source: SearchSource,
}

/// Direct provider client. Redirects are disabled on the JSON transport so the
/// Authorization header can never follow a redirect to another host.
pub struct SoundClient {
    token: Token,
    transport: Arc<dyn HttpTransport>,
}

#[derive(Deserialize)]
struct RawPage {
    count: u64,
    next: Option<String>,
    results: Vec<RawSound>,
}
#[derive(Deserialize)]
struct RawSound {
    id: u64,
    name: String,
    username: String,
    license: String,
    #[serde(default)]
    num_downloads: u64,
    #[serde(default)]
    previews: BTreeMap<String, String>,
    #[serde(default)]
    duration: Option<f64>,
    url: String,
}

/// One saved sound and its local save time. The provider result is immutable so a
/// cached preview and its attribution always describe the same representation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SavedSound {
    pub sound: SoundResult,
    pub saved_at_unix_ms: u64,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredLibrary {
    format: String,
    version: u32,
    sounds: Vec<SavedSound>,
}
/// Durable local library: `library.json` plus content-addressed preview bytes.
/// It never stores a credential or an absolute project path. One handle serializes
/// library mutations; preview downloads may run concurrently and publish atomically.
pub struct SoundLibrary {
    root: PathBuf,
    audio: PathBuf,
    lease: project::WriterLease,
    state: Mutex<Vec<SavedSound>>,
}
static NEXT: AtomicU64 = AtomicU64::new(0);

/// A durable cached preview with the probed media identity needed for a project asset.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CachedSound {
    pub path: PathBuf,
    pub content_hash: ContentHash,
    pub probe: SourceProbe,
}

struct Busy(Arc<AtomicBool>);
impl Drop for Busy {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

/// Streaming preview downloader. It performs no retries; the shared runner owns the
/// attempt budget. No credential is sent to preview/CDN URLs.
pub struct SoundDownloads {
    library: Arc<SoundLibrary>,
    transport: Arc<dyn DownloadTransport>,
    max_bytes: u64,
    busy: Arc<AtomicBool>,
}

/// Borrowed preview bytes and a cancellation predicate, valid until terminal play.
pub struct PreviewClip<'a> {
    pub name: &'a str,
    pub audio_url: &'a str,
    pub bytes: &'a [u8],
    pub cancelled: &'a dyn Fn() -> bool,
}
pub trait PreviewSink: Send + Sync {
    /// Return once, at device drain, stop/cancellation or typed failure, never at
    /// startup. Poll cancellation during decode/conversion/playback. The caller
    /// retains and joins its worker; the sink must not detach borrowed work.
    fn play(&self, clip: PreviewClip<'_>) -> Sound<()>;
    /// Idempotent, nonblocking cancellation, including while play is decoding.
    fn stop(&self) -> Sound<()>;
}
/// One preview at a time. Stop cancels an in-flight download and delegates to the
/// sink; a missing device is a typed `NoAudioDevice` from the sink.
pub struct SoundPreview {
    downloads: Arc<SoundDownloads>,
    sink: Arc<dyn PreviewSink>,
    active: Mutex<Option<Arc<AtomicBool>>>,
}
mod search;

mod saved;

mod cache;

mod downloads;

mod preview;

mod timeline;

mod control;

use search::{hex, retry_after, url};

pub use search::commercial;

use saved::{private_directory, validate_result};

use cache::cached_from_path;

pub use timeline::{add_saved, add_to_timeline};

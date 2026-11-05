//! Bring-your-own-credential sound search over the public Freesound APIv2, plus a
//! durable local saved library and content-addressed cached previews. The hosted
//! browser proxy is deliberately absent: the desktop adapter calls the provider
//! directly with the user's token and degrades to the saved local library when no
//! credential is configured. No token is ever serialized or logged.
use crate::cache::{ArtifactKind, CacheKey, KeyInputs};
use crate::jobs::{
    CancelToken, JobBytes, JobId, JobKind, JobRunner, JobSpec, JobStage, ResourceClass,
};
use crate::{EditError, Placement, Result, Session};
use ai::transport::{DownloadTransport, HttpRequest, HttpTransport, download_redirect};
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

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Freesound credential is absent")]
    NoCredentials,
    #[error("Freesound authentication failed")]
    Auth,
    #[error("Freesound quota is exhausted")]
    Quota,
    #[error("Freesound rate limit reached; retry after {retry_after_seconds:?} seconds")]
    RateLimit { retry_after_seconds: Option<u32> },
    #[error("Freesound network request failed")]
    Network,
    #[error("Freesound server failed")]
    Server,
    #[error("Freesound response is invalid")]
    InvalidResponse,
    #[error("sound response exceeds the configured limit")]
    Limit,
    #[error("sound audio device is unavailable")]
    NoAudioDevice,
    #[error("sound audio is not cached")]
    NotCached,
    #[error("sound library file is corrupt or unsupported")]
    LibraryCorrupt,
    #[error("sound operation cancelled")]
    Cancelled,
    #[error("sound IO failed")]
    Io(#[from] std::io::Error),
    #[error("sound media failed")]
    Media(#[from] media::MediaError),
    #[error("sound library persistence failed")]
    Project(#[from] project::Error),
    #[error("invalid sound input: {0}")]
    Invalid(&'static str),
}
impl From<ai::Error> for Error {
    fn from(error: ai::Error) -> Self {
        match error {
            ai::Error::Auth => Self::Auth,
            ai::Error::Quota => Self::Quota,
            ai::Error::RateLimit {
                retry_after_seconds,
            } => Self::RateLimit {
                retry_after_seconds,
            },
            ai::Error::Network | ai::Error::Timeout => Self::Network,
            ai::Error::Server => Self::Server,
            ai::Error::Payload => Self::Limit,
            ai::Error::Cancelled => Self::Cancelled,
            _ => Self::InvalidResponse,
        }
    }
}
pub type Sound<T> = std::result::Result<T, Error>;

/// User-supplied credential held only for this process. Debug is always redacted.
#[derive(Clone)]
pub struct Token(String);
impl Token {
    pub fn new(value: impl Into<String>) -> Sound<Self> {
        let value = value.into();
        if value.is_empty()
            || value.trim() != value
            || value.len() > 4096
            || value.bytes().any(|b| b.is_ascii_control() || b == b' ')
        {
            return Err(Error::NoCredentials);
        }
        Ok(Self(value))
    }
    /// The composition root may pass an OS-keychain token here. Only the environment
    /// is read by this contract; keychain persistence stays app-owned.
    pub fn from_env() -> Sound<Option<Self>> {
        match std::env::var("FREESOUND_API_KEY") {
            Ok(value) => Ok(Some(Self::new(value)?)),
            Err(std::env::VarError::NotPresent) => Ok(None),
            Err(std::env::VarError::NotUnicode(_)) => Err(Error::NoCredentials),
        }
    }
    fn expose(&self) -> &str {
        &self.0
    }
}
impl fmt::Debug for Token {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Token([redacted])")
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchRequest {
    pub query: String,
    pub page: u32,
    pub page_size: u32,
    /// Default is commercial-only; false includes NC and legacy licences.
    pub commercial_only: bool,
}
impl Default for SearchRequest {
    fn default() -> Self {
        Self {
            query: String::new(),
            page: 1,
            page_size: 15,
            commercial_only: true,
        }
    }
}
impl SearchRequest {
    fn validate(&self) -> Sound<()> {
        if self.query.len() > MAX_QUERY
            || !(1..=1000).contains(&self.page)
            || !(1..=150).contains(&self.page_size)
        {
            return Err(Error::Invalid("search request"));
        }
        Ok(())
    }
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
impl SoundResult {
    pub fn library_sound(&self) -> LibrarySound {
        LibrarySound {
            id: self.id,
            name: self.name.as_str().into(),
            author: self.author.as_str().into(),
            licence: self.licence.as_str().into(),
            attribution: self.attribution.as_str().into(),
            source_url: self.source_url.clone(),
            audio_url: self.audio_url.clone(),
            downloads: self.downloads,
            duration_ms: self.duration_ms,
        }
    }
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
impl SoundClient {
    pub fn new(token: Token, transport: Arc<dyn HttpTransport>) -> Self {
        Self { token, transport }
    }
    /// Production composition helper; transport policy stays in ai's shared owner.
    pub fn ureq(token: Token) -> Self {
        Self::new(token, Arc::new(ai::transport::UreqTransport::new()))
    }
    pub fn search(&self, request: &SearchRequest) -> Sound<SearchPage> {
        request.validate()?;
        let mut url = format!(
            "{API}?query={}&sort=downloads_desc&page={}&page_size={}&fields={}",
            encode(&request.query),
            request.page,
            request.page_size,
            encode(FIELDS)
        );
        if request.commercial_only {
            url.push_str("&filter=");
            url.push_str(&encode(COMMERCIAL_FILTER));
        }
        let reply = self
            .transport
            .send_bounded(
                HttpRequest {
                    method: "GET",
                    url,
                    headers: vec![(
                        "Authorization".to_owned(),
                        format!("Token {}", self.token.expose()),
                    )],
                    body: Vec::new(),
                },
                MAX_SEARCH_BYTES,
            )
            .map_err(Error::from)?;
        match reply.status {
            200 => parse_page(&reply.body, request),
            401 => Err(Error::Auth),
            402 => Err(Error::Quota),
            403 => Err(Error::Auth),
            404 => Err(Error::InvalidResponse),
            408 | 502 | 503 | 504 => Err(Error::Server),
            429 => Err(Error::RateLimit {
                retry_after_seconds: retry_after(&reply.headers),
            }),
            500..=599 => Err(Error::Server),
            _ => Err(Error::InvalidResponse),
        }
    }
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
fn parse_page(bytes: &[u8], request: &SearchRequest) -> Sound<SearchPage> {
    let raw: RawPage = serde_json::from_slice(bytes).map_err(|_| Error::InvalidResponse)?;
    if raw.results.len() > request.page_size as usize || raw.count > 100_000_000 {
        return Err(Error::InvalidResponse);
    }
    let mut results = Vec::new();
    for sound in raw.results {
        let Some(result) = parse_sound(sound)? else {
            continue;
        };
        // Defence in depth if the server ignored the filter: never present an
        // NC/Sampling+ result under the commercial-only switch.
        if request.commercial_only && !commercial(&result.licence) {
            continue;
        }
        results.push(result);
    }
    Ok(SearchPage {
        count: raw.count,
        page: request.page,
        page_size: request.page_size,
        results,
        has_next: raw.next.is_some(),
        source: SearchSource::Remote,
    })
}
fn parse_sound(raw: RawSound) -> Sound<Option<SoundResult>> {
    if raw.id == 0
        || raw.name.len() > MAX_TEXT
        || raw.username.len() > MAX_TEXT
        || raw.license.len() > MAX_TEXT
        || raw.name.is_empty()
        || raw.username.is_empty()
        || raw.license.is_empty()
        || !url(&raw.url)
    {
        return Err(Error::InvalidResponse);
    }
    let audio_url = raw
        .previews
        .get("preview-hq-mp3")
        .or_else(|| raw.previews.get("preview-lq-mp3"))
        .ok_or(Error::InvalidResponse)?
        .clone();
    if !url(&audio_url) {
        return Err(Error::InvalidResponse);
    }
    let duration = raw.duration.ok_or(Error::InvalidResponse)?;
    if !duration.is_finite() || !(0.0..=86_400.0).contains(&duration) {
        return Err(Error::InvalidResponse);
    }
    let duration_ms = (duration * 1000.0).round() as u64;
    let attribution = format!("{} by {} ({})", raw.name, raw.username, raw.license);
    Ok(Some(SoundResult {
        id: raw.id,
        name: raw.name,
        author: raw.username,
        licence: raw.license,
        attribution,
        source_url: raw.url,
        audio_url,
        duration_ms,
        downloads: raw.num_downloads,
    }))
}
fn url(value: &str) -> bool {
    value.len() <= MAX_URL
        && value.starts_with("https://")
        && !value
            .bytes()
            .any(|b| b.is_ascii_control() || b == b' ' || b == b'"' || b == b'\\')
}
pub fn commercial(licence: &str) -> bool {
    matches!(licence, "Creative Commons 0" | "Attribution")
}
fn retry_after(headers: &[(String, String)]) -> Option<u32> {
    headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("retry-after"))
        .and_then(|(_, value)| value.trim().parse().ok())
}
fn encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(byte));
        } else {
            out.push('%');
            out.push(hex(byte >> 4));
            out.push(hex(byte & 0x0f));
        }
    }
    out
}
fn hex(value: u8) -> char {
    match value & 0x0f {
        0..=9 => char::from(b'0' + (value & 0x0f)),
        _ => char::from(b'A' + ((value & 0x0f) - 10)),
    }
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
    state: Mutex<Vec<SavedSound>>,
}
impl SoundLibrary {
    pub fn open(root: &Path) -> Sound<Self> {
        private_directory(root)?;
        let root = fs::canonicalize(root)?;
        let audio = root.join("audio");
        private_directory(&audio)?;
        let state = Mutex::new(load_library(&root)?);
        Ok(Self { root, audio, state })
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn saved(&self) -> Sound<Vec<SavedSound>> {
        Ok(self
            .state
            .lock()
            .map_err(|_| Error::LibraryCorrupt)?
            .clone())
    }
    pub fn is_saved(&self, id: u64) -> Sound<bool> {
        Ok(self
            .state
            .lock()
            .map_err(|_| Error::LibraryCorrupt)?
            .iter()
            .any(|saved| saved.sound.id == id))
    }
    /// Save is an idempotent upsert. A changed audio URL invalidates the old
    /// content-addressed preview when no other saved sound still cites it.
    pub fn save(&self, sound: &SoundResult) -> Sound<SavedSound> {
        validate_result(sound)?;
        let mut saved = SavedSound {
            sound: sound.clone(),
            saved_at_unix_ms: now_ms(),
        };
        let mut state = self.state.lock().map_err(|_| Error::LibraryCorrupt)?;
        if let Some(existing) = state.iter_mut().find(|item| item.sound.id == sound.id) {
            saved.saved_at_unix_ms = saved.saved_at_unix_ms.max(existing.saved_at_unix_ms);
            let old_url = existing.sound.audio_url.clone();
            *existing = saved.clone();
            self.persist(&state)?;
            if old_url != sound.audio_url
                && !state.iter().any(|item| item.sound.audio_url == old_url)
            {
                let _ = self.remove_cached(&old_url);
            }
            return Ok(saved);
        }
        if state.len() >= MAX_SAVED {
            return Err(Error::Limit);
        }
        state.push(saved.clone());
        state.sort_by(|a, b| {
            b.saved_at_unix_ms
                .cmp(&a.saved_at_unix_ms)
                .then_with(|| a.sound.id.cmp(&b.sound.id))
        });
        self.persist(&state)?;
        Ok(saved)
    }
    pub fn unsave(&self, id: u64) -> Sound<bool> {
        let mut state = self.state.lock().map_err(|_| Error::LibraryCorrupt)?;
        let before = state.len();
        let removed_url: Option<String> = state
            .iter()
            .find(|item| item.sound.id == id)
            .map(|item| item.sound.audio_url.clone());
        state.retain(|item| item.sound.id != id);
        let changed = state.len() != before;
        if changed {
            self.persist(&state)?;
            if let Some(url) = removed_url
                && !state.iter().any(|item| item.sound.audio_url == url)
            {
                let _ = self.remove_cached(&url);
            }
        }
        Ok(changed)
    }
    /// Clear saved sounds and every cached preview. The directory remains usable.
    pub fn clear_all(&self) -> Sound<usize> {
        let mut state = self.state.lock().map_err(|_| Error::LibraryCorrupt)?;
        let removed = state.len();
        state.clear();
        self.persist(&state)?;
        for entry in fs::read_dir(&self.audio)? {
            let entry = entry?;
            if entry.file_type()?.is_file() {
                let _ = fs::remove_file(entry.path());
            }
        }
        Ok(removed)
    }
    pub fn search_local(&self, request: &SearchRequest) -> Sound<SearchPage> {
        request.validate()?;
        let needle = request.query.to_lowercase();
        let mut sounds: Vec<SoundResult> = self
            .saved()?
            .into_iter()
            .map(|saved| saved.sound)
            .filter(|sound| !request.commercial_only || commercial(&sound.licence))
            .filter(|sound| {
                needle.is_empty()
                    || sound.name.to_lowercase().contains(&needle)
                    || sound.author.to_lowercase().contains(&needle)
            })
            .collect();
        sounds.sort_by(|a, b| b.downloads.cmp(&a.downloads).then_with(|| a.id.cmp(&b.id)));
        let count = sounds.len() as u64;
        let start = usize::try_from((request.page - 1).saturating_mul(request.page_size))
            .map_err(|_| Error::Limit)?;
        let results: Vec<_> = sounds
            .into_iter()
            .skip(start)
            .take(request.page_size as usize)
            .collect();
        let has_next = start.saturating_add(results.len()) < count as usize;
        Ok(SearchPage {
            count,
            page: request.page,
            page_size: request.page_size,
            results,
            has_next,
            source: SearchSource::Local,
        })
    }
    pub fn cache_path(&self, url: &str) -> PathBuf {
        self.audio.join(format!("{}.audio", cache_key(url)))
    }
    pub fn cached(&self, url: &str) -> Sound<Option<CachedSound>> {
        let path = self.cache_path(url);
        let meta = match fs::symlink_metadata(&path) {
            Ok(meta) => meta,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        if !meta.file_type().is_file() || meta.file_type().is_symlink() || meta.len() == 0 {
            return Err(Error::LibraryCorrupt);
        }
        if meta.len() > MAX_AUDIO_BYTES {
            return Err(Error::Limit);
        }
        cached_from_path(path, Some(url)).map(Some)
    }
    /// Store already downloaded preview bytes. Only complete, probed audio is
    /// published; a failed probe leaves no cache entry.
    pub fn store_cached(&self, source: &str, bytes: &[u8]) -> Sound<CachedSound> {
        if !url(source) || bytes.is_empty() || bytes.len() as u64 > MAX_AUDIO_BYTES {
            return Err(Error::Invalid("cached audio"));
        }
        let path = self.cache_path(source);
        let temp = self.audio.join(format!(
            ".part-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let result = (|| {
            let mut options = fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options.open(&temp)?;
            file.write_all(bytes)?;
            file.sync_all()?;
            drop(file);
            let cached = cached_from_path(temp.clone(), Some(source))?;
            if cached.probe.duration.is_none_or(|d| d <= Tick::ZERO)
                || !cached
                    .probe
                    .streams
                    .iter()
                    .any(|s| s.kind == media::StreamKind::Audio)
            {
                return Err(Error::InvalidResponse);
            }
            fs::rename(&temp, &path)?;
            cached_from_path(path, Some(source))
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temp);
        }
        result
    }
    fn remove_cached(&self, url: &str) -> Sound<()> {
        match fs::remove_file(self.cache_path(url)) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
    fn persist(&self, state: &[SavedSound]) -> Sound<()> {
        let library = StoredLibrary {
            format: LIBRARY_FORMAT.into(),
            version: LIBRARY_VERSION,
            sounds: state.to_vec(),
        };
        let bytes = serde_json::to_vec(&library).map_err(|_| Error::LibraryCorrupt)?;
        if bytes.len() as u64 > MAX_LIBRARY_BYTES {
            return Err(Error::Limit);
        }
        project::atomic_write(&self.root.join("library.json"), &bytes)?;
        Ok(())
    }
}
fn load_library(root: &Path) -> Sound<Vec<SavedSound>> {
    let path = root.join("library.json");
    let meta = match fs::symlink_metadata(&path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    if !meta.file_type().is_file() || meta.len() > MAX_LIBRARY_BYTES {
        return Err(Error::LibraryCorrupt);
    }
    let bytes = fs::read(&path)?;
    let stored: StoredLibrary =
        serde_json::from_slice(&bytes).map_err(|_| Error::LibraryCorrupt)?;
    if stored.format != LIBRARY_FORMAT
        || stored.version != LIBRARY_VERSION
        || stored.sounds.len() > MAX_SAVED
    {
        return Err(Error::LibraryCorrupt);
    }
    for saved in &stored.sounds {
        validate_result(&saved.sound)?;
    }
    Ok(stored.sounds)
}
fn validate_result(sound: &SoundResult) -> Sound<()> {
    if sound.id == 0
        || sound.name.is_empty()
        || sound.author.is_empty()
        || sound.licence.is_empty()
        || sound.attribution.is_empty()
        || sound.name.len() > MAX_TEXT
        || sound.author.len() > MAX_TEXT
        || sound.licence.len() > MAX_TEXT
        || sound.attribution.len() > MAX_TEXT
        || !url(&sound.source_url)
        || !url(&sound.audio_url)
        || sound.duration_ms == 0
        || sound.duration_ms > 86_400_000
    {
        return Err(Error::InvalidResponse);
    }
    Ok(())
}
fn cache_key(url: &str) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"vibecut-sound-v1");
    hasher.update(url.as_bytes());
    hasher.finalize().to_hex().to_string()
}
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| u64::try_from(d.as_millis()).ok())
        .unwrap_or(0)
}
fn private_directory(path: &Path) -> std::io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(path)
}
static NEXT: AtomicU64 = AtomicU64::new(0);

/// A durable cached preview with the probed media identity needed for a project asset.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CachedSound {
    pub path: PathBuf,
    pub content_hash: ContentHash,
    pub probe: SourceProbe,
}
impl CachedSound {
    pub fn duration(&self) -> Option<Tick> {
        self.probe
            .duration
            .filter(|duration| *duration > Tick::ZERO)
    }
    pub fn asset(&self, id: media::AssetId) -> Result<SourceAsset> {
        if !self
            .probe
            .streams
            .iter()
            .any(|s| s.kind == media::StreamKind::Audio)
        {
            return Err(Error::InvalidResponse.into());
        }
        let asset = SourceAsset::try_from(AssetData {
            ephemeral: false,
            id,
            path: self.path.clone(),
            content_hash: self.content_hash,
            probe: self.probe.clone(),
        })?;
        Ok(asset)
    }
    pub fn bytes(&self) -> Sound<Vec<u8>> {
        let meta = fs::metadata(&self.path)?;
        if !meta.is_file() || meta.len() == 0 || meta.len() > MAX_AUDIO_BYTES {
            return Err(Error::Limit);
        }
        Ok(fs::read(&self.path)?)
    }
}
fn cached_from_path(path: PathBuf, source: Option<&str>) -> Sound<CachedSound> {
    if let Some(source) = source
        && !url(source)
    {
        return Err(Error::Invalid("cached audio URL"));
    }
    let hash = ContentHash::read_with_cancel(&path, || false)?;
    let probe = media::probe(&path)?;
    Ok(CachedSound {
        path,
        content_hash: hash,
        probe: SourceProbe::from(&probe),
    })
}

fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}
fn decimal(value: &str) -> Option<u64> {
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
        None
    } else {
        value.parse().ok()
    }
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
impl SoundDownloads {
    pub fn new(library: Arc<SoundLibrary>, transport: Arc<dyn DownloadTransport>) -> Self {
        Self {
            library,
            transport,
            max_bytes: MAX_AUDIO_BYTES,
            busy: Arc::new(AtomicBool::new(false)),
        }
    }
    /// Production composition helper. Downloads carry no credential.
    pub fn ureq(library: Arc<SoundLibrary>) -> Self {
        Self::new(library, Arc::new(ai::transport::UreqTransport::new()))
    }
    pub fn library(&self) -> &Arc<SoundLibrary> {
        &self.library
    }
    /// Submit one cache download through the shared runner. There is one active
    /// download per library; the runner owns retries, backoff and queue bounds.
    pub fn submit(
        &self,
        runner: &mut JobRunner,
        source: &str,
    ) -> crate::Result<(JobId, CancelToken)> {
        if !url(source) {
            return Err(Error::Invalid("sound URL").into());
        }
        self.busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| EditError::CacheBusy)?;
        let guard = Busy(self.busy.clone());
        let key = CacheKey::new(KeyInputs {
            kind: ArtifactKind::SoundAudio,
            source: *blake3::hash(source.as_bytes()).as_bytes(),
            provider: "freesound",
            model: "preview",
            version: "1",
            settings: &[],
            transcript: [0; 32],
            cues: [0; 32],
            clip_range: [0; 32],
            translation: [0; 32],
        })?;
        let worker = SoundDownloads {
            library: self.library.clone(),
            transport: self.transport.clone(),
            max_bytes: self.max_bytes,
            busy: Arc::new(AtomicBool::new(false)),
        };
        let source = source.to_owned();
        runner.submit(
            JobSpec {
                kind: JobKind::SoundDownload,
                class: ResourceClass::Network,
                input: key,
                bytes: JobBytes::default(),
            },
            move |ctx| {
                let _guard = &guard;
                ctx.check_cancelled()?;
                match worker.download(&source, &|| ctx.cancelled(), &|done, total| {
                    let fraction = total
                        .filter(|total| *total > 0)
                        .map_or(0.0, |total| (done as f64 / total as f64) as f32);
                    ctx.progress(JobStage::Download, fraction).ok();
                }) {
                    Ok(_) => Ok(Vec::new()),
                    Err(Error::Network) => Err(EditError::NetworkTransient),
                    Err(error) => Err(error.into()),
                }
            },
        )
    }
    /// Download through bounded HTTPS redirects into the library cache. Cancellation
    /// is checked before every read and write and leaves no partial cache entry.
    pub fn download(
        &self,
        source: &str,
        cancelled: &dyn Fn() -> bool,
        progress: &dyn Fn(u64, Option<u64>),
    ) -> Sound<CachedSound> {
        if !url(source) {
            return Err(Error::Invalid("sound URL"));
        }
        let mut target = source.to_owned();
        for redirect in 0..=5 {
            if cancelled() {
                return Err(Error::Cancelled);
            }
            let reply = self.transport.get(&target, &[]).map_err(Error::from)?;
            if matches!(reply.status, 301 | 302 | 303 | 307 | 308) {
                if redirect == 5 {
                    return Err(Error::InvalidResponse);
                }
                let location = header(&reply.headers, "location").ok_or(Error::InvalidResponse)?;
                target = download_redirect(&target, location).map_err(Error::from)?;
                continue;
            }
            match reply.status {
                200 => {}
                401 | 403 => return Err(Error::Auth),
                402 => return Err(Error::Quota),
                404 => return Err(Error::InvalidResponse),
                429 => {
                    return Err(Error::RateLimit {
                        retry_after_seconds: retry_after(&reply.headers),
                    });
                }
                408 | 500..=599 => return Err(Error::Network),
                _ => return Err(Error::InvalidResponse),
            }
            let expected = match header(&reply.headers, "content-length") {
                Some(value) => {
                    let size = decimal(value).ok_or(Error::InvalidResponse)?;
                    if size == 0 || size > self.max_bytes {
                        return Err(Error::Limit);
                    }
                    Some(size)
                }
                None => None,
            };
            let temp = self.library.audio.join(format!(
                ".part-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let result = (|| {
                let mut options = fs::OpenOptions::new();
                options.write(true).create_new(true);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::OpenOptionsExt;
                    options.mode(0o600);
                }
                let mut file = options.open(&temp)?;
                let mut body = reply.body;
                let mut total = 0u64;
                let mut buffer = [0u8; 65536];
                loop {
                    if cancelled() {
                        return Err(Error::Cancelled);
                    }
                    let read = match body.read(&mut buffer) {
                        Ok(0) => break,
                        Ok(read) => read,
                        Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                        Err(_) => return Err(Error::Network),
                    };
                    total = total.checked_add(read as u64).ok_or(Error::Limit)?;
                    if total > self.max_bytes {
                        return Err(Error::Limit);
                    }
                    file.write_all(buffer.get(..read).ok_or(Error::Limit)?)?;
                    progress(total, expected);
                }
                if total == 0 || expected.is_some_and(|size| size != total) {
                    return Err(Error::InvalidResponse);
                }
                file.sync_all()?;
                drop(file);
                self.library.publish_temp(source, &temp)
            })();
            if result.is_err() {
                let _ = fs::remove_file(&temp);
            }
            return result;
        }
        Err(Error::InvalidResponse)
    }
}
impl SoundLibrary {
    fn publish_temp(&self, source: &str, temp: &Path) -> Sound<CachedSound> {
        let cached = cached_from_path(temp.to_owned(), Some(source))?;
        if cached.probe.duration.is_none_or(|d| d <= Tick::ZERO)
            || !cached
                .probe
                .streams
                .iter()
                .any(|s| s.kind == media::StreamKind::Audio)
        {
            return Err(Error::InvalidResponse);
        }
        fs::rename(temp, self.cache_path(source))?;
        cached_from_path(self.cache_path(source), Some(source))
    }
}

/// App-owned audio device seam. `play` must not return before playback is started;
/// `stop` must be idempotent and safe while `play` is still decoding.
pub struct PreviewClip<'a> {
    pub name: &'a str,
    pub audio_url: &'a str,
    pub bytes: &'a [u8],
    pub cancelled: &'a dyn Fn() -> bool,
}
pub trait PreviewSink: Send + Sync {
    fn play(&self, clip: PreviewClip<'_>) -> Sound<()>;
    fn stop(&self) -> Sound<()>;
}
/// One preview at a time. Stop cancels an in-flight download and delegates to the
/// sink; a missing device is a typed `NoAudioDevice` from the sink.
pub struct SoundPreview {
    downloads: Arc<SoundDownloads>,
    sink: Arc<dyn PreviewSink>,
    active: Mutex<Option<Arc<AtomicBool>>>,
}
impl SoundPreview {
    pub fn new(downloads: Arc<SoundDownloads>, sink: Arc<dyn PreviewSink>) -> Self {
        Self {
            downloads,
            sink,
            active: Mutex::new(None),
        }
    }
    pub fn play(&self, sound: &SoundResult) -> Sound<()> {
        validate_result(sound)?;
        let cancel = Arc::new(AtomicBool::new(false));
        {
            let mut active = self.active.lock().map_err(|_| Error::LibraryCorrupt)?;
            if let Some(old) = active.replace(cancel.clone()) {
                old.store(true, Ordering::Release);
            }
        }
        let result = (|| {
            let cached = match self.downloads.library.cached(&sound.audio_url)? {
                Some(cached) => cached,
                None => self.downloads.download(
                    &sound.audio_url,
                    &|| cancel.load(Ordering::Acquire),
                    &|_, _| {},
                )?,
            };
            if cancel.load(Ordering::Acquire) {
                return Err(Error::Cancelled);
            }
            let bytes = cached.bytes()?;
            self.sink.play(PreviewClip {
                name: &sound.name,
                audio_url: &sound.audio_url,
                bytes: &bytes,
                cancelled: &|| cancel.load(Ordering::Acquire),
            })
        })();
        let mut active = self.active.lock().map_err(|_| Error::LibraryCorrupt)?;
        if active
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, &cancel))
        {
            *active = None;
        }
        result
    }
    pub fn stop(&self) -> Sound<()> {
        if let Some(cancel) = self
            .active
            .lock()
            .map_err(|_| Error::LibraryCorrupt)?
            .take()
        {
            cancel.store(true, Ordering::Release);
        }
        self.sink.stop()
    }
    pub fn cancel(&self) {
        if let Ok(mut active) = self.active.lock()
            && let Some(cancel) = active.take()
        {
            cancel.store(true, Ordering::Release);
        }
    }
}

/// Attach a cached library preview as one audio item and one asset in one undo step.
pub fn add_to_timeline(
    session: &mut Session,
    sound: &SoundResult,
    cached: &CachedSound,
    placement: Placement,
    start: Tick,
) -> Result<crate::Outcome> {
    validate_result(sound)?;
    let id = next_asset_id(session)?;
    let asset = cached.asset(id)?;
    session.apply(crate::Command::AddLibrarySound {
        placement,
        start,
        asset,
        sound: sound.library_sound(),
    })
}
/// Offline add: the saved record must have a cached preview for its exact URL.
pub fn add_saved(
    session: &mut Session,
    library: &SoundLibrary,
    id: u64,
    placement: Placement,
    start: Tick,
) -> Result<crate::Outcome> {
    let sound = library
        .saved()?
        .into_iter()
        .find(|saved| saved.sound.id == id)
        .map(|saved| saved.sound)
        .ok_or(Error::Invalid("saved sound"))?;
    let cached = library.cached(&sound.audio_url)?.ok_or(Error::NotCached)?;
    add_to_timeline(session, &sound, &cached, placement, start)
}
fn next_asset_id(session: &Session) -> Result<media::AssetId> {
    let next = session
        .project()
        .data()
        .assets
        .iter()
        .map(|asset| asset.id().0)
        .max()
        .map_or(Some(0), |id| id.checked_add(1))
        .ok_or(EditError::IdExhausted)?;
    Ok(media::AssetId(next))
}

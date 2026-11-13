//! Owns the Sounds tab's saved library, provider search, cache jobs and preview.
//!
//! Provider, cache, persistence and command behaviour stay in `libraries::sounds`;
//! this module only presents them and owns the app's device sink composition.

use crate::{Error, Result};
use engine::SoundError;
use engine::{
    ByteBudget,
    control::Error as EngineError,
    jobs::{CancelToken, JobId, JobRunner, JobState, RunnerConfig, SystemClock},
};
use libraries::sounds::{
    CachedSound, SavedSound, SearchPage, SearchRequest, SearchSource, SoundClient, SoundDownloads,
    SoundLibrary, SoundPreview, SoundResult, Token,
};
use std::{
    collections::BTreeSet,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, TryRecvError},
    },
    time::Duration,
};
use transport::HttpTransport;

pub mod music;
pub(super) mod sink;

const PAGE_SIZE: u32 = 30;

/// The two Sounds views. Saved is the offline library; Discover queries the
/// provider when a credential exists and otherwise shows the local library.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SoundsMode {
    #[default]
    Discover,
    Saved,
}

impl SoundsMode {
    pub const fn id(self) -> &'static str {
        match self {
            Self::Discover => "discover",
            Self::Saved => "saved",
        }
    }
}

/// A creator-facing failure, independent of engine's internal error tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SoundsProblem {
    Auth,
    Quota,
    RateLimit(Option<u32>),
    Network,
    Server,
    Invalid,
    Limit,
    NoAudioDevice,
    NotCached,
    Library,
    CacheBusy,
    CacheFailed,
    Publication,
    Cancelled,
}

impl SoundsProblem {
    fn from_error(error: &SoundError) -> Self {
        match error {
            SoundError::NoCredentials | SoundError::Auth => Self::Auth,
            SoundError::Quota => Self::Quota,
            SoundError::RateLimit {
                retry_after_seconds,
            } => Self::RateLimit(*retry_after_seconds),
            SoundError::Network => Self::Network,
            SoundError::Server => Self::Server,
            SoundError::InvalidResponse => Self::Invalid,
            SoundError::Limit => Self::Limit,
            SoundError::NoAudioDevice => Self::NoAudioDevice,
            SoundError::NotCached => Self::NotCached,
            SoundError::LibraryCorrupt => Self::Library,
            SoundError::Cancelled => Self::Cancelled,
            SoundError::Io(_) | SoundError::Project(_) | SoundError::Media(_) => Self::CacheFailed,
            SoundError::Invalid(_) => Self::Invalid,
        }
    }

    pub fn from_edit(error: &EngineError) -> Self {
        match error {
            EngineError::Sounds(error) => Self::from_error(error),
            EngineError::Runtime(engine::RuntimeError::NetworkTransient) => Self::Network,
            EngineError::Runtime(engine::RuntimeError::CacheBusy) => Self::CacheBusy,
            _ => Self::CacheFailed,
        }
    }

    pub fn from_app(error: &Error) -> Self {
        match error {
            Error::Engine(error) => Self::from_edit(error),
            _ => Self::CacheFailed,
        }
    }
}

/// Why a library store did not open. The typed cause stays with the lending
/// owner; the section shows only this reason.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LibraryProblem {
    /// Another writer holds the library's lease.
    Busy,
    /// The library file is damaged or unsupported.
    Corrupt,
    /// Reading or creating the library files failed.
    Io,
}

impl LibraryProblem {
    pub fn from_sound(error: &SoundError) -> Self {
        match error {
            SoundError::Project(project::Error::Busy) => Self::Busy,
            SoundError::LibraryCorrupt => Self::Corrupt,
            _ => Self::Io,
        }
    }

    pub fn from_music(error: &engine::MusicError) -> Self {
        match error {
            engine::MusicError::Project(project::Error::Busy) => Self::Busy,
            engine::MusicError::LibraryCorrupt => Self::Corrupt,
            _ => Self::Io,
        }
    }
}

/// A library section that can ask the lending owner to open its store again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LibraryStore {
    Sounds,
    Music,
}

/// A provider client plus its transport. Tests inject a fixture transport.
pub struct Client {
    token: Token,
    transport: Arc<dyn HttpTransport>,
}

impl Client {
    pub fn new(token: Token, transport: Arc<dyn HttpTransport>) -> Self {
        Self { token, transport }
    }
}

impl Clone for Client {
    fn clone(&self) -> Self {
        Self {
            token: self.token.clone(),
            transport: self.transport.clone(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CacheView {
    pub sound: u64,
    pub progress: f32,
    pub cancelling: bool,
}

/// Read-only projection consumed by the assets view and inspection.
#[derive(Clone, Debug)]
pub struct SoundsView {
    pub mode: SoundsMode,
    pub commercial_only: bool,
    pub searching: bool,
    pub has_token: bool,
    pub source: SearchSource,
    pub results: Vec<SoundResult>,
    pub saved: Vec<SavedSound>,
    pub saved_ids: BTreeSet<u64>,
    pub page: u32,
    pub count: u64,
    pub has_next: bool,
    pub previewing: Option<u64>,
    pub search_problem: Option<SoundsProblem>,
    pub preview_problem: Option<SoundsProblem>,
    pub cache_problem: Option<SoundsProblem>,
    pub saved_problem: Option<SoundsProblem>,
    pub cache: Option<CacheView>,
}

struct CacheJob {
    id: JobId,
    cancel: CancelToken,
    sound: SoundResult,
    add: bool,
    progress: f32,
    cancelling: bool,
}

/// One editor's Sounds presentation over the app-shared saved library.
pub struct Sounds {
    library: Arc<SoundLibrary>,
    client: Option<Client>,
    downloads: Arc<SoundDownloads>,
    preview: Arc<SoundPreview>,
    runner: JobRunner,
    mode: SoundsMode,
    commercial_only: bool,
    query: String,
    results: Vec<SoundResult>,
    source: SearchSource,
    page: u32,
    count: u64,
    has_next: bool,
    searching: bool,
    search_generation: u64,
    search_rx: Option<Receiver<(u64, std::result::Result<SearchPage, SoundError>)>>,
    search_problem: Option<SoundsProblem>,
    saved: Vec<SavedSound>,
    saved_ids: BTreeSet<u64>,
    saved_problem: Option<SoundsProblem>,
    previewing: Option<u64>,
    preview_rx: Option<Receiver<std::result::Result<(), SoundError>>>,
    preview_worker: Option<std::thread::JoinHandle<()>>,
    preview_pending: Option<SoundResult>,
    preview_cancel: Option<Arc<AtomicBool>>,
    preview_problem: Option<SoundsProblem>,
    cache: Option<CacheJob>,
    cache_problem: Option<SoundsProblem>,
    ready_add: Option<SoundResult>,
}

impl Sounds {
    /// Production composition: environment credential when present, direct
    /// provider downloads and the app-owned cpal preview sink.
    pub fn new(library: Arc<SoundLibrary>, budget: ByteBudget) -> Result<Self> {
        let client = Token::from_env()
            .ok()
            .flatten()
            .map(|token| Client::new(token, Arc::new(transport::UreqTransport::new())));
        let downloads = Arc::new(SoundDownloads::ureq(library.clone()));
        let preview = Arc::new(SoundPreview::new(
            downloads.clone(),
            Arc::new(sink::OutputSink::new(budget.clone())),
        ));
        Self::with_parts(library, client, downloads, preview, budget)
    }

    /// Injection boundary for tests and audit scenes. The caller owns the
    /// library, both transports and the preview sink.
    pub fn with_parts(
        library: Arc<SoundLibrary>,
        client: Option<Client>,
        downloads: Arc<SoundDownloads>,
        preview: Arc<SoundPreview>,
        budget: ByteBudget,
    ) -> Result<Self> {
        let runner = JobRunner::new(
            RunnerConfig {
                concurrency: [1, 1, 1],
                queue: 4,
                retained: 8,
                network_attempts: 1,
                backoff: Duration::from_millis(250),
                progress_interval: Duration::from_millis(20),
            },
            budget,
            Arc::new(SystemClock::default()),
        )?;
        let saved = library
            .saved()
            .map_err(|_| Error::Invalid("saved sounds"))?;
        let saved_ids = saved.iter().map(|item| item.sound.id).collect();
        Ok(Self {
            library,
            client,
            downloads,
            preview,
            runner,
            mode: SoundsMode::Discover,
            commercial_only: true,
            query: String::new(),
            results: Vec::new(),
            source: SearchSource::Local,
            page: 1,
            count: 0,
            has_next: false,
            searching: false,
            search_generation: 0,
            search_rx: None,
            search_problem: None,
            saved,
            saved_ids,
            saved_problem: None,
            previewing: None,
            preview_rx: None,
            preview_worker: None,
            preview_pending: None,
            preview_cancel: None,
            preview_problem: None,
            cache: None,
            cache_problem: None,
            ready_add: None,
        })
    }

    pub fn mode(&self) -> SoundsMode {
        self.mode
    }

    pub fn commercial_only(&self) -> bool {
        self.commercial_only
    }

    pub fn has_token(&self) -> bool {
        self.client.is_some()
    }

    pub fn set_mode(&mut self, mode: SoundsMode) {
        self.mode = mode;
    }

    pub fn set_commercial_only(&mut self, commercial_only: bool) {
        self.commercial_only = commercial_only;
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    /// Start a page-one search, or append the next page when `more` is set.
    pub fn search(&mut self, query: &str, commercial_only: bool, mode: SoundsMode, more: bool) {
        self.mode = mode;
        self.commercial_only = commercial_only;
        self.query = query.to_owned();
        let page = if more { self.page.saturating_add(1) } else { 1 };
        let request = SearchRequest {
            query: query.to_owned(),
            page,
            page_size: PAGE_SIZE,
            commercial_only,
        };
        self.search_problem = None;
        if mode == SoundsMode::Saved || self.client.is_none() {
            self.apply_page(
                self.library.search_local(&request),
                more,
                SearchSource::Local,
            );
            return;
        }
        let Some(client) = self.client.clone() else {
            return;
        };
        let (tx, rx) = mpsc::channel();
        self.search_generation = self.search_generation.wrapping_add(1);
        let generation = self.search_generation;
        let spawned = std::thread::Builder::new()
            .name("vibecut-sounds-search".into())
            .spawn(move || {
                let client = SoundClient::new(client.token, client.transport);
                let _ = tx.send((generation, client.search(&request)));
            });
        match spawned {
            Ok(_) => {
                self.searching = true;
                self.search_rx = Some(rx);
            }
            Err(_) => {
                self.searching = false;
                self.search_problem = Some(SoundsProblem::Invalid);
            }
        }
    }

    pub fn search_more(&mut self) {
        if !self.has_next || self.searching {
            return;
        }
        let (query, commercial, mode) = (self.query.clone(), self.commercial_only, self.mode);
        self.search(&query, commercial, mode, true);
    }

    fn apply_page(
        &mut self,
        page: std::result::Result<SearchPage, SoundError>,
        append: bool,
        source: SearchSource,
    ) {
        self.searching = false;
        match page {
            Ok(page) => {
                if append {
                    self.results.extend(page.results);
                } else {
                    self.results = page.results;
                }
                self.page = page.page;
                self.count = page.count;
                self.has_next = page.has_next;
                self.source = if page.source == SearchSource::Remote {
                    SearchSource::Remote
                } else {
                    source
                };
            }
            Err(error) => {
                self.search_problem = Some(SoundsProblem::from_error(&error));
            }
        }
    }

    pub fn refresh_saved(&mut self) {
        match self.library.saved() {
            Ok(saved) => {
                self.saved_ids = saved.iter().map(|item| item.sound.id).collect();
                self.saved = saved;
                self.saved_problem = None;
            }
            Err(_) => self.saved_problem = Some(SoundsProblem::Library),
        }
    }

    pub fn sound(&self, id: u64) -> Option<SoundResult> {
        self.find(id)
    }

    fn find(&self, id: u64) -> Option<SoundResult> {
        self.results
            .iter()
            .chain(self.saved.iter().map(|saved| &saved.sound))
            .find(|sound| sound.id == id)
            .cloned()
    }

    /// Save or unsave one result; the local library owns both operations.
    pub fn toggle_saved(&mut self, id: u64) {
        let Some(sound) = self.find(id) else {
            self.saved_problem = Some(SoundsProblem::Library);
            return;
        };
        let result = if self.saved_ids.contains(&id) {
            self.library.unsave(id).map(|_| ())
        } else {
            self.library.save(&sound).map(|_| ())
        };
        match result {
            Ok(()) => {
                self.saved_problem = None;
                self.refresh_saved();
                if self.mode == SoundsMode::Saved {
                    let request = SearchRequest {
                        query: self.query.clone(),
                        page: 1,
                        page_size: PAGE_SIZE,
                        commercial_only: self.commercial_only,
                    };
                    self.apply_page(
                        self.library.search_local(&request),
                        false,
                        SearchSource::Local,
                    );
                }
            }
            Err(error) => self.saved_problem = Some(SoundsProblem::from_error(&error)),
        }
    }

    pub fn play(&mut self, id: u64) {
        if self.previewing == Some(id) {
            self.stop();
            return;
        }
        let Some(sound) = self.find(id) else {
            self.preview_problem = Some(SoundsProblem::Library);
            return;
        };
        if self.preview_worker.is_some() {
            self.stop();
        }
        self.previewing = Some(id);
        self.preview_problem = None;
        self.preview_pending = Some(sound);
        self.start_preview();
    }

    fn start_preview(&mut self) {
        if self.preview_worker.is_some() {
            return;
        }
        let Some(sound) = self.preview_pending.take() else {
            return;
        };
        let preview = self.preview.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        self.preview_cancel = Some(cancel.clone());
        let (tx, rx) = mpsc::sync_channel(1);
        let spawned = std::thread::Builder::new()
            .name("vibecut-sounds-preview".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    preview.play(&sound, &|| cancel.load(Ordering::Acquire))
                }))
                .unwrap_or(Err(SoundError::NoAudioDevice));
                let _ = tx.send(result);
            });
        match spawned {
            Ok(worker) => {
                self.preview_worker = Some(worker);
                self.preview_rx = Some(rx);
            }
            Err(_) => {
                self.previewing = None;
                self.preview_problem = Some(SoundsProblem::NoAudioDevice);
            }
        }
    }

    pub fn stop(&mut self) {
        if let Some(cancel) = self.preview_cancel.take() {
            cancel.store(true, Ordering::Release);
        }
        let _ = self.preview.stop();
        self.previewing = None;
        self.preview_pending = None;
    }

    /// Queue a cache job for an add that has no local audio yet. The editor
    /// consumes `take_ready_add` after the job publishes complete audio.
    pub fn request_add(&mut self, sound: SoundResult) {
        if self.cache.is_some() {
            self.cache_problem = Some(SoundsProblem::CacheBusy);
            return;
        }
        match self.library.cached(&sound.audio_url) {
            Ok(Some(_)) => {
                self.cache_problem = None;
                self.ready_add = Some(sound);
                return;
            }
            Ok(None) => {}
            Err(error) => {
                self.cache_problem = Some(SoundsProblem::from_error(&error));
                return;
            }
        }
        match self.downloads.submit(&mut self.runner, &sound.audio_url) {
            Ok((id, cancel)) => {
                self.cache_problem = None;
                self.cache = Some(CacheJob {
                    id,
                    cancel,
                    sound,
                    add: true,
                    progress: 0.0,
                    cancelling: false,
                });
            }
            Err(error) => self.cache_problem = Some(SoundsProblem::from_edit(&error)),
        }
    }

    pub fn cancel_cache(&mut self) {
        if let Some(cache) = &mut self.cache {
            cache.cancel.cancel();
            cache.cancelling = true;
        }
    }

    pub fn take_ready_add(&mut self) -> Option<SoundResult> {
        self.ready_add.take()
    }

    pub fn add_failed(&mut self, problem: SoundsProblem) {
        self.cache_problem = Some(problem);
    }

    pub fn cached(&self, sound: &SoundResult) -> Option<CachedSound> {
        self.library.cached(&sound.audio_url).ok().flatten()
    }

    pub fn clear_problems(&mut self) {
        self.search_problem = None;
        self.preview_problem = None;
        self.cache_problem = None;
        self.saved_problem = None;
    }

    /// Pump jobs and retain only terminal truth the panel can show.
    pub fn tick(&mut self) -> Result<()> {
        if let Err(error) = self.runner.tick() {
            self.cache_problem = Some(SoundsProblem::from_edit(&error.into()));
            self.cache = None;
        }
        if let Some(rx) = &self.search_rx {
            match rx.try_recv() {
                Ok((generation, page)) => {
                    if generation == self.search_generation {
                        self.search_rx = None;
                        let append = page.as_ref().is_ok_and(|page| page.page > 1);
                        self.apply_page(page, append, SearchSource::Local);
                    }
                }
                Err(TryRecvError::Disconnected) => {
                    self.search_rx = None;
                    self.searching = false;
                    self.search_problem = Some(SoundsProblem::Invalid);
                }
                Err(TryRecvError::Empty) => {}
            }
        }
        if self
            .preview_worker
            .as_ref()
            .is_some_and(|worker| worker.is_finished())
        {
            if let Some(worker) = self.preview_worker.take() {
                let _ = worker.join(); // Entry catches panics into the terminal result.
            }
            if let Some(rx) = self.preview_rx.take()
                && self
                    .preview_cancel
                    .take()
                    .is_some_and(|cancel| !cancel.load(Ordering::Acquire))
            {
                match rx.try_recv() {
                    Ok(result) => {
                        self.previewing = None;
                        if let Err(error) = result
                            && !matches!(error, SoundError::Cancelled)
                        {
                            self.preview_problem = Some(SoundsProblem::from_error(&error));
                        }
                    }
                    Err(TryRecvError::Disconnected) => {
                        self.previewing = None;
                        self.preview_problem = Some(SoundsProblem::NoAudioDevice);
                    }
                    Err(TryRecvError::Empty) => {}
                }
            }
            self.start_preview();
        }
        let (sound, add, cancelling, state, error, progress) = {
            let Some(cache) = self.cache.as_ref() else {
                return Ok(());
            };
            let Some(snapshot) = self.runner.snapshot(cache.id) else {
                return Ok(());
            };
            (
                cache.sound.clone(),
                cache.add,
                cache.cancelling,
                snapshot.state,
                snapshot.error.clone(),
                snapshot.progress.clamp(0.0, 1.0),
            )
        };
        if let Some(cache) = &mut self.cache {
            cache.progress = progress;
        }
        match state {
            JobState::Completed => {
                self.cache = None;
                match self.library.cached(&sound.audio_url) {
                    Ok(Some(_)) => {
                        if add {
                            self.ready_add = Some(sound);
                        }
                    }
                    Ok(None) | Err(_) => {
                        self.cache_problem = Some(SoundsProblem::CacheFailed);
                    }
                }
            }
            JobState::Failed | JobState::Cancelled => {
                self.cache = None;
                if !cancelling && state == JobState::Failed {
                    self.cache_problem = Some(
                        error
                            .as_deref()
                            .and_then(|error| {
                                (error as &(dyn std::error::Error + 'static))
                                    .downcast_ref::<EngineError>()
                            })
                            .map(SoundsProblem::from_edit)
                            .filter(|problem| !matches!(problem, SoundsProblem::Cancelled))
                            .unwrap_or(SoundsProblem::CacheFailed),
                    );
                }
            }
            JobState::Queued
            | JobState::Running
            | JobState::Cancelling
            | JobState::Publishing
            | JobState::Backoff => {}
        }
        Ok(())
    }

    pub fn view(&self) -> SoundsView {
        SoundsView {
            mode: self.mode,
            commercial_only: self.commercial_only,
            searching: self.searching,
            has_token: self.client.is_some(),
            source: self.source,
            results: self.results.clone(),
            saved: self.saved.clone(),
            saved_ids: self.saved_ids.clone(),
            page: self.page,
            count: self.count,
            has_next: self.has_next,
            previewing: self.previewing,
            search_problem: self.search_problem,
            preview_problem: self.preview_problem,
            cache_problem: self.cache_problem,
            saved_problem: self.saved_problem,
            cache: self.cache.as_ref().map(|cache| CacheView {
                sound: cache.sound.id,
                progress: cache.progress,
                cancelling: cache.cancelling,
            }),
        }
    }
}

impl Drop for Sounds {
    fn drop(&mut self) {
        if let Some(cache) = &self.cache {
            cache.cancel.cancel();
        }
        self.stop();
        if let Some(worker) = self.preview_worker.take() {
            let _ = worker.join();
        }
        self.runner.shutdown();
    }
}

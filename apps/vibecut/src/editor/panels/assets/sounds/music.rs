//! Owns the Audio tab's Music section: provider search, cache jobs and preview.
//!
//! Provider, cache, persistence and command behaviour stay in `libraries::music`;
//! this module only presents them and owns the app's device sink composition.

use crate::services::music::MusicSession;
use crate::{Error, Result};
use engine::MusicError;
use engine::{
    ByteBudget,
    control::Error as EngineError,
    jobs::{CancelToken, JobId, JobRunner, JobState, RunnerConfig, SystemClock},
};
use libraries::music::{
    CachedMusic, MusicClient, MusicDownloads, MusicPreview, MusicResult, SavedMusic, SearchPage,
    SearchRequest, SearchSource,
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

const PAGE_SIZE: u32 = 15;

/// The two Music views. Saved is the offline library; Browse searches the
/// open source by tag or free text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MusicMode {
    #[default]
    Browse,
    Saved,
}

impl MusicMode {
    pub const fn id(self) -> &'static str {
        match self {
            Self::Browse => "browse",
            Self::Saved => "saved",
        }
    }
}

/// A creator-facing failure, independent of engine's internal error tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MusicProblem {
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

impl MusicProblem {
    fn from_error(error: &MusicError) -> Self {
        match error {
            MusicError::Auth => Self::Auth,
            MusicError::Quota => Self::Quota,
            MusicError::RateLimit {
                retry_after_seconds,
            } => Self::RateLimit(*retry_after_seconds),
            MusicError::Network => Self::Network,
            MusicError::Server => Self::Server,
            MusicError::InvalidResponse => Self::Invalid,
            MusicError::Limit => Self::Limit,
            MusicError::NoAudioDevice => Self::NoAudioDevice,
            MusicError::NotCached => Self::NotCached,
            MusicError::LibraryCorrupt => Self::Library,
            MusicError::Cancelled => Self::Cancelled,
            MusicError::Io(_) | MusicError::Project(_) | MusicError::Media(_) => Self::CacheFailed,
            MusicError::Invalid(_) => Self::Invalid,
        }
    }

    pub fn from_edit(error: &EngineError) -> Self {
        match error {
            EngineError::Music(error) => Self::from_error(error),
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

#[derive(Clone, Debug, PartialEq)]
pub struct MusicCacheView {
    pub id: String,
    pub progress: f32,
    pub cancelling: bool,
}

/// Read-only projection consumed by the assets view and inspection.
#[derive(Clone, Debug)]
pub struct MusicView {
    pub mode: MusicMode,
    pub chip: &'static str,
    pub searching: bool,
    pub online: bool,
    pub key_set: bool,
    pub source: SearchSource,
    pub results: Vec<MusicResult>,
    pub saved: Vec<SavedMusic>,
    pub saved_ids: BTreeSet<String>,
    pub page: u32,
    pub count: u64,
    pub has_next: bool,
    pub previewing: Option<String>,
    pub search_problem: Option<MusicProblem>,
    pub preview_problem: Option<MusicProblem>,
    pub cache_problem: Option<MusicProblem>,
    pub saved_problem: Option<MusicProblem>,
    pub cache: Option<MusicCacheView>,
}

struct CacheJob {
    id: JobId,
    cancel: CancelToken,
    music: MusicResult,
    progress: f32,
    cancelling: bool,
}

/// One editor's Music presentation over the app-shared session and library.
pub struct Music {
    session: Arc<MusicSession>,
    /// A resolved client snapshot, or a test-injected client that outranks it.
    client: Option<MusicClient>,
    client_override: Option<MusicClient>,
    /// Whether this presentation may run remote searches at all.
    online: bool,
    key_generation: u64,
    downloads: Arc<MusicDownloads>,
    preview: Arc<MusicPreview>,
    runner: JobRunner,
    mode: MusicMode,
    chip: &'static str,
    query: String,
    results: Vec<MusicResult>,
    source: SearchSource,
    page: u32,
    count: u64,
    has_next: bool,
    searching: bool,
    searched: bool,
    search_generation: u64,
    search_rx: Option<Receiver<(u64, std::result::Result<SearchPage, MusicError>)>>,
    search_problem: Option<MusicProblem>,
    saved: Vec<SavedMusic>,
    saved_ids: BTreeSet<String>,
    saved_problem: Option<MusicProblem>,
    previewing: Option<String>,
    preview_rx: Option<Receiver<std::result::Result<(), MusicError>>>,
    preview_worker: Option<std::thread::JoinHandle<()>>,
    preview_pending: Option<MusicResult>,
    preview_cancel: Option<Arc<AtomicBool>>,
    preview_problem: Option<MusicProblem>,
    cache: Option<CacheJob>,
    cache_problem: Option<MusicProblem>,
    /// The track whose add failed, retained so Retry resubmits it rather than
    /// reloading search or losing the creator's target.
    failed_add: Option<MusicResult>,
    ready_add: Option<MusicResult>,
}

impl Music {
    /// Production composition: the process-shared session (anonymous until a key
    /// is applied), direct downloads and the app-owned preview sink.
    pub fn new(session: Arc<MusicSession>, budget: ByteBudget) -> Result<Self> {
        let client = Some(session.client());
        let downloads = Arc::new(MusicDownloads::ureq(session.library().clone()));
        let preview = Arc::new(MusicPreview::new(
            downloads.clone(),
            Arc::new(super::sink::OutputSink::new(budget.clone())),
        ));
        Self::with_parts(session, client, downloads, preview, budget)
    }

    /// An injected client over the production download and preview composition;
    /// tests and audit scenes use it to answer from a recorded transport.
    pub fn with_client(
        session: Arc<MusicSession>,
        client: MusicClient,
        budget: ByteBudget,
    ) -> Result<Self> {
        let downloads = Arc::new(MusicDownloads::ureq(session.library().clone()));
        let preview = Arc::new(MusicPreview::new(
            downloads.clone(),
            Arc::new(super::sink::OutputSink::new(budget.clone())),
        ));
        Self::with_parts(session, Some(client), downloads, preview, budget)
    }

    /// Injection boundary for tests and audit scenes. The caller owns the
    /// session, both transports and the preview sink.
    pub fn with_parts(
        session: Arc<MusicSession>,
        client_override: Option<MusicClient>,
        downloads: Arc<MusicDownloads>,
        preview: Arc<MusicPreview>,
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
        let saved = session
            .library()
            .saved()
            .map_err(|_| Error::Invalid("saved music"))?;
        let saved_ids = saved.iter().map(|item| item.music.id.clone()).collect();
        let online = client_override.is_some();
        let client = client_override.clone();
        let key_generation = session.generation();
        Ok(Self {
            session,
            client,
            client_override,
            online,
            key_generation,
            downloads,
            preview,
            runner,
            mode: MusicMode::Browse,
            chip: libraries::music::CHIPS[1],
            query: String::new(),
            results: Vec::new(),
            source: SearchSource::Local,
            page: 1,
            count: 0,
            has_next: false,
            searching: false,
            searched: false,
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
            failed_add: None,
            ready_add: None,
        })
    }

    pub fn session(&self) -> &Arc<MusicSession> {
        &self.session
    }

    pub fn mode(&self) -> MusicMode {
        self.mode
    }

    pub fn chip(&self) -> &'static str {
        self.chip
    }

    pub fn has_client(&self) -> bool {
        self.online
    }

    /// Apply or remove the session key. A refused key keeps the previous
    /// credential and returns its typed failure; an accepted change invalidates
    /// outstanding search results so the next Browse uses the new header.
    pub fn apply_key(&mut self, key: Option<&str>) -> Result<()> {
        self.session.set_key(key)?;
        self.invalidate_key();
        Ok(())
    }

    fn invalidate_key(&mut self) {
        self.client = self
            .client_override
            .clone()
            .or_else(|| self.online.then(|| self.session.client()));
        self.key_generation = self.session.generation();
        self.search_generation = self.search_generation.wrapping_add(1);
        self.search_rx = None;
        self.searching = false;
        self.searched = false;
        self.results.clear();
        self.page = 1;
        self.count = 0;
        self.has_next = false;
        self.search_problem = None;
        self.stop();
    }

    /// Run the first browse search once, when the section is first opened.
    pub fn ensure_loaded(&mut self) {
        if self.searched {
            return;
        }
        self.searched = true;
        self.search("", MusicMode::Browse, self.chip, false);
    }

    pub fn set_mode(&mut self, mode: MusicMode) {
        self.mode = mode;
    }

    /// Start a page-one search, or append the next page when `more` is set.
    pub fn search(&mut self, query: &str, mode: MusicMode, chip: &'static str, more: bool) {
        self.mode = mode;
        self.query = query.to_owned();
        if query.is_empty() {
            self.chip = chip;
        }
        let page = if more { self.page.saturating_add(1) } else { 1 };
        let request = SearchRequest {
            query: query.to_owned(),
            tag: (mode == MusicMode::Browse && query.is_empty()).then(|| chip.to_owned()),
            page,
            page_size: PAGE_SIZE.min(20),
        };
        self.search_problem = None;
        self.searched = true;
        if mode == MusicMode::Saved || self.client.is_none() {
            self.apply_page(self.session.library().search_local(&request), more);
            return;
        }
        let Some(client) = self.client.clone() else {
            return;
        };
        let (tx, rx) = mpsc::channel();
        self.search_generation = self.search_generation.wrapping_add(1);
        let generation = self.search_generation;
        let spawned = std::thread::Builder::new()
            .name("vibecut-music-search".into())
            .spawn(move || {
                let _ = tx.send((generation, client.search(&request)));
            });
        match spawned {
            Ok(_) => {
                self.searching = true;
                self.search_rx = Some(rx);
            }
            Err(_) => {
                self.searching = false;
                self.search_problem = Some(MusicProblem::Invalid);
            }
        }
    }

    pub fn search_more(&mut self) {
        if !self.has_next || self.searching {
            return;
        }
        let (query, mode, chip) = (self.query.clone(), self.mode, self.chip);
        self.search(&query, mode, chip, true);
    }

    fn apply_page(&mut self, page: std::result::Result<SearchPage, MusicError>, append: bool) {
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
                self.source = page.source;
            }
            Err(error) => {
                self.search_problem = Some(MusicProblem::from_error(&error));
            }
        }
    }

    pub fn refresh_saved(&mut self) {
        match self.session.library().saved() {
            Ok(saved) => {
                self.saved_ids = saved.iter().map(|item| item.music.id.clone()).collect();
                self.saved = saved;
                self.saved_problem = None;
            }
            Err(_) => self.saved_problem = Some(MusicProblem::Library),
        }
    }

    pub fn music(&self, id: &str) -> Option<MusicResult> {
        self.results
            .iter()
            .chain(self.saved.iter().map(|saved| &saved.music))
            .find(|music| music.id == id)
            .cloned()
    }

    /// Save or unsave one track; the local library owns both operations.
    pub fn toggle_saved(&mut self, id: &str) {
        let Some(music) = self.music(id) else {
            self.saved_problem = Some(MusicProblem::Library);
            return;
        };
        let result = if self.saved_ids.contains(id) {
            self.session.library().unsave(id).map(|_| ())
        } else {
            self.session.library().save(&music).map(|_| ())
        };
        match result {
            Ok(()) => {
                self.saved_problem = None;
                self.refresh_saved();
                if self.mode == MusicMode::Saved {
                    self.reload_saved();
                }
            }
            Err(error) => self.saved_problem = Some(MusicProblem::from_error(&error)),
        }
    }

    fn reload_saved(&mut self) {
        let request = SearchRequest {
            query: self.query.clone(),
            tag: None,
            page: 1,
            page_size: PAGE_SIZE.min(20),
        };
        self.apply_page(self.session.library().search_local(&request), false);
    }

    pub fn play(&mut self, id: &str) {
        if self.previewing.as_deref() == Some(id) {
            self.stop();
            return;
        }
        let Some(music) = self.music(id) else {
            self.preview_problem = Some(MusicProblem::Library);
            return;
        };
        if self.preview_worker.is_some() {
            self.stop();
        }
        self.previewing = Some(id.to_owned());
        self.preview_problem = None;
        self.preview_pending = Some(music);
        self.start_preview();
    }

    fn start_preview(&mut self) {
        if self.preview_worker.is_some() {
            return;
        }
        let Some(music) = self.preview_pending.take() else {
            return;
        };
        let preview = self.preview.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        self.preview_cancel = Some(cancel.clone());
        let (tx, rx) = mpsc::sync_channel(1);
        let spawned = std::thread::Builder::new()
            .name("vibecut-music-preview".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    preview.play(&music, &|| cancel.load(Ordering::Acquire))
                }))
                .unwrap_or(Err(MusicError::NoAudioDevice));
                let _ = tx.send(result);
            });
        match spawned {
            Ok(worker) => {
                self.preview_worker = Some(worker);
                self.preview_rx = Some(rx);
            }
            Err(_) => {
                self.previewing = None;
                self.preview_problem = Some(MusicProblem::NoAudioDevice);
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
    pub fn request_add(&mut self, music: MusicResult) {
        if self.cache.is_some() {
            self.cache_problem = Some(MusicProblem::CacheBusy);
            return;
        }
        match self.session.library().cached(&music.audio_url) {
            Ok(Some(_)) => {
                self.cache_problem = None;
                self.failed_add = None;
                self.ready_add = Some(music);
                return;
            }
            Ok(None) => {}
            Err(error) => {
                self.cache_problem = Some(MusicProblem::from_error(&error));
                self.failed_add = Some(music);
                return;
            }
        }
        match self.downloads.submit(&mut self.runner, &music.audio_url) {
            Ok((id, cancel)) => {
                self.cache_problem = None;
                self.failed_add = None;
                self.cache = Some(CacheJob {
                    id,
                    cancel,
                    music,
                    progress: 0.0,
                    cancelling: false,
                });
            }
            Err(error) => {
                self.cache_problem = Some(MusicProblem::from_edit(&error));
                self.failed_add = Some(music);
            }
        }
    }

    pub fn cancel_cache(&mut self) {
        if let Some(cache) = &mut self.cache {
            cache.cancel.cancel();
            cache.cancelling = true;
        }
    }

    /// Resubmit the retained failed add with its original track and target.
    pub fn retry_add(&mut self) {
        if let Some(music) = self.failed_add.take() {
            self.cache_problem = None;
            self.request_add(music);
        }
    }

    pub fn take_ready_add(&mut self) -> Option<MusicResult> {
        self.ready_add.take()
    }

    pub fn add_failed(&mut self, problem: MusicProblem) {
        self.cache_problem = Some(problem);
    }

    pub fn cached(&self, music: &MusicResult) -> Option<CachedMusic> {
        self.session
            .library()
            .cached(&music.audio_url)
            .ok()
            .flatten()
    }

    pub fn clear_problems(&mut self) {
        self.search_problem = None;
        self.preview_problem = None;
        self.cache_problem = None;
        self.saved_problem = None;
    }

    /// Pump jobs, key changes and retained workers; keep only terminal truth.
    pub fn tick(&mut self) -> Result<()> {
        if self.key_generation != self.session.generation() {
            self.invalidate_key();
        }
        if let Err(error) = self.runner.tick() {
            self.cache_problem = Some(MusicProblem::from_edit(&error.into()));
            self.cache = None;
        }
        if let Some(rx) = &self.search_rx {
            match rx.try_recv() {
                Ok((generation, page)) => {
                    if generation == self.search_generation {
                        self.search_rx = None;
                        let append = page.as_ref().is_ok_and(|page| page.page > 1);
                        self.apply_page(page, append);
                    }
                }
                Err(TryRecvError::Disconnected) => {
                    self.search_rx = None;
                    self.searching = false;
                    self.search_problem = Some(MusicProblem::Invalid);
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
                            && !matches!(error, MusicError::Cancelled)
                        {
                            self.preview_problem = Some(MusicProblem::from_error(&error));
                        }
                    }
                    Err(TryRecvError::Disconnected) => {
                        self.previewing = None;
                        self.preview_problem = Some(MusicProblem::NoAudioDevice);
                    }
                    Err(TryRecvError::Empty) => {}
                }
            }
            self.start_preview();
        }
        let (music, cancelling, state, error, progress) = {
            let Some(cache) = self.cache.as_ref() else {
                return Ok(());
            };
            let Some(snapshot) = self.runner.snapshot(cache.id) else {
                return Ok(());
            };
            (
                cache.music.clone(),
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
                match self.session.library().cached(&music.audio_url) {
                    Ok(Some(_)) => {
                        self.failed_add = None;
                        self.ready_add = Some(music);
                    }
                    Ok(None) | Err(_) => {
                        self.cache_problem = Some(MusicProblem::CacheFailed);
                        self.failed_add = Some(music);
                    }
                }
            }
            JobState::Failed | JobState::Cancelled => {
                self.cache = None;
                if state == JobState::Failed {
                    self.failed_add = Some(music.clone());
                }
                if !cancelling && state == JobState::Failed {
                    self.cache_problem = Some(
                        error
                            .as_deref()
                            .and_then(|error| {
                                (error as &(dyn std::error::Error + 'static))
                                    .downcast_ref::<EngineError>()
                            })
                            .map(MusicProblem::from_edit)
                            .filter(|problem| !matches!(problem, MusicProblem::Cancelled))
                            .unwrap_or(MusicProblem::CacheFailed),
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

    pub fn view(&self) -> MusicView {
        MusicView {
            mode: self.mode,
            chip: self.chip,
            searching: self.searching,
            online: self.online,
            key_set: self.session.key_set(),
            source: self.source,
            results: self.results.clone(),
            saved: self.saved.clone(),
            saved_ids: self.saved_ids.clone(),
            page: self.page,
            count: self.count,
            has_next: self.has_next,
            previewing: self.previewing.clone(),
            search_problem: self.search_problem,
            preview_problem: self.preview_problem,
            cache_problem: self.cache_problem,
            saved_problem: self.saved_problem,
            cache: self.cache.as_ref().map(|cache| MusicCacheView {
                id: cache.music.id.clone(),
                progress: cache.progress,
                cancelling: cache.cancelling,
            }),
        }
    }
}

impl Drop for Music {
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

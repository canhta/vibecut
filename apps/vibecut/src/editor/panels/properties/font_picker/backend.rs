//! Adapts the picker to the registered engine `fonts.*` commands.
use engine::control::{Caller, ControlResult, FontAccess, FontEntry, FontLoad, JobView, Policy};
use engine::jobs::{JobId, JobRunner, RunnerConfig, SystemClock};
use engine::{ByteBudget, Command, Session};
use std::sync::Arc;
use std::time::Duration;
pub type FontChoice =
    std::pin::Pin<Box<dyn std::future::Future<Output = Option<std::path::PathBuf>> + Send>>;

/// The catalog operations the picker consumes plus job progress and
/// cancellation. A second adapter is the test host; production uses
/// [`EngineBackend`].
pub trait Backend: Send {
    /// Attach current editor accounting; return true when old jobs were discarded.
    /// Recording adapters with no background work need no accounting binding.
    fn bind_budget(&mut self, _: ByteBudget) -> crate::Result<bool> {
        Ok(false)
    }
    fn choose_font(&mut self, locale: crate::i18n::Locale) -> FontChoice;
    fn import(&mut self, session: &mut Session, path: std::path::PathBuf) -> crate::Result<JobId>;
    fn list(&mut self, session: &mut Session, query: Option<&str>)
    -> crate::Result<Vec<FontEntry>>;
    fn load(&mut self, session: &mut Session, family: &str) -> crate::Result<FontLoad>;
    fn download(&mut self, session: &mut Session, family: &str) -> crate::Result<JobId>;
    fn job(&mut self, job: JobId) -> Option<JobView>;
    fn cancel(&mut self, session: &mut Session, job: JobId) -> crate::Result<()>;
}

/// Production adapter: the shared engine font store behind the registered
/// `fonts.list`, `fonts.load`, `fonts.download` and `job_cancel` commands.
pub struct EngineBackend {
    store: Arc<libraries::fonts::FontStore>,
    runner: JobRunner,
    choose: Box<dyn FnMut(crate::i18n::Locale) -> FontChoice + Send>,
}
impl EngineBackend {
    pub fn native(budget: ByteBudget) -> crate::Result<Self> {
        let store = Arc::new(libraries::fonts::FontStore::native()?);
        Self::with_store(store, budget, |locale| {
            let title = crate::i18n::text(locale, crate::i18n::Text::FontUpload);
            let dialog = rfd::AsyncFileDialog::new()
                .set_title(title)
                .add_filter(title, &["ttf", "otf", "woff2"]);
            Box::pin(async move { dialog.pick_file().await.map(|file| file.path().to_owned()) })
        })
    }
    pub fn with_store(
        store: Arc<libraries::fonts::FontStore>,
        budget: ByteBudget,
        choose: impl FnMut(crate::i18n::Locale) -> FontChoice + Send + 'static,
    ) -> crate::Result<Self> {
        Ok(Self {
            store,
            runner: Self::runner(budget)?,
            choose: Box::new(choose),
        })
    }
    fn runner(budget: ByteBudget) -> crate::Result<JobRunner> {
        Ok(JobRunner::new(
            RunnerConfig {
                concurrency: [1, 1, 1],
                queue: 1,
                retained: 4,
                network_attempts: 1,
                backoff: Duration::from_millis(250),
                progress_interval: Duration::from_millis(40),
            },
            budget,
            Arc::new(SystemClock::default()),
        )?)
    }
    fn request(&mut self, session: &mut Session, command: Command) -> crate::Result<ControlResult> {
        let store = &self.store;
        let runner = &mut self.runner;
        let list = |query: Option<&str>| store.list(query);
        let load = |family: &str| store.load(family);
        let download = |runner: &mut JobRunner, family: &str, accepted: bool| {
            store
                .submit_download(runner, family, accepted)
                .map(|(id, _)| id)
        };
        let import = |runner: &mut JobRunner, path: &std::path::Path, accepted: bool| {
            store
                .submit_import(runner, path.to_owned(), accepted)
                .map(|(id, _)| id)
        };
        let access = FontAccess::new(&list, &load, &download, &import);
        Ok(session.control_command(
            command,
            Caller {
                principal: "vibecut-desktop",
                external: false,
                policy: &Allow,
                fonts: Some(&access),
                sounds: None,
                music: None,
                brand: None,
                media: None,
                chapters: None,
                captions: None,
                vision: None,
            },
            runner,
        )?)
    }
}
impl Backend for EngineBackend {
    fn bind_budget(&mut self, budget: ByteBudget) -> crate::Result<bool> {
        if self.runner.memory_budget().same_policy(&budget) {
            return Ok(false);
        }
        self.runner.shutdown();
        self.runner = Self::runner(budget)?;
        Ok(true)
    }
    fn choose_font(&mut self, locale: crate::i18n::Locale) -> FontChoice {
        (self.choose)(locale)
    }
    fn import(&mut self, session: &mut Session, path: std::path::PathBuf) -> crate::Result<JobId> {
        match self.request(
            session,
            Command::FontsImport {
                path,
                accept_license: true,
            },
        )? {
            ControlResult::JobAccepted { job, .. } => Ok(job),
            _ => Err(crate::Error::Invalid("fonts.import result")),
        }
    }
    fn list(
        &mut self,
        session: &mut Session,
        query: Option<&str>,
    ) -> crate::Result<Vec<FontEntry>> {
        let query = query.map(engine::NfcText::from);
        match self.request(session, Command::FontsList { query })? {
            ControlResult::Fonts { entries } => Ok(entries),
            _ => Err(crate::Error::Invalid("fonts.list result")),
        }
    }
    fn load(&mut self, session: &mut Session, family: &str) -> crate::Result<FontLoad> {
        match self.request(
            session,
            Command::FontsLoad {
                family: family.into(),
            },
        )? {
            ControlResult::Font { font } => Ok(font),
            _ => Err(crate::Error::Invalid("fonts.load result")),
        }
    }
    fn download(&mut self, session: &mut Session, family: &str) -> crate::Result<JobId> {
        match self.request(
            session,
            Command::FontsDownload {
                family: family.into(),
                accept_license: true,
            },
        )? {
            ControlResult::JobAccepted { job, .. } => Ok(job),
            _ => Err(crate::Error::Invalid("fonts.download result")),
        }
    }
    fn job(&mut self, job: JobId) -> Option<JobView> {
        self.runner.tick().ok()?;
        self.runner.snapshot(job).map(JobView::from)
    }
    fn cancel(&mut self, session: &mut Session, job: JobId) -> crate::Result<()> {
        match self.request(session, Command::JobCancel { job })? {
            ControlResult::CancelRequested { .. } => Ok(()),
            _ => Err(crate::Error::Invalid("job_cancel result")),
        }
    }
}
struct Allow;
impl Policy for Allow {
    fn authorize(&self, _: &engine::Command, _: &project::Project) -> engine::control::Result<()> {
        Ok(())
    }
}

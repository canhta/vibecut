//! One process-shared music library and its in-memory session credential.
//!
//! The credential lives only in memory for this process. It is never written to
//! Preferences, a project, automation, undo or a log; `Debug` is redacted and the
//! key is sent only as a bearer header by the library's transport.
use crate::{Error, Result};
use engine::{
    ByteBudget, Command, Session,
    control::{API_VERSION, Caller, ControlResult, Policy, Request, registry_hash},
    jobs::{JobRunner, RunnerConfig, SystemClock},
};
use libraries::music::{MusicClient, MusicLibrary, MusicToken};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};

/// One music library shared by every project window plus the current process key.
pub struct MusicSession {
    library: Arc<MusicLibrary>,
    credential: Mutex<Credential>,
    generation: AtomicU64,
}

#[derive(Clone)]
enum Credential {
    Anonymous,
    User(MusicToken),
}

impl MusicSession {
    pub fn new(library: Arc<MusicLibrary>) -> Self {
        Self {
            library,
            credential: Mutex::new(Credential::Anonymous),
            generation: AtomicU64::new(0),
        }
    }

    pub fn library(&self) -> &Arc<MusicLibrary> {
        &self.library
    }

    /// Replace the session key; `None` or an empty value removes it and returns
    /// to anonymous operation. A malformed key keeps the previous credential and
    /// returns the typed auth failure. A change invalidates outstanding display
    /// work through [`Self::generation`].
    pub fn set_key(&self, key: Option<&str>) -> Result<()> {
        let credential = match key {
            None | Some("") => Credential::Anonymous,
            Some(text) => Credential::User(
                MusicToken::new(text)
                    .map_err(|error| Error::Engine(engine::control::Error::Music(error)))?,
            ),
        };
        *self.credential.lock().map_err(|_| Error::Poisoned)? = credential;
        self.generation.fetch_add(1, Ordering::AcqRel);
        Ok(())
    }

    /// Whether a user key is set; the value itself never leaves this object.
    pub fn key_set(&self) -> bool {
        self.credential
            .lock()
            .is_ok_and(|credential| matches!(*credential, Credential::User(_)))
    }

    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    /// Resolve a lazy client snapshot. Anonymous operation still has a client;
    /// it simply carries no bearer header. A poisoned credential lock degrades to
    /// anonymous rather than exposing or reusing a key.
    pub fn client(&self) -> MusicClient {
        match self.credential.lock() {
            Ok(credential) => match &*credential {
                Credential::Anonymous => MusicClient::ureq(None),
                Credential::User(token) => MusicClient::ureq(Some(token.clone())),
            },
            Err(_) => MusicClient::ureq(None),
        }
    }
}

/// The desktop is the trusted composition root of its own music requests.
struct Trusted;
impl Policy for Trusted {
    fn authorize(&self, _: &Command, _: &project::Project) -> engine::control::Result<()> {
        Ok(())
    }
}

/// Run one `music.*` registry command over `library`, the way the CLI does: an
/// empty workflow session per call and the library adapter as the lent access.
/// `remote` is the scoped approval for a deliberate remote search; Saved, save and
/// unsave pass `None` and never resolve a client. Callable from any thread.
pub fn run_music(
    library: &MusicLibrary,
    remote: Option<&MusicClient>,
    budget: &ByteBudget,
    command: &str,
    params: serde_json::Value,
) -> engine::control::Result<ControlResult> {
    let mut session = Session::new(
        project::Project::empty(project::ProjectId(0), String::new().into())?,
        1,
    )
    .map_err(engine::control::Error::Edit)?;
    let mut runner = JobRunner::new(
        RunnerConfig {
            concurrency: [1, 1, 1],
            queue: 1,
            retained: 1,
            network_attempts: 1,
            backoff: std::time::Duration::from_millis(250),
            progress_interval: std::time::Duration::from_millis(50),
        },
        budget.clone(),
        Arc::new(SystemClock::default()),
    )?;
    let request = Request {
        api_version: API_VERSION,
        registry_hash: Some(registry_hash().into()),
        session_id: session.session_id().into(),
        request_id: command.into(),
        idempotency_key: None,
        base_rev: None,
        command: command.into(),
        params,
    };
    let resolve = remote.map(|client| move || -> libraries::music::Music<MusicClient> {
        Ok(client.clone())
    });
    let approval = resolve
        .as_ref()
        .map(|resolve| resolve as &dyn Fn() -> libraries::music::Music<MusicClient>);
    let result = library.with_control(approval, |access| {
        session.control_typed(
            request,
            Caller {
                principal: "vibecut-desktop",
                external: false,
                policy: &Trusted,
                fonts: None,
                sounds: None,
                music: Some(access),
                stickers: None,
                brand: None,
                media: None,
                chapters: None,
                captions: None,
                vision: None,
            },
            &mut runner,
        )
    });
    runner.shutdown();
    result
}

impl std::fmt::Debug for MusicSession {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MusicSession")
            .field("library", &self.library.root())
            .field("key_set", &self.key_set())
            .field("generation", &self.generation())
            .finish()
    }
}

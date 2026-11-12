//! One process-shared music library and its in-memory session credential.
//!
//! The credential lives only in memory for this process. It is never written to
//! Preferences, a project, automation, undo or a log; `Debug` is redacted and the
//! key is sent only as a bearer header by the library's transport.
use crate::{Error, Result};
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

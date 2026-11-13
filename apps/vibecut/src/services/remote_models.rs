//! Owns remote model selection independently of any window.
mod selection;
use ai::foundation::transport::ApiKey;
use engine::providers::Credentials;
pub use selection::{AiPreferences, MAX_RECENT_MODELS, ModelChoice};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use transport::HttpTransport;

struct Hooks(Arc<AtomicBool>);
impl ai::ExecutionHooks for Hooks {
    fn cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
    fn stage(&self, _: ai::Stage) {}
}
struct Refresh {
    generation: u64,
    cancel: Arc<AtomicBool>,
    receive: mpsc::Receiver<Result<Vec<captions::NfcText>, ai::Error>>,
    worker: std::thread::JoinHandle<()>,
}
pub(crate) struct Catalog {
    pub(crate) provider: &'static str,
    key: Option<ApiKey>,
    generation: u64,
    refresh: Option<Refresh>,
    pub(crate) models: Vec<captions::NfcText>,
    pub(crate) error: Option<ai::Error>,
    pub(crate) fetched: bool,
}
impl Catalog {
    fn new(provider: &'static str) -> Self {
        Self {
            provider,
            key: None,
            generation: 0,
            refresh: None,
            models: vec![],
            error: None,
            fetched: false,
        }
    }
    pub(crate) fn loading(&self) -> bool {
        self.refresh.is_some()
    }
    pub(crate) fn has_key(&self) -> bool {
        self.key.is_some()
    }
    fn sync(&mut self, credentials: &Credentials) {
        let key = match self.provider {
            "openai" => credentials.openai(),
            "anthropic" => credentials.anthropic(),
            _ => credentials.elevenlabs(),
        }
        .ok();
        if self.key != key {
            self.key = key;
            self.generation = self.generation.wrapping_add(1);
            self.models.clear();
            self.error = None;
            self.fetched = false;
            if let Some(refresh) = &self.refresh {
                refresh.cancel.store(true, Ordering::Release);
            }
        }
        let ready = self
            .refresh
            .as_ref()
            .and_then(|refresh| match refresh.receive.try_recv() {
                Ok(result) => Some(result),
                Err(mpsc::TryRecvError::Disconnected) => Some(Err(ai::Error::Inference)),
                Err(mpsc::TryRecvError::Empty) => None,
            });
        if let Some(result) = ready
            && let Some(refresh) = self.refresh.take()
        {
            let joined = refresh.worker.join();
            if refresh.generation == self.generation && self.key.is_some() {
                self.fetched = true;
                match result {
                    Ok(models) if joined.is_ok() => {
                        self.models = models;
                        self.error = None
                    }
                    Err(error) => {
                        self.models.clear();
                        self.error = Some(error)
                    }
                    _ => self.error = Some(ai::Error::Inference),
                }
            }
        }
    }
    pub(crate) fn request(&mut self, transport: Arc<dyn HttpTransport>) -> crate::Result<()> {
        if self.refresh.is_some() {
            return Ok(());
        }
        let key = self
            .key
            .clone()
            .ok_or(crate::Error::Invalid("selected service key is unavailable"))?;
        let provider = self.provider;
        let cancel = Arc::new(AtomicBool::new(false));
        let hooks = Hooks(cancel.clone());
        let (send, receive) = mpsc::sync_channel(1);
        let worker = std::thread::Builder::new()
            .name("remote-model-list".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    ai::list_remote_models(provider, &key, transport.as_ref(), &hooks)
                }))
                .unwrap_or(Err(ai::Error::Inference));
                let _ = send.send(result);
            })?;
        self.error = None;
        self.refresh = Some(Refresh {
            generation: self.generation,
            cancel,
            receive,
            worker,
        });
        Ok(())
    }
}
impl Drop for Catalog {
    fn drop(&mut self) {
        if let Some(refresh) = self.refresh.take() {
            refresh.cancel.store(true, Ordering::Release);
            let _ = refresh.worker.join();
        }
    }
}
pub(crate) struct Catalogs {
    pub(crate) entries: Vec<Catalog>,
}
impl Default for Catalogs {
    fn default() -> Self {
        Self {
            entries: ["openai", "anthropic", "elevenlabs"]
                .into_iter()
                .map(Catalog::new)
                .collect(),
        }
    }
}
impl Catalogs {
    pub(crate) fn sync(&mut self, credentials: &Credentials) {
        for catalog in &mut self.entries {
            catalog.sync(credentials);
        }
    }
    pub(crate) fn get(&self, provider: &str) -> Option<&Catalog> {
        self.entries
            .iter()
            .find(|catalog| catalog.provider == provider)
    }
    pub(crate) fn get_mut(&mut self, provider: &str) -> Option<&mut Catalog> {
        self.entries
            .iter_mut()
            .find(|catalog| catalog.provider == provider)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
    use super::*;
    use std::sync::Mutex;
    use transport::{HttpRequest, HttpResponse};
    struct Blocked {
        started: mpsc::SyncSender<()>,
        release: Mutex<mpsc::Receiver<()>>,
    }
    impl HttpTransport for Blocked {
        fn send(&self, request: HttpRequest) -> Result<HttpResponse, transport::Error> {
            assert_eq!(request.method, "GET");
            assert!(request.body.is_empty());
            self.started.send(()).unwrap();
            self.release
                .lock()
                .unwrap()
                .recv_timeout(std::time::Duration::from_secs(3))
                .unwrap();
            Ok(HttpResponse {
                status: 200,
                headers: vec![],
                body: br#"{"data":[{"id":"fixture-stale"}]}"#.to_vec(),
            })
        }
    }
    struct Reply(u16, &'static [u8]);
    impl HttpTransport for Reply {
        fn send(&self, _: HttpRequest) -> Result<HttpResponse, transport::Error> {
            Ok(HttpResponse {
                status: self.0,
                headers: vec![],
                body: self.1.to_vec(),
            })
        }
    }
    fn wait(catalogs: &mut Catalogs, credentials: &Credentials) {
        let until = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while catalogs.get("openai").unwrap().loading() {
            catalogs.sync(credentials);
            assert!(std::time::Instant::now() < until);
            std::thread::yield_now();
        }
    }
    #[test]
    fn refresh_stale_key_result_is_discarded_and_error_empty_and_provider_states_are_independent() {
        let mut catalogs = Catalogs::default();
        let none = Credentials::new(None, None, None);
        catalogs.sync(&none);
        assert!(!catalogs.get("openai").unwrap().has_key());
        assert!(
            catalogs
                .get_mut("openai")
                .unwrap()
                .request(Arc::new(Reply(200, b"{}")))
                .is_err()
        );
        let first = Credentials::new(
            Some("synthetic-one".into()),
            None,
            Some("unrelated-invalid ".into()),
        );
        catalogs.sync(&first);
        assert!(catalogs.get("openai").unwrap().has_key());
        assert!(!catalogs.get("anthropic").unwrap().has_key());
        let (started, receive) = mpsc::sync_channel(1);
        let (release, wait_release) = mpsc::sync_channel(1);
        catalogs
            .get_mut("openai")
            .unwrap()
            .request(Arc::new(Blocked {
                started,
                release: Mutex::new(wait_release),
            }))
            .unwrap();
        receive
            .recv_timeout(std::time::Duration::from_secs(3))
            .unwrap();
        assert!(catalogs.get("openai").unwrap().loading());
        let second = Credentials::new(Some("synthetic-two".into()), None, None);
        catalogs.sync(&second);
        release.send(()).unwrap();
        wait(&mut catalogs, &second);
        assert!(catalogs.get("openai").unwrap().models.is_empty());
        assert!(!catalogs.get("openai").unwrap().fetched);
        catalogs
            .get_mut("openai")
            .unwrap()
            .request(Arc::new(Reply(200, br#"{"data":[]}"#)))
            .unwrap();
        wait(&mut catalogs, &second);
        assert!(catalogs.get("openai").unwrap().fetched);
        assert!(catalogs.get("openai").unwrap().models.is_empty());
        catalogs
            .get_mut("openai")
            .unwrap()
            .request(Arc::new(Reply(401, b"synthetic-private-body")))
            .unwrap();
        wait(&mut catalogs, &second);
        assert!(matches!(
            catalogs.get("openai").unwrap().error,
            Some(ai::Error::Auth)
        ));
        catalogs.sync(&none);
        assert!(catalogs.get("openai").unwrap().error.is_none());
        assert!(!catalogs.get("openai").unwrap().fetched);
        assert!(catalogs.get("elevenlabs").unwrap().models.is_empty());
    }
}

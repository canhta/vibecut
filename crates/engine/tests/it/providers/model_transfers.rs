#![allow(
    clippy::unwrap_used,
    clippy::panic,
    reason = "model transfer integration fixtures"
)]
use ai::{
    models::{DownloadConsent, Model, ModelFile},
    transport::{DownloadTransport, HttpStreamResponse},
};
use engine::{
    ByteBudget, ByteOwner, EditError,
    cache::ArtifactCache,
    jobs::*,
    models::{ModelIo, ModelTransfers, SystemModelIo},
};
use std::{
    collections::VecDeque,
    fs::{self, File},
    io::{self, Read, Write},
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
#[derive(Default)]
struct TestClock(AtomicU64);
impl Clock for TestClock {
    fn now(&self) -> Duration {
        Duration::from_millis(self.0.load(Ordering::Relaxed))
    }
}
impl TestClock {
    fn advance(&self) {
        self.0.fetch_add(100, Ordering::Relaxed);
    }
}
struct NoHooks;
impl ai::ExecutionHooks for NoHooks {
    fn cancelled(&self) -> bool {
        false
    }
    fn stage(&self, _: ai::Stage) {}
}
fn specimen(size: usize) -> (Arc<[u8]>, Model) {
    let hash = match size {
        11 => "c1c12d3c59a499c51380e179c361417820a2f1e247731339cf34684fb77f8cf1",
        131072 => "51b1754edc87242e9dd3190af0d6bbc796d258fd14cc6bac980cded045b19824",
        2097152 => "c406296b30d433e27c08e2989ad557c7e9ae7825d1bea14c42aa4ef53c9e8a9d",
        8388608 => "15d3670274ee9cc36af50fee9e6d49164c9eec2228d48fe29e4f313aa3784a91",
        _ => panic!("fixture size"),
    };
    (
        Arc::from(vec![7; size]),
        Model {
            id: "fixture".into(),
            license: "MIT".into(),
            license_url: "https://models.example/LICENSE".into(),
            source: "https://models.example/source".into(),
            languages: vec![captions::LanguageTag::parse("vi").unwrap()],
            files: vec![ModelFile {
                name: "weights.bin".into(),
                url: "https://models.example/weights.bin".into(),
                sha256: hash.into(),
                size: size as u64,
            }],
        },
    )
}
struct Body {
    data: Arc<[u8]>,
    position: usize,
    end: usize,
    fail: bool,
    cancel: Option<CancelToken>,
    max: Arc<AtomicUsize>,
}
impl Read for Body {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        self.max.fetch_max(out.len(), Ordering::Relaxed);
        if self.position >= self.end {
            return if self.fail {
                Err(io::Error::new(
                    io::ErrorKind::ConnectionReset,
                    "interrupted",
                ))
            } else {
                Ok(0)
            };
        }
        let n = out.len().min(self.end - self.position).min(65536);
        out.get_mut(..n)
            .unwrap()
            .copy_from_slice(self.data.get(self.position..self.position + n).unwrap());
        self.position += n;
        if let Some(token) = self.cancel.take() {
            token.cancel();
        }
        Ok(n)
    }
}
struct Reply {
    status: u16,
    headers: Vec<(String, String)>,
    data: Arc<[u8]>,
    start: usize,
    end: usize,
    fail: bool,
    cancel: Option<CancelToken>,
}
impl Reply {
    fn ok(data: Arc<[u8]>, tag: &str) -> Self {
        let len = data.len();
        Self {
            status: 200,
            headers: vec![
                ("ETag".into(), tag.into()),
                ("Content-Length".into(), len.to_string()),
            ],
            data,
            start: 0,
            end: len,
            fail: false,
            cancel: None,
        }
    }
    fn range(data: Arc<[u8]>, start: usize, tag: &str) -> Self {
        let len = data.len();
        Self {
            status: 206,
            headers: vec![
                ("ETag".into(), tag.into()),
                ("Content-Length".into(), (len - start).to_string()),
                (
                    "Content-Range".into(),
                    format!("bytes {start}-{}/{len}", len - 1),
                ),
            ],
            data,
            start,
            end: len,
            fail: false,
            cancel: None,
        }
    }
    fn interrupted(data: Arc<[u8]>) -> Self {
        let mut r = Self::ok(data, "\"one\"");
        r.end = 65536;
        r.fail = true;
        r
    }
}
type RequestLog = Vec<(String, Vec<(String, String)>)>;

#[derive(Default)]
struct Http {
    replies: Mutex<VecDeque<Reply>>,
    requests: Mutex<RequestLog>,
    max: Arc<AtomicUsize>,
}
impl Http {
    fn push(&self, r: Reply) {
        self.replies.lock().unwrap().push_back(r);
    }
    fn count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }
    fn h(&self, n: usize, name: &str) -> Option<String> {
        self.requests
            .lock()
            .unwrap()
            .get(n)
            .unwrap()
            .1
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.clone())
    }
}
impl DownloadTransport for Http {
    fn get(
        &self,
        url: &str,
        headers: &[(String, String)],
    ) -> Result<HttpStreamResponse, ai::Error> {
        self.requests
            .lock()
            .unwrap()
            .push((url.into(), headers.into()));
        let r = self
            .replies
            .lock()
            .unwrap()
            .pop_front()
            .ok_or(ai::Error::Network)?;
        Ok(HttpStreamResponse {
            status: r.status,
            headers: r.headers,
            body: Box::new(Body {
                data: r.data,
                position: r.start,
                end: r.end,
                fail: r.fail,
                cancel: r.cancel,
                max: self.max.clone(),
            }),
        })
    }
}
struct Io {
    available: u64,
    fail_call: usize,
    calls: AtomicUsize,
}
impl Io {
    fn healthy() -> Arc<Self> {
        Arc::new(Self {
            available: u64::MAX,
            fail_call: usize::MAX,
            calls: AtomicUsize::new(0),
        })
    }
}
impl ModelIo for Io {
    fn available(&self, _: &Path) -> io::Result<u64> {
        Ok(self.available)
    }
    fn write(&self, file: &mut File, data: &[u8]) -> io::Result<()> {
        let call = self.calls.fetch_add(1, Ordering::Relaxed);
        if call == self.fail_call {
            file.write_all(data.get(..data.len() / 2).unwrap())?;
            return Err(io::Error::new(io::ErrorKind::StorageFull, "injected"));
        }
        file.write_all(data)
    }
}
fn setup(
    root: &Path,
    http: Arc<Http>,
    io: Arc<dyn ModelIo>,
    attempts: u8,
) -> (ByteBudget, ModelTransfers, JobRunner, Arc<TestClock>) {
    let budget = ByteBudget::new(512 * 1024).unwrap();
    let clock = Arc::new(TestClock::default());
    let (cache, diagnostics) = ArtifactCache::open(root, budget.clone(), 32).unwrap();
    assert_eq!(diagnostics, 0);
    let store = ModelTransfers::new(cache, http, io);
    let runner = JobRunner::new(
        RunnerConfig {
            concurrency: [1, 1, 1],
            queue: 2,
            retained: 4,
            network_attempts: attempts,
            backoff: Duration::from_millis(100),
            progress_interval: Duration::from_millis(10),
        },
        budget.clone(),
        clock.clone(),
    )
    .unwrap();
    (budget, store, runner, clock)
}
fn submit(store: &ModelTransfers, runner: &mut JobRunner, m: &Model) -> (JobId, CancelToken) {
    store
        .submit(runner, m.clone(), Some(DownloadConsent::grant(m).unwrap()))
        .unwrap()
}
fn terminal(r: &mut JobRunner, id: JobId, clock: &TestClock) -> JobSnapshot {
    // Durable checkpoint/publication IO can stall on a shared host; this is a
    // liveness guard, while the callers assert the transfer's exact outcome.
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        clock.advance();
        r.tick().unwrap();
        let snapshot = r.snapshot(id).unwrap();
        if matches!(
            snapshot.state,
            JobState::Completed | JobState::Cancelled | JobState::Failed
        ) {
            return snapshot;
        }
        assert!(
            Instant::now() < deadline,
            "job {id:?} timed out in state {:?}",
            snapshot.state
        );
        thread::sleep(Duration::from_millis(1));
    }
}
#[test]
fn model_admission_reclaims_an_unpinned_ordinary_entry_at_the_entry_limit() {
    let dir = tempfile::tempdir().unwrap();
    let http = Arc::new(Http::default());
    let (data, model) = specimen(11);
    http.push(Reply::ok(data, "\"one\""));
    let budget = ByteBudget::new(512 * 1024).unwrap();
    let (mut cache, _) = ArtifactCache::open(dir.path(), budget.clone(), 1).unwrap();
    let ordinary = engine::cache::CacheKey::new(engine::cache::KeyInputs {
        kind: engine::cache::ArtifactKind::Preview,
        source: [1; 32],
        provider: "local",
        model: "preview",
        version: "1",
        settings: b"{}",
        transcript: [0; 32],
        cues: [0; 32],
        clip_range: [0; 32],
        translation: [0; 32],
    })
    .unwrap();
    cache.put(ordinary, b"preview").unwrap();
    let store = ModelTransfers::new(cache, http, Io::healthy());
    let clock = Arc::new(TestClock::default());
    let mut runner = JobRunner::new(
        RunnerConfig {
            concurrency: [1; 3],
            queue: 1,
            retained: 1,
            network_attempts: 1,
            backoff: Duration::from_millis(1),
            progress_interval: Duration::from_millis(10),
        },
        budget,
        clock.clone(),
    )
    .unwrap();
    let (id, _) = submit(&store, &mut runner, &model);
    let result = terminal(&mut runner, id, &clock);
    assert_eq!(result.state, JobState::Completed, "{:?}", result.error);
    assert!(store.installed(&model, &NoHooks).unwrap().is_some());
}
#[test]
fn streaming_bundle_is_pinned_reported_exempt_and_removed_only_explicitly() {
    let dir = tempfile::tempdir().unwrap();
    let http = Arc::new(Http::default());
    let (data, model) = specimen(2097152);
    http.push(Reply::ok(data.clone(), "\"one\""));
    let (budget, store, mut runner, clock) = setup(dir.path(), http.clone(), Io::healthy(), 3);
    assert!(store.installed(&model, &NoHooks).unwrap().is_none());
    assert!(matches!(
        store.submit(&mut runner, model.clone(), None),
        Err(EditError::Ai(ai::Error::ConsentRequired))
    ));
    assert_eq!(http.count(), 0);
    let (id, _) = submit(&store, &mut runner, &model);
    assert_eq!(terminal(&mut runner, id, &clock).state, JobState::Completed);
    assert!(http.max.load(Ordering::Relaxed) <= 65536);
    let usage = budget.snapshot().unwrap();
    assert!(usage.used > usage.limit);
    assert!(usage.owners.get(6).copied().unwrap() > data.len() as u64);
    let lease = store.installed(&model, &NoHooks).unwrap().unwrap();
    assert_eq!(
        fs::read(lease.file("weights.bin").unwrap()).unwrap(),
        data.as_ref()
    );
    let pressure = budget.reserve(ByteOwner::Export, 512 * 1024).unwrap();
    assert!(lease.file("weights.bin").unwrap().exists());
    drop(pressure);
    assert!(matches!(
        store.remove(&model),
        Err(EditError::Runtime(engine::RuntimeError::CacheBusy))
    ));
    drop(store);
    assert!(matches!(
        ArtifactCache::open(dir.path(), budget.clone(), 32),
        Err(EditError::Runtime(engine::RuntimeError::CacheBusy))
    ));
    drop(lease);
    let (mut cache, diagnostics) = ArtifactCache::open(dir.path(), budget.clone(), 32).unwrap();
    assert_eq!(diagnostics, 0);
    assert_eq!(cache.clear().unwrap(), 1);
    let lease = cache.model(&model, &NoHooks).unwrap().unwrap();
    drop(lease);
    cache.remove_model(&model).unwrap();
    assert_eq!(cache.len(), 0);
    assert_eq!(budget.snapshot().unwrap().owners.get(6), Some(&0));
}
#[test]
fn interrupted_transfer_resumes_after_cache_reopen_with_range_and_strong_etag() {
    let dir = tempfile::tempdir().unwrap();
    let http = Arc::new(Http::default());
    let (data, model) = specimen(131072);
    http.push(Reply::interrupted(data.clone()));
    let (budget, store, mut runner, clock) = setup(dir.path(), http.clone(), Io::healthy(), 1);
    let (id, _) = submit(&store, &mut runner, &model);
    assert_eq!(terminal(&mut runner, id, &clock).state, JobState::Failed);
    assert!(store.installed(&model, &NoHooks).unwrap().is_none());
    drop(store);
    drop(runner);
    http.push(Reply::range(data.clone(), 65536, "\"one\""));
    let (cache, diagnostics) = ArtifactCache::open(dir.path(), budget.clone(), 32).unwrap();
    assert_eq!(diagnostics, 0);
    let io = Arc::new(Io {
        available: 65536 + 131072,
        fail_call: usize::MAX,
        calls: AtomicUsize::new(0),
    });
    let store = ModelTransfers::new(cache, http.clone(), io);
    let mut runner = JobRunner::new(
        RunnerConfig {
            concurrency: [1; 3],
            queue: 1,
            retained: 1,
            network_attempts: 1,
            backoff: Duration::from_millis(1),
            progress_interval: Duration::from_millis(10),
        },
        budget,
        clock.clone(),
    )
    .unwrap();
    let (id, _) = submit(&store, &mut runner, &model);
    assert_eq!(terminal(&mut runner, id, &clock).state, JobState::Completed);
    assert_eq!(http.h(1, "Range"), Some("bytes=65536-".into()));
    assert_eq!(http.h(1, "If-Range"), Some("\"one\"".into()));
    assert_eq!(
        fs::read(
            store
                .installed(&model, &NoHooks)
                .unwrap()
                .unwrap()
                .file("weights.bin")
                .unwrap()
        )
        .unwrap(),
        data.as_ref()
    );
}

#[test]
fn corrupt_bundles_refuse_without_refetch_and_return_cache_custody() {
    for fault in 0..5 {
        let dir = tempfile::tempdir().unwrap();
        let http = Arc::new(Http::default());
        let (data, model) = specimen(131072);
        let (budget, store, mut runner, clock) = setup(dir.path(), http.clone(), Io::healthy(), 1);
        let bundle = dir
            .path()
            .join(ArtifactCache::model_key(&model).unwrap().filename());
        if fault == 0 {
            fs::create_dir(&bundle).unwrap();
        } else {
            http.push(Reply::interrupted(data.clone()));
            let (id, _) = submit(&store, &mut runner, &model);
            assert_eq!(terminal(&mut runner, id, &clock).state, JobState::Failed);
        }
        let sentinel = bundle.join("unrelated");
        fs::write(&sentinel, b"preserve").unwrap();
        let marker = bundle.join(".bundle");
        match fault {
            1 => fs::write(&marker, b"short").unwrap(),
            2 => File::options()
                .write(true)
                .open(bundle.join("weights.bin"))
                .unwrap()
                .set_len(1)
                .unwrap(),
            3 => {
                fs::remove_file(bundle.join("weights.bin")).unwrap();
                fs::create_dir(bundle.join("weights.bin")).unwrap();
            }
            4 => {
                let mut bytes = fs::read(&marker).unwrap();
                *bytes.get_mut(8).unwrap() ^= 1;
                fs::write(&marker, bytes).unwrap();
            }
            _ => {}
        }
        let before = fs::read(&marker).ok();
        let calls = http.count();
        let charged = budget.snapshot().unwrap().used;
        let (id, _) = submit(&store, &mut runner, &model);
        let snapshot = terminal(&mut runner, id, &clock);
        assert_eq!((snapshot.state, snapshot.attempts), (JobState::Failed, 1));
        let cause = snapshot.error.as_deref().unwrap();
        assert!(matches!(
            (cause as &(dyn std::error::Error + 'static)).downcast_ref::<EditError>(),
            Some(EditError::Runtime(engine::RuntimeError::CacheCorrupt))
        ));
        assert!(runner.take_output(id).is_none());
        assert_eq!(http.count(), calls);
        assert_eq!(fs::read(&marker).ok(), before);
        assert_eq!(fs::read(&sentinel).unwrap(), b"preserve");
        assert_eq!(budget.snapshot().unwrap().used, charged);
        if fault == 0 {
            // Only this explicit fixture repair moves a foreign directory.
            fs::rename(&bundle, dir.path().join("preserved-foreign")).unwrap();
        } else {
            store.remove(&model).unwrap();
        }
        http.push(Reply::ok(data, "\"one\""));
        let (id, _) = submit(&store, &mut runner, &model);
        assert_eq!(terminal(&mut runner, id, &clock).state, JobState::Completed);
        let lease = store.installed(&model, &NoHooks).unwrap().unwrap();
        assert_eq!(
            fs::metadata(lease.file("weights.bin").unwrap())
                .unwrap()
                .len(),
            131072
        );
        drop(lease);
        store.remove(&model).unwrap();
        drop(store);
        drop(runner);
        assert_eq!(budget.snapshot().unwrap().used, 0);
    }
}
#[test]
fn runner_retries_and_changed_entity_never_appends_unrelated_bytes() {
    for full_restart in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let http = Arc::new(Http::default());
        let (data, model) = specimen(131072);
        http.push(Reply::interrupted(data.clone()));
        if full_restart {
            http.push(Reply::ok(data.clone(), "\"two\""));
        } else {
            http.push(Reply::range(data.clone(), 65536, "\"two\""));
            http.push(Reply::ok(data.clone(), "\"two\""));
        }
        let (_, store, mut runner, clock) = setup(dir.path(), http.clone(), Io::healthy(), 3);
        let (id, _) = submit(&store, &mut runner, &model);
        let result = terminal(&mut runner, id, &clock);
        assert_eq!(result.state, JobState::Completed);
        assert_eq!(result.attempts, if full_restart { 2 } else { 3 });
        assert_eq!(http.h(1, "If-Range"), Some("\"one\"".into()));
        if !full_restart {
            assert_eq!(http.h(2, "Range"), None);
        }
        assert_eq!(
            fs::read(
                store
                    .installed(&model, &NoHooks)
                    .unwrap()
                    .unwrap()
                    .file("weights.bin")
                    .unwrap()
            )
            .unwrap(),
            data.as_ref()
        );
    }
}
#[test]
fn cancel_mid_stream_has_no_installed_marker_and_retains_a_safe_checkpoint() {
    let dir = tempfile::tempdir().unwrap();
    let http = Arc::new(Http::default());
    let (data, model) = specimen(131072);
    let (_, store, mut runner, clock) = setup(dir.path(), http.clone(), Io::healthy(), 3);
    let (id, token) = submit(&store, &mut runner, &model);
    let mut reply = Reply::ok(data, "\"one\"");
    reply.cancel = Some(token.clone());
    http.push(reply);
    assert_eq!(terminal(&mut runner, id, &clock).state, JobState::Cancelled);
    assert!(store.installed(&model, &NoHooks).unwrap().is_none());
    assert!(runner.take_output(id).is_none());
}
#[test]
fn hostile_ranges_hash_sizes_and_oversized_or_truncated_streams_never_promote() {
    for variant in 0..9 {
        let dir = tempfile::tempdir().unwrap();
        let http = Arc::new(Http::default());
        let (data, mut model) = specimen(11);
        let mut reply = Reply::ok(data.clone(), "\"one\"");
        match variant {
            0 => model.files.first_mut().unwrap().sha256 = "0".repeat(64),
            1 => reply.headers.push(("Content-Length".into(), "999".into())),
            2 => {
                reply.end = 10;
            }
            3 => {
                reply.headers.retain(|(n, _)| n != "Content-Length");
                reply.data = Arc::from(vec![7; 12]);
                reply.end = 12;
            }
            4 => {
                reply.status = 206;
                reply
                    .headers
                    .push(("Content-Range".into(), "bytes 0-10/11".into()));
            }
            5 => reply
                .headers
                .push(("Content-Encoding".into(), "gzip".into())),
            6 => {
                reply.status = 302;
                reply.headers.push(("Location".into(), "x".repeat(8193)));
            }
            7 => reply.status = 302, // a redirect without Location
            _ => {}
        }
        if variant == 8 {
            for _ in 0..6 {
                let mut hop = Reply::ok(data.clone(), "\"one\"");
                hop.status = 302;
                hop.headers
                    .push(("Location".into(), model.files[0].url.clone()));
                http.push(hop);
            }
        } else {
            http.push(reply);
        }
        let (_, store, mut runner, clock) = setup(dir.path(), http.clone(), Io::healthy(), 1);
        let (id, _) = submit(&store, &mut runner, &model);
        let snapshot = terminal(&mut runner, id, &clock);
        assert_eq!((snapshot.state, snapshot.attempts), (JobState::Failed, 1));
        let raw = snapshot.error.as_deref().unwrap();
        let typed = (raw as &(dyn std::error::Error + 'static))
            .downcast_ref::<EditError>()
            .unwrap();
        assert!(
            matches!(
                (variant, typed),
                (0 | 3, EditError::Ai(ai::Error::ModelIntegrity))
                    | (
                        2,
                        EditError::Runtime(engine::RuntimeError::NetworkTransient)
                    )
                    | (
                        1 | 4..=8,
                        EditError::Runtime(engine::RuntimeError::ModelResponse)
                    )
            ),
            "variant {variant}: {typed:?}"
        );
        if matches!(variant, 1 | 4..=8) {
            assert_eq!(raw.code(), engine::control::ErrorCode::Refused);
            assert!(!raw.transient());
            assert!(raw.source().is_none());
        }
        assert!(runner.take_output(id).is_none());
        assert_eq!(http.count(), if variant == 8 { 6 } else { 1 });
        assert!(store.installed(&model, &NoHooks).unwrap().is_none());
    }
}
#[test]
fn disk_preflight_and_mid_write_storage_full_are_typed_and_resumable() {
    for mid_write in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let http = Arc::new(Http::default());
        let (data, model) = specimen(131072);
        http.push(Reply::ok(data.clone(), "\"one\""));
        let io = Arc::new(Io {
            available: if mid_write { u64::MAX } else { 0 },
            fail_call: if mid_write { 1 } else { usize::MAX },
            calls: AtomicUsize::new(0),
        });
        let (budget, store, mut runner, clock) = setup(dir.path(), http.clone(), io, 1);
        let (id, _) = submit(&store, &mut runner, &model);
        let result = terminal(&mut runner, id, &clock);
        assert!(matches!(
            result
                .error
                .as_deref()
                .and_then(|error| (error as &(dyn std::error::Error + 'static))
                    .downcast_ref::<EditError>()),
            Some(EditError::Runtime(engine::RuntimeError::StorageFull))
        ));
        assert_eq!((result.state, result.attempts), (JobState::Failed, 1));
        let raw = result.error.as_deref().unwrap();
        assert_eq!(raw.code(), engine::control::ErrorCode::Refused);
        assert!(!raw.transient());
        assert!(raw.source().is_none());
        assert!(runner.take_output(id).is_none());
        assert!(store.installed(&model, &NoHooks).unwrap().is_none());
        if !mid_write {
            assert_eq!(http.count(), 0);
        } else {
            drop(store);
            drop(runner);
            http.push(Reply::range(data.clone(), 65536, "\"one\""));
            let (cache, _) = ArtifactCache::open(dir.path(), budget.clone(), 32).unwrap();
            let store = ModelTransfers::new(cache, http.clone(), Io::healthy());
            let mut runner = JobRunner::new(
                RunnerConfig {
                    concurrency: [1; 3],
                    queue: 1,
                    retained: 1,
                    network_attempts: 1,
                    backoff: Duration::from_millis(1),
                    progress_interval: Duration::from_millis(10),
                },
                budget,
                clock.clone(),
            )
            .unwrap();
            let (id, _) = submit(&store, &mut runner, &model);
            assert_eq!(terminal(&mut runner, id, &clock).state, JobState::Completed);
            assert_eq!(http.h(1, "Range"), Some("bytes=65536-".into()));
        }
    }
}
#[test]
fn verified_multi_file_bundle_cache_hit_needs_no_network_or_new_consent() {
    let dir = tempfile::tempdir().unwrap();
    let http = Arc::new(Http::default());
    let (data, mut model) = specimen(11);
    let mut other = model.files.first().unwrap().clone();
    other.name = "tokens.txt".into();
    other.url = "https://models.example/tokens.txt".into();
    model.files.push(other);
    http.push(Reply::ok(data.clone(), "\"one\""));
    http.push(Reply::ok(data.clone(), "\"two\""));
    let (_, store, mut runner, clock) = setup(dir.path(), http.clone(), Io::healthy(), 3);
    let (id, _) = submit(&store, &mut runner, &model);
    assert_eq!(terminal(&mut runner, id, &clock).state, JobState::Completed);
    let lease = store.installed(&model, &NoHooks).unwrap().unwrap();
    for file in &model.files {
        assert_eq!(
            fs::read(lease.file(&file.name).unwrap()).unwrap(),
            data.as_ref()
        );
    }
    let (id, _) = submit(&store, &mut runner, &model);
    assert_eq!(terminal(&mut runner, id, &clock).state, JobState::Completed);
    assert_eq!(http.count(), 2);
    fs::write(lease.file("tokens.txt").unwrap(), b"bad").unwrap();
    assert!(store.installed(&model, &NoHooks).is_err());
}
#[test]
fn consent_scope_queue_admission_and_metadata_namespace_are_strict() {
    let dir = tempfile::tempdir().unwrap();
    let http = Arc::new(Http::default());
    let (_, model) = specimen(11);
    let (_, store, mut runner, _) = setup(dir.path(), http.clone(), Io::healthy(), 3);
    let mut changed = model.clone();
    changed.license = "Other".into();
    assert!(matches!(
        store.submit(
            &mut runner,
            changed,
            Some(DownloadConsent::grant(&model).unwrap())
        ),
        Err(EditError::Ai(ai::Error::ConsentRequired))
    ));
    let key = ArtifactCache::model_key(&model).unwrap();
    let entries = || {
        fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<std::collections::BTreeSet<_>>()
    };
    let before = entries();
    let mut fillers = Vec::new();
    for _ in 0..2 {
        let (_, token) = runner
            .submit::<engine::EditError>(
                JobSpec {
                    kind: JobKind::DownloadModel,
                    class: ResourceClass::Network,
                    input: key,
                    bytes: JobBytes::default(),
                },
                |_| panic!("cancelled filler ran"),
            )
            .unwrap();
        fillers.push(token);
    }
    assert!(matches!(
        store.submit(
            &mut runner,
            model.clone(),
            Some(DownloadConsent::grant(&model).unwrap())
        ),
        Err(EditError::Runtime(engine::RuntimeError::QueueFull))
    ));
    assert_eq!(http.count(), 0);
    assert_eq!(entries(), before, "refusal wrote a checkpoint or artifact");
    assert!(store.installed(&model, &NoHooks).unwrap().is_none());
    for token in fillers {
        token.cancel();
    }
    runner.tick().unwrap();
    assert_eq!(runner.counts().0, 0);
    let (_id, token) = submit(&store, &mut runner, &model);
    assert!(matches!(
        store.submit(
            &mut runner,
            model.clone(),
            Some(DownloadConsent::grant(&model).unwrap())
        ),
        Err(EditError::Runtime(engine::RuntimeError::CacheBusy))
    ));
    token.cancel();
    runner.shutdown();
    for name in [
        ".bundle",
        "CON",
        "aux.txt",
        "weights.bin.",
        "../weights.bin",
    ] {
        let mut bad = model.clone();
        bad.files.first_mut().unwrap().name = name.into();
        assert!(DownloadConsent::grant(&bad).is_err());
    }
    assert_eq!(http.count(), 0);
}
#[test]
fn publication_seal_prevents_a_cancelled_job_from_claiming_success_after_promotion() {
    let budget = ByteBudget::new(512 * 1024).unwrap();
    let clock = Arc::new(TestClock::default());
    let mut r = JobRunner::new(
        RunnerConfig {
            concurrency: [1; 3],
            queue: 1,
            retained: 1,
            network_attempts: 1,
            backoff: Duration::from_millis(1),
            progress_interval: Duration::from_millis(10),
        },
        budget,
        clock.clone(),
    )
    .unwrap();
    let (_, model) = specimen(11);
    let token_slot = Arc::new(Mutex::new(None::<CancelToken>));
    let slot = token_slot.clone();
    let (id, token) = r
        .submit::<engine::EditError>(
            JobSpec {
                kind: JobKind::DownloadModel,
                class: ResourceClass::Network,
                input: ArtifactCache::model_key(&model).unwrap(),
                bytes: JobBytes::default(),
            },
            move |ctx| {
                ctx.commit_publication(|| {
                    slot.lock().unwrap().as_ref().unwrap().cancel();
                    assert!(!ctx.cancelled());
                    Ok(Vec::new())
                })
            },
        )
        .unwrap();
    *token_slot.lock().unwrap() = Some(token);
    assert_eq!(terminal(&mut r, id, &clock).state, JobState::Completed);
}
#[test]
fn platform_free_space_preflight_uses_the_real_filesystem() {
    let dir = tempfile::tempdir().unwrap();
    assert!(SystemModelIo.available(dir.path()).unwrap() > 0);
}

#[test]
#[ignore = "manual synthetic disk-transfer timing, no portable threshold"]
fn measure_model_streaming_publication() {
    let mut samples = Vec::new();
    for _ in 0..5 {
        let dir = tempfile::tempdir().unwrap();
        let http = Arc::new(Http::default());
        let (data, model) = specimen(2097152);
        http.push(Reply::ok(data.clone(), "\"one\""));
        let (_, store, mut runner, clock) = setup(dir.path(), http, Io::healthy(), 1);
        let start = Instant::now();
        let (id, _) = submit(&store, &mut runner, &model);
        assert_eq!(terminal(&mut runner, id, &clock).state, JobState::Completed);
        samples.push(start.elapsed().as_secs_f64());
        assert_eq!(
            fs::read(
                store
                    .installed(&model, &NoHooks)
                    .unwrap()
                    .unwrap()
                    .file("weights.bin")
                    .unwrap()
            )
            .unwrap(),
            data.as_ref()
        );
    }
    println!(
        "synthetic 2097152-byte model publication seconds: {samples:?}; five SHA/size-verified and byte-identical outputs"
    );
}

#[test]
fn backoff_requires_clock_advance_and_never_removes_a_queued_download() {
    let dir = tempfile::tempdir().unwrap();
    let http = Arc::new(Http::default());
    let (data, model) = specimen(11);
    let mut retry = Reply::ok(data.clone(), "\"one\"");
    retry.status = 503;
    http.push(retry);
    http.push(Reply::ok(data, "\"one\""));
    let (_, store, mut runner, clock) = setup(dir.path(), http.clone(), Io::healthy(), 2);
    let (id, _) = submit(&store, &mut runner, &model);
    assert!(matches!(
        store.remove(&model),
        Err(EditError::Runtime(engine::RuntimeError::CacheBusy))
    ));
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        runner.tick().unwrap();
        if runner.snapshot(id).unwrap().state == JobState::Backoff {
            break;
        }
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(1));
    }
    for _ in 0..10 {
        runner.tick().unwrap();
    }
    assert_eq!(http.count(), 1);
    assert!(matches!(
        store.remove(&model),
        Err(EditError::Runtime(engine::RuntimeError::CacheBusy))
    ));
    assert_eq!(terminal(&mut runner, id, &clock).state, JobState::Completed);
    assert_eq!(http.count(), 2);
}
struct BlockedReader {
    cursor: std::io::Cursor<Arc<[u8]>>,
    ready: std::path::PathBuf,
}
impl Read for BlockedReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if self.cursor.position() >= 5 * 1024 * 1024 {
            fs::write(&self.ready, b"checkpointed")?;
            loop {
                thread::sleep(Duration::from_secs(1));
            }
        }
        self.cursor.read(out)
    }
}
struct ChildHttp {
    data: Arc<[u8]>,
    ready: std::path::PathBuf,
}
impl DownloadTransport for ChildHttp {
    fn get(&self, _: &str, _: &[(String, String)]) -> Result<HttpStreamResponse, ai::Error> {
        Ok(HttpStreamResponse {
            status: 200,
            headers: vec![
                ("ETag".into(), "\"one\"".into()),
                ("Content-Length".into(), self.data.len().to_string()),
            ],
            body: Box::new(BlockedReader {
                cursor: std::io::Cursor::new(self.data.clone()),
                ready: self.ready.clone(),
            }),
        })
    }
}
struct ChildGuard(std::process::Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
#[test]
fn transfer_child() {
    let Some(root) = std::env::var_os("VIBECUT_TEST_MODEL_ROOT") else {
        return;
    };
    let root = std::path::PathBuf::from(root);
    let (data, model) = specimen(8388608);
    let budget = ByteBudget::new(512 * 1024).unwrap();
    let clock = Arc::new(TestClock::default());
    let (cache, _) = ArtifactCache::open(&root.join("cache"), budget.clone(), 32).unwrap();
    let store = ModelTransfers::new(
        cache,
        Arc::new(ChildHttp {
            data,
            ready: root.join("ready"),
        }),
        Arc::new(SystemModelIo),
    );
    let mut runner = JobRunner::new(
        RunnerConfig {
            concurrency: [1; 3],
            queue: 1,
            retained: 1,
            network_attempts: 1,
            backoff: Duration::from_millis(1),
            progress_interval: Duration::from_millis(10),
        },
        budget,
        clock,
    )
    .unwrap();
    submit(&store, &mut runner, &model);
    loop {
        runner.tick().unwrap();
        thread::sleep(Duration::from_millis(1));
    }
}
#[test]
fn killed_process_resumes_only_the_durable_prefix_and_publishes_atomically() {
    let root = tempfile::tempdir().unwrap();
    let mut child = ChildGuard(
        std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "providers::model_transfers::transfer_child",
                "--nocapture",
            ])
            .env("VIBECUT_TEST_MODEL_ROOT", root.path())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    while !root.path().join("ready").exists() {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(1));
    }
    let budget = ByteBudget::new(512 * 1024).unwrap();
    assert!(matches!(
        ArtifactCache::open(&root.path().join("cache"), budget.clone(), 32),
        Err(EditError::Runtime(engine::RuntimeError::CacheBusy))
    ));
    child.0.kill().unwrap();
    child.0.wait().unwrap();
    let (data, model) = specimen(8388608);
    let http = Arc::new(Http::default());
    http.push(Reply::range(data.clone(), 4 * 1024 * 1024, "\"one\""));
    let (cache, diagnostics) =
        ArtifactCache::open(&root.path().join("cache"), budget.clone(), 32).unwrap();
    assert_eq!(diagnostics, 0);
    let store = ModelTransfers::new(
        cache,
        http.clone(),
        Arc::new(Io {
            available: 3 * 1024 * 1024 + 131072,
            fail_call: usize::MAX,
            calls: AtomicUsize::new(0),
        }),
    );
    assert!(store.installed(&model, &NoHooks).unwrap().is_none());
    let clock = Arc::new(TestClock::default());
    let mut runner = JobRunner::new(
        RunnerConfig {
            concurrency: [1; 3],
            queue: 1,
            retained: 1,
            network_attempts: 1,
            backoff: Duration::from_millis(1),
            progress_interval: Duration::from_millis(10),
        },
        budget,
        clock.clone(),
    )
    .unwrap();
    let (id, _) = submit(&store, &mut runner, &model);
    assert_eq!(terminal(&mut runner, id, &clock).state, JobState::Completed);
    assert_eq!(http.h(0, "Range"), Some("bytes=4194304-".into()));
    assert_eq!(
        fs::read(
            store
                .installed(&model, &NoHooks)
                .unwrap()
                .unwrap()
                .file("weights.bin")
                .unwrap()
        )
        .unwrap(),
        data.as_ref()
    );
}

#[test]
fn malformed_resume_headers_are_refused_without_poisoning_future_restart() {
    for variant in 0..7 {
        let dir = tempfile::tempdir().unwrap();
        let http = Arc::new(Http::default());
        let (data, model) = specimen(131072);
        http.push(Reply::interrupted(data.clone()));
        let (_, store, mut runner, clock) = setup(dir.path(), http.clone(), Io::healthy(), 1);
        let (id, _) = submit(&store, &mut runner, &model);
        assert_eq!(terminal(&mut runner, id, &clock).state, JobState::Failed);
        let mut reply = Reply::range(data.clone(), 65536, "\"one\"");
        match variant {
            0 => {
                reply.headers.retain(|(n, _)| n != "Content-Range");
                reply
                    .headers
                    .push(("Content-Range".into(), "bytes 1-131071/131072".into()));
            }
            1 => {
                reply.headers.retain(|(n, _)| n != "Content-Range");
                reply
                    .headers
                    .push(("Content-Range".into(), "bytes 65536-131071/999999".into()));
            }
            2 => reply
                .headers
                .push(("Content-Range".into(), "bytes 65536-131071/131072".into())),
            3 => {
                reply.headers.retain(|(n, _)| n != "Content-Range");
                reply
                    .headers
                    .push(("Content-Range".into(), "bytes +65536-131071/131072".into()));
            }
            4 => reply.headers.retain(|(n, _)| n != "Content-Range"),
            5 => {
                reply.headers.retain(|(n, _)| n != "ETag");
                reply.headers.push(("ETag".into(), "W/\"one\"".into()));
            }
            _ => reply.headers.retain(|(n, _)| n != "ETag"),
        }
        http.push(reply);
        let (id, _) = submit(&store, &mut runner, &model);
        let snapshot = terminal(&mut runner, id, &clock);
        assert_eq!((snapshot.state, snapshot.attempts), (JobState::Failed, 1));
        let raw = snapshot.error.as_deref().unwrap();
        assert!(matches!(
            (
                variant,
                (raw as &(dyn std::error::Error + 'static)).downcast_ref::<EditError>()
            ),
            (
                0..=4,
                Some(EditError::Runtime(engine::RuntimeError::ModelResponse))
            ) | (
                5..=6,
                Some(EditError::Runtime(engine::RuntimeError::NetworkTransient))
            )
        ));
        assert_eq!(raw.transient(), variant >= 5);
        assert!(runner.take_output(id).is_none());
        assert!(store.installed(&model, &NoHooks).unwrap().is_none());
        http.push(Reply::ok(data.clone(), "\"one\""));
        let (id, _) = submit(&store, &mut runner, &model);
        assert_eq!(terminal(&mut runner, id, &clock).state, JobState::Completed);
        assert_eq!(http.count(), 3);
    }
}
#[test]
fn weak_initial_validator_restarts_instead_of_sending_if_range() {
    let dir = tempfile::tempdir().unwrap();
    let http = Arc::new(Http::default());
    let (data, model) = specimen(131072);
    let mut first = Reply::interrupted(data.clone());
    first.headers.retain(|(n, _)| n != "ETag");
    first.headers.push(("ETag".into(), "W/\"one\"".into()));
    http.push(first);
    http.push(Reply::ok(data, "\"one\""));
    let (_, store, mut runner, clock) = setup(dir.path(), http.clone(), Io::healthy(), 2);
    let (id, _) = submit(&store, &mut runner, &model);
    assert_eq!(terminal(&mut runner, id, &clock).state, JobState::Completed);
    assert_eq!(http.h(1, "Range"), None);
    assert_eq!(http.h(1, "If-Range"), None);
}
#[test]
fn bounded_safe_redirects_keep_the_original_consent_scope() {
    for downgrade in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let http = Arc::new(Http::default());
        let (data, model) = specimen(11);
        let mut redirect = Reply::ok(Arc::from(Vec::<u8>::new()), "\"one\"");
        redirect.status = 302;
        redirect.headers.push((
            "Location".into(),
            if downgrade {
                "http://cdn.example/weights"
            } else {
                "https://cdn.example/weights?signature=test"
            }
            .into(),
        ));
        http.push(redirect);
        http.push(Reply::ok(data, "\"one\""));
        let (_, store, mut runner, clock) = setup(dir.path(), http.clone(), Io::healthy(), 1);
        let (id, _) = submit(&store, &mut runner, &model);
        let result = terminal(&mut runner, id, &clock);
        assert_eq!(
            result.state,
            if downgrade {
                JobState::Failed
            } else {
                JobState::Completed
            }
        );
        assert_eq!(http.count(), if downgrade { 1 } else { 2 });
    }
}
#[cfg(unix)]
#[test]
fn checkpoint_symlinks_cannot_redirect_model_writes() {
    let dir = tempfile::tempdir().unwrap();
    let http = Arc::new(Http::default());
    let (data, model) = specimen(131072);
    http.push(Reply::interrupted(data));
    let (_, store, mut runner, clock) = setup(dir.path(), http.clone(), Io::healthy(), 1);
    let (id, _) = submit(&store, &mut runner, &model);
    assert_eq!(terminal(&mut runner, id, &clock).state, JobState::Failed);
    let bundle = dir
        .path()
        .join(ArtifactCache::model_key(&model).unwrap().filename());
    fs::remove_file(bundle.join("weights.bin")).unwrap();
    let victim = dir.path().join("unrelated");
    fs::write(&victim, b"preserve").unwrap();
    std::os::unix::fs::symlink(&victim, bundle.join("weights.bin")).unwrap();
    let (id, _) = submit(&store, &mut runner, &model);
    assert_eq!(terminal(&mut runner, id, &clock).state, JobState::Failed);
    assert_eq!(fs::read(&victim).unwrap(), b"preserve");
    assert_eq!(http.count(), 1);
}
#[test]
fn a_sealed_publication_is_never_retried_even_for_a_transient_result() {
    let budget = ByteBudget::new(512 * 1024).unwrap();
    let clock = Arc::new(TestClock::default());
    let mut runner = JobRunner::new(
        RunnerConfig {
            concurrency: [1; 3],
            queue: 1,
            retained: 1,
            network_attempts: 3,
            backoff: Duration::from_millis(1),
            progress_interval: Duration::from_millis(10),
        },
        budget,
        clock.clone(),
    )
    .unwrap();
    let (_, model) = specimen(11);
    let (id, _) = runner
        .submit(
            JobSpec {
                kind: JobKind::DownloadModel,
                class: ResourceClass::Network,
                input: ArtifactCache::model_key(&model).unwrap(),
                bytes: JobBytes::default(),
            },
            |ctx| {
                ctx.commit_publication(|| {
                    Err(EditError::Runtime(engine::RuntimeError::NetworkTransient))
                })
            },
        )
        .unwrap();
    let result = terminal(&mut runner, id, &clock);
    assert_eq!(result.state, JobState::Failed);
    assert_eq!(result.attempts, 1);
    let raw = result.error.as_deref().unwrap();
    assert!(raw.transient());
    assert_eq!(raw.code(), engine::control::ErrorCode::Job);
    assert!(raw.source().is_none());
    assert!(matches!(
        (raw as &(dyn std::error::Error + 'static)).downcast_ref::<EditError>(),
        Some(EditError::Runtime(engine::RuntimeError::NetworkTransient))
    ));
    assert!(runner.take_output(id).is_none());
}

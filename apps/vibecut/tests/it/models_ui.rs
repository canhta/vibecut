#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
//! Model-manager UI evidence through real widgets and injected engine transfers.
use ai::{
    models::{Model, ModelFile},
    transport::{DownloadTransport, HttpStreamResponse},
};
use egui_kittest::{
    Harness,
    kittest::{By, NodeT, Queryable},
};
use engine::{
    ByteBudget,
    cache::ArtifactCache,
    jobs::JobState,
    models::{ModelIo, ModelTransfers},
};
use sha2::{Digest, Sha256};
use std::{
    collections::VecDeque,
    fs::File,
    io::{self, Read, Write},
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use vibecut::{
    Editor,
    catalog::Preferences,
    egui,
    model_manager::{ModelManager, ModelProblem},
    playback::{Files, Playback},
};

struct Reply {
    status: u16,
    headers: Vec<(String, String)>,
    data: Arc<[u8]>,
    start: usize,
    end: usize,
    fail: bool,
    delay: Duration,
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
            delay: Duration::ZERO,
        }
    }
    fn interrupted(data: Arc<[u8]>, end: usize) -> Self {
        let mut reply = Self::ok(data, "\"one\"");
        reply.end = end;
        reply.fail = true;
        reply
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
            delay: Duration::ZERO,
        }
    }
    fn slow(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }
}
struct Body {
    data: Arc<[u8]>,
    position: usize,
    end: usize,
    fail: bool,
    delay: Duration,
}
impl Read for Body {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if !self.delay.is_zero() {
            std::thread::sleep(self.delay);
        }
        if self.position >= self.end {
            return if self.fail {
                Err(io::Error::new(
                    io::ErrorKind::ConnectionReset,
                    "injected interruption",
                ))
            } else {
                Ok(0)
            };
        }
        let n = out.len().min(self.end - self.position).min(65536);
        out.get_mut(..n)
            .ok_or_else(|| io::Error::other("body buffer"))?
            .copy_from_slice(
                self.data
                    .get(self.position..self.position + n)
                    .ok_or_else(|| io::Error::other("body range"))?,
            );
        self.position += n;
        Ok(n)
    }
}
type Request = (String, Vec<(String, String)>);
#[derive(Default)]
struct Http {
    replies: Mutex<VecDeque<Reply>>,
    requests: Mutex<Vec<Request>>,
}
impl Http {
    fn push(&self, reply: Reply) {
        self.replies.lock().unwrap().push_back(reply);
    }
    fn count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }
    fn header(&self, index: usize, name: &str) -> Option<String> {
        self.requests
            .lock()
            .unwrap()
            .get(index)
            .and_then(|(_, headers)| {
                headers
                    .iter()
                    .find(|(key, _)| key.eq_ignore_ascii_case(name))
                    .map(|(_, value)| value.clone())
            })
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
        let reply = self
            .replies
            .lock()
            .unwrap()
            .pop_front()
            .ok_or(ai::Error::Network)?;
        Ok(HttpStreamResponse {
            status: reply.status,
            headers: reply.headers,
            body: Box::new(Body {
                data: reply.data,
                position: reply.start,
                end: reply.end,
                fail: reply.fail,
                delay: reply.delay,
            }),
        })
    }
}
struct Io {
    available: u64,
}
impl ModelIo for Io {
    fn available(&self, _: &Path) -> io::Result<u64> {
        Ok(self.available)
    }
    fn write(&self, file: &mut File, data: &[u8]) -> io::Result<()> {
        file.write_all(data)
    }
}
fn digest(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
fn model(id: &str, license: &str, data: &[u8]) -> Model {
    Model {
        id: id.into(),
        license: license.into(),
        license_url: format!("https://models.example/{id}/LICENSE"),
        source: format!("https://models.example/{id}"),
        languages: vec![captions::LanguageTag::parse("vi").unwrap()],
        files: vec![ModelFile {
            name: "weights.bin".into(),
            url: format!("https://models.example/{id}/weights.bin"),
            sha256: digest(data),
            size: data.len() as u64,
        }],
    }
}
fn manager(dir: &Path, models: Vec<Model>, http: Arc<Http>, io: Arc<Io>) -> ModelManager {
    let budget = ByteBudget::new(4 << 20).unwrap();
    let (cache, diagnostics) = ArtifactCache::open(dir, budget.clone(), 16).unwrap();
    assert_eq!(diagnostics, 0);
    ModelManager::with_transfers(budget, models, ModelTransfers::new(cache, http, io)).unwrap()
}
fn harness(editor: Editor) -> Harness<'static, Editor> {
    Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .build_ui_state(|ui, editor: &mut Editor| editor.ui(ui), editor)
}
fn wait(harness: &mut Harness<'_, Editor>, what: &str, ready: impl Fn(&Editor) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        harness.run_steps(1);
        if ready(harness.state()) {
            return;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn click(harness: &mut Harness<'_, Editor>, id: &str) {
    harness
        .get(By::new().predicate(|node| node.author_id() == Some(id)))
        .click();
    harness.run_steps(3);
}
fn inspect(harness: &Harness<'_, Editor>) -> vibecut::model_manager::ModelInspection {
    harness.state().inspect().unwrap().models.unwrap()
}
fn open(manager: ModelManager, id: &str) -> Editor {
    let mut editor = editor();
    editor.set_model_manager(manager);
    editor.open_model(id).unwrap();
    editor
}
fn wait_check(harness: &mut Harness<'_, Editor>) {
    wait(harness, "installed check", |editor| {
        !editor.inspect().unwrap().models.unwrap().checking
    });
    // The centered modal settles its size for a couple of frames; click after that.
    harness.run_steps(6);
}
fn wait_terminal(harness: &mut Harness<'_, Editor>) {
    wait(harness, "download terminal state", |editor| {
        let inspection = editor.inspect().unwrap().models.unwrap();
        inspection.job.as_ref().is_some_and(|job| {
            matches!(
                job.state,
                JobState::Completed | JobState::Cancelled | JobState::Failed
            )
        }) && !inspection.checking
    });
}
fn editor() -> Editor {
    let session = engine::Session::new(
        project::Project::empty(project::ProjectId(23), "Models".into()).unwrap(),
        10,
    )
    .unwrap();
    let playback = Playback::new(
        &session,
        ByteBudget::new(64 << 20).unwrap(),
        Arc::new(Files),
    )
    .unwrap();
    Editor::new(session, playback, Preferences::default())
}

#[test]
fn opening_lists_licence_scope_and_never_transfers_before_download_click() {
    let dir = tempfile::tempdir().unwrap();
    let data = vec![7u8; 64 * 1024];
    let permissive = model("whisper-large-v3-turbo", "MIT", &data);
    let restricted = model("fixture-restricted", "CDLA-Permissive-2.0", &data);
    let http = Arc::new(Http::default());
    http.push(Reply::ok(Arc::from(data), "\"one\""));
    let manager = manager(
        dir.path(),
        vec![permissive, restricted],
        http.clone(),
        Arc::new(Io {
            available: u64::MAX,
        }),
    );
    let mut harness = harness(open(manager, "whisper-large-v3-turbo"));
    wait_check(&mut harness);
    assert!(harness.query_all(By::new().label("Models")).count() > 0);
    assert!(harness.query_by_label("Suggested default").is_some());
    assert_eq!(http.count(), 0, "opening the manager must not transfer");
    click(&mut harness, "models.row.fixture-restricted");
    assert_eq!(
        inspect(&harness).selected.as_deref(),
        Some("fixture-restricted")
    );
    assert!(harness.query_by_label("Suggested default").is_none());
    assert!(
        harness
            .query_by_label(
                "This licence is not eligible as a suggested default. Review the exact licence and source before opting in."
            )
            .is_some(),
        "the restriction is shown before consent"
    );
    assert_eq!(http.count(), 0, "selecting a model must not transfer");
    let disabled = harness.get(
        By::new().predicate(|node| node.author_id() == Some("models.download.fixture-restricted")),
    );
    assert!(disabled.accesskit_node().is_disabled());
    harness
        .get(
            By::new()
                .predicate(|node| node.author_id() == Some("models.accept.fixture-restricted")),
        )
        .click_accesskit();
    harness.run_steps(3);
    assert_eq!(http.count(), 0, "accepting the licence must not transfer");
    assert!(
        inspect(&harness)
            .accepted
            .contains(&"fixture-restricted".into())
    );
    click(&mut harness, "models.download.fixture-restricted");
    wait_terminal(&mut harness);
    assert_eq!(http.count(), 1, "only the download click starts transfer");
    let inspection = inspect(&harness);
    assert_eq!(inspection.job.unwrap().state, JobState::Completed);
    assert!(inspection.installed.contains(&"fixture-restricted".into()));
    assert!(harness.query_all(By::new().label("Installed")).count() > 0);
}

#[test]
fn cancel_stops_the_active_download() {
    let dir = tempfile::tempdir().unwrap();
    let data = vec![9u8; 512 * 1024];
    let fixture = model("fixture-cancel", "MIT", &data);
    let http = Arc::new(Http::default());
    http.push(Reply::ok(Arc::from(data), "\"one\"").slow(Duration::from_millis(200)));
    let manager = manager(
        dir.path(),
        vec![fixture],
        http,
        Arc::new(Io {
            available: u64::MAX,
        }),
    );
    let mut harness = harness(open(manager, "fixture-cancel"));
    wait_check(&mut harness);
    click(&mut harness, "models.download.fixture-cancel");
    wait(&mut harness, "running download", |editor| {
        editor.inspect().unwrap().models.unwrap().active.is_some()
    });
    click(&mut harness, "models.cancel");
    wait_terminal(&mut harness);
    let inspection = inspect(&harness);
    assert_eq!(inspection.job.unwrap().state, JobState::Cancelled);
    assert_eq!(inspection.problem, Some(ModelProblem::Cancelled));
}

#[test]
fn interrupted_download_resumes_with_range_and_strong_etag() {
    let dir = tempfile::tempdir().unwrap();
    let data: Arc<[u8]> = Arc::from(vec![5u8; 128 * 1024]);
    let fixture = model("fixture-resume", "MIT", &data);
    let http = Arc::new(Http::default());
    http.push(Reply::interrupted(data.clone(), 64 * 1024));
    http.push(Reply::range(data, 64 * 1024, "\"one\""));
    let manager = manager(
        dir.path(),
        vec![fixture],
        http.clone(),
        Arc::new(Io {
            available: u64::MAX,
        }),
    );
    let mut harness = harness(open(manager, "fixture-resume"));
    wait_check(&mut harness);
    click(&mut harness, "models.download.fixture-resume");
    wait_terminal(&mut harness);
    let inspection = inspect(&harness);
    assert_eq!(inspection.problem, Some(ModelProblem::Network));
    assert!(
        inspection.resume.contains(&"fixture-resume".into()),
        "an interrupted transfer offers Resume"
    );
    click(&mut harness, "models.download.fixture-resume");
    wait_terminal(&mut harness);
    assert_eq!(inspect(&harness).job.unwrap().state, JobState::Completed);
    assert_eq!(http.count(), 2);
    assert_eq!(http.header(1, "range").as_deref(), Some("bytes=65536-"));
    assert_eq!(http.header(1, "if-range").as_deref(), Some("\"one\""));
}

#[test]
fn checksum_failure_is_discarded_and_reported() {
    let dir = tempfile::tempdir().unwrap();
    let expected = vec![3u8; 64 * 1024];
    let fixture = model("fixture-hash", "MIT", &expected);
    let http = Arc::new(Http::default());
    http.push(Reply::ok(Arc::from(vec![4u8; expected.len()]), "\"one\""));
    let manager = manager(
        dir.path(),
        vec![fixture],
        http,
        Arc::new(Io {
            available: u64::MAX,
        }),
    );
    let mut harness = harness(open(manager, "fixture-hash"));
    wait_check(&mut harness);
    click(&mut harness, "models.download.fixture-hash");
    wait_terminal(&mut harness);
    let inspection = inspect(&harness);
    assert_eq!(inspection.job.unwrap().state, JobState::Failed);
    assert_eq!(inspection.problem, Some(ModelProblem::Integrity));
    assert!(
        harness
            .query_by_label("The downloaded file failed its checksum and was discarded. Retry.")
            .is_some()
    );
    assert!(inspection.installed.is_empty());
}

#[test]
fn disk_full_is_reported_before_any_bytes_are_written() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = model("fixture-disk", "MIT", &vec![6u8; 64 * 1024]);
    let http = Arc::new(Http::default());
    let manager = manager(
        dir.path(),
        vec![fixture],
        http.clone(),
        Arc::new(Io { available: 1 }),
    );
    let mut harness = harness(open(manager, "fixture-disk"));
    wait_check(&mut harness);
    click(&mut harness, "models.download.fixture-disk");
    wait_terminal(&mut harness);
    let inspection = inspect(&harness);
    assert_eq!(inspection.job.unwrap().state, JobState::Failed);
    assert_eq!(inspection.problem, Some(ModelProblem::StorageFull));
    assert!(
        harness
            .query_by_label("Not enough disk space for these weights. Free space and retry.")
            .is_some()
    );
    assert_eq!(http.count(), 0, "disk preflight precedes transfer");
}

#[test]
fn dropping_the_manager_releases_the_cache_before_returning() {
    let dir = tempfile::tempdir().unwrap();
    let models = (0..128)
        .map(|index| model(&format!("fixture-check-{index}"), "MIT", &[1]))
        .collect();
    let manager = manager(
        dir.path(),
        models,
        Arc::new(Http::default()),
        Arc::new(Io {
            available: u64::MAX,
        }),
    );
    // Closing/replacing the editor must finish its checker before another
    // model owner can open the same cache; no timing allowance after Drop.
    drop(manager);
    let reopened = ArtifactCache::open(dir.path(), ByteBudget::new(4 << 20).unwrap(), 16);
    assert!(
        reopened.is_ok(),
        "the checker must release its cache lock on drop"
    );
}

#[test]
fn editor_header_opens_the_manager_without_a_transfer() {
    let dir = tempfile::tempdir().unwrap();
    let data = vec![8u8; 32 * 1024];
    let fixture = model("fixture-header", "MIT", &data);
    let http = Arc::new(Http::default());
    let manager = manager(
        dir.path(),
        vec![fixture],
        http.clone(),
        Arc::new(Io {
            available: u64::MAX,
        }),
    );
    let mut editor = editor();
    editor.set_model_manager(manager);
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .build_ui_state(|ui, editor: &mut Editor| editor.ui(ui), editor);
    harness.run_steps(2);
    harness
        .get(By::new().predicate(|node| node.author_id() == Some("models.open")))
        .click();
    harness.run_steps(2);
    let inspection = harness.state().inspect().unwrap();
    assert!(inspection.models.unwrap().open);
    assert_eq!(http.count(), 0);
}

#[test]
fn model_manager_screenshot_is_available_at_both_scales() {
    let dir = tempfile::tempdir().unwrap();
    for scale in [1.0, 2.0] {
        let manager = manager(
            dir.path(),
            ai::models::catalog().unwrap(),
            Arc::new(Http::default()),
            Arc::new(Io {
                available: u64::MAX,
            }),
        );
        let mut editor = editor();
        editor.set_model_manager(manager);
        editor.open_models().unwrap();
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1280.0, 800.0))
            .with_pixels_per_point(scale)
            .build_ui_state(|ui, editor: &mut Editor| editor.ui(ui), editor);
        wait_check(&mut harness);
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(format!("../../target/p0-23-model-manager-{scale}x.png"));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        harness.render().unwrap().save(&path).unwrap();
        println!("model manager screenshot: {}", path.display());
    }
}

#[test]
fn packaged_inputs_never_include_model_weights() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let names: std::collections::BTreeSet<String> = ai::models::catalog()
        .unwrap()
        .iter()
        .flat_map(|model| model.files.iter().map(|file| file.name.clone()))
        .collect();
    for directory in ["assets", "packaging"] {
        assert!(
            !contains_file(&root.join(directory), &names),
            "weights must not be packaged inputs"
        );
    }
}
fn contains_file(path: &Path, names: &std::collections::BTreeSet<String>) -> bool {
    let Ok(entries) = std::fs::read_dir(path) else {
        return false;
    };
    entries.flatten().any(|entry| {
        let path = entry.path();
        if path.is_dir() {
            contains_file(&path, names)
        } else {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| names.contains(name))
        }
    })
}

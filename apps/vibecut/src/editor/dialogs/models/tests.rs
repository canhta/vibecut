//! Drives the model manager dialog alone over the one-area bench.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
use super::model_manager::{ModelManager, ModelProblem};
use crate::editor::frame::bench::Bench;
use crate::editor::{Editor, egui};
use crate::playback::{Files, Playback};
use crate::preferences::Preferences;
use ai::{
    models::{Model, ModelFile},
    transport::{DownloadTransport, HttpStreamResponse},
};
use egui_kittest::kittest::{By, NodeT, Queryable};
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
fn bench(editor: Editor) -> Bench<super::super::State, super::super::Action> {
    Bench::hosted(
        editor,
        |editor| std::mem::take(&mut editor.dialogs),
        |editor, state| editor.dialogs = state,
        super::super::show,
        super::super::apply,
        egui::vec2(1280.0, 800.0),
    )
}
fn open(manager: ModelManager, id: &str) -> Editor {
    let mut editor = editor();
    editor.set_model_manager(manager);
    editor.open_model(id).unwrap();
    editor
}
fn inspect(
    bench: &Bench<super::super::State, super::super::Action>,
) -> crate::model_manager::ModelInspection {
    bench.editor().inspect().unwrap().models.unwrap()
}
fn wait(
    bench: &mut Bench<super::super::State, super::super::Action>,
    what: &str,
    ready: impl Fn(&Editor) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        bench.editor_mut().tick();
        bench.step();
        if ready(bench.editor()) {
            return;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn wait_check(bench: &mut Bench<super::super::State, super::super::Action>) {
    wait(bench, "installed check", |editor| {
        !editor.inspect().unwrap().models.unwrap().checking
    });
    // The centered modal settles its size for a couple of frames; click after that.
    bench.harness_mut().run_steps(6);
}
fn wait_terminal(bench: &mut Bench<super::super::State, super::super::Action>) {
    wait(bench, "download terminal state", |editor| {
        let inspection = editor.inspect().unwrap().models.unwrap();
        inspection.job.as_ref().is_some_and(|job| {
            matches!(
                job.state,
                JobState::Completed | JobState::Cancelled | JobState::Failed
            )
        }) && !inspection.checking
    });
}
/// The AccessKit node id egui assigned to the widget behind one automation id.
fn node_id(bench: &Bench<super::super::State, super::super::Action>, id: &str) -> String {
    format!(
        "{:?}",
        bench
            .harness()
            .get(By::new().predicate(|node| node.author_id() == Some(id)))
            .accesskit_node()
            .id()
    )
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
    let mut bench = bench(open(manager, "whisper-large-v3-turbo"));
    wait_check(&mut bench);
    assert!(bench.harness().query_all(By::new().label("Models")).count() > 0);
    assert!(
        bench
            .harness()
            .query_by_label("Suggested default")
            .is_some()
    );
    assert_eq!(http.count(), 0, "opening the manager must not transfer");
    bench.click("models.row.fixture-restricted");
    assert_eq!(
        inspect(&bench).selected.as_deref(),
        Some("fixture-restricted")
    );
    assert!(
        bench
            .harness()
            .query_by_label("Suggested default")
            .is_none()
    );
    assert!(
        bench
            .harness()
            .query_by_label("Check the licence before downloading.")
            .is_some(),
        "the restriction is shown before consent"
    );
    assert_eq!(http.count(), 0, "selecting a model must not transfer");
    let disabled = bench.harness().get(
        By::new().predicate(|node| node.author_id() == Some("models.download.fixture-restricted")),
    );
    assert!(disabled.accesskit_node().is_disabled());
    bench
        .harness_mut()
        .get(
            By::new()
                .predicate(|node| node.author_id() == Some("models.accept.fixture-restricted")),
        )
        .click_accesskit();
    bench.harness_mut().run_steps(3);
    assert_eq!(http.count(), 0, "accepting the licence must not transfer");
    assert!(
        inspect(&bench)
            .accepted
            .contains(&"fixture-restricted".into())
    );
    bench.click("models.download.fixture-restricted");
    wait_terminal(&mut bench);
    assert_eq!(http.count(), 1, "only the download click starts transfer");
    let inspection = inspect(&bench);
    assert_eq!(inspection.job.unwrap().state, JobState::Completed);
    assert!(inspection.installed.contains(&"fixture-restricted".into()));
    assert!(
        bench
            .harness()
            .query_all(By::new().label("Installed"))
            .count()
            > 0
    );
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
    let mut bench = bench(open(manager, "fixture-cancel"));
    wait_check(&mut bench);
    bench.click("models.download.fixture-cancel");
    wait(&mut bench, "running download", |editor| {
        editor.inspect().unwrap().models.unwrap().active.is_some()
    });
    bench.click("models.cancel");
    wait_terminal(&mut bench);
    let inspection = inspect(&bench);
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
    let mut bench = bench(open(manager, "fixture-resume"));
    wait_check(&mut bench);
    bench.click("models.download.fixture-resume");
    wait_terminal(&mut bench);
    let inspection = inspect(&bench);
    assert_eq!(inspection.problem, Some(ModelProblem::Network));
    assert!(
        inspection.resume.contains(&"fixture-resume".into()),
        "an interrupted transfer offers Resume"
    );
    bench.click("models.download.fixture-resume");
    wait_terminal(&mut bench);
    assert_eq!(inspect(&bench).job.unwrap().state, JobState::Completed);
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
    let mut bench = bench(open(manager, "fixture-hash"));
    wait_check(&mut bench);
    bench.click("models.download.fixture-hash");
    wait_terminal(&mut bench);
    let inspection = inspect(&bench);
    assert_eq!(inspection.job.unwrap().state, JobState::Failed);
    assert_eq!(inspection.problem, Some(ModelProblem::Integrity));
    assert!(
        bench
            .harness()
            .query_by_label("Download was damaged. Try again.")
            .is_some()
    );
    assert!(inspection.installed.is_empty());
}

#[test]
fn footer_rows_do_not_shift_the_model_action_controls() {
    // The footer draws the installed-check spinner, a problem row and the job
    // progress above its buttons. They appear asynchronously, so a press that
    // straddles the update must still resolve to the same widget.
    let dir = tempfile::tempdir().unwrap();
    let expected = vec![3u8; 64 * 1024];
    let fixture = model("fixture-footer", "MIT", &expected);
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
    let mut bench = bench(open(manager, "fixture-footer"));
    wait_check(&mut bench);
    let close = node_id(&bench, "models.close");
    let download = node_id(&bench, "models.download.fixture-footer");

    bench.click("models.download.fixture-footer");
    wait_terminal(&mut bench);
    assert_eq!(inspect(&bench).problem, Some(ModelProblem::Integrity));
    assert!(
        bench
            .harness()
            .query_by_label("Download was damaged. Try again.")
            .is_some(),
        "the problem row is drawn above the buttons"
    );
    assert_eq!(
        node_id(&bench, "models.close"),
        close,
        "Close must keep its widget id when the problem row appears"
    );
    assert_eq!(
        node_id(&bench, "models.download.fixture-footer"),
        download,
        "the download control must keep its widget id when the problem row appears"
    );
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
    let mut bench = bench(open(manager, "fixture-disk"));
    wait_check(&mut bench);
    bench.click("models.download.fixture-disk");
    wait_terminal(&mut bench);
    let inspection = inspect(&bench);
    assert_eq!(inspection.job.unwrap().state, JobState::Failed);
    assert_eq!(inspection.problem, Some(ModelProblem::StorageFull));
    assert!(
        bench
            .harness()
            .query_by_label("Not enough space. Free some and retry.")
            .is_some()
    );
    assert_eq!(http.count(), 0, "disk preflight precedes transfer");
}

#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
//! Model-manager evidence that needs a whole window, the app owner or the repo tree.
use ai::models::{Model, ModelFile};
use egui_kittest::kittest::{By, Queryable};
use engine::{
    ByteBudget,
    cache::ArtifactCache,
    models::{ModelIo, ModelTransfers},
};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{self, Write},
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use transport::{DownloadTransport, HttpStreamResponse};
use vibecut::{
    Editor, egui,
    model_manager::ModelManager,
    preferences::Preferences,
    services::playback::{Files, Playback},
};

type Request = (String, Vec<(String, String)>);
#[derive(Default)]
struct Http {
    requests: Mutex<Vec<Request>>,
}
impl Http {
    fn count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }
}
impl DownloadTransport for Http {
    fn get(
        &self,
        url: &str,
        headers: &[(String, String)],
    ) -> Result<HttpStreamResponse, transport::Error> {
        self.requests
            .lock()
            .unwrap()
            .push((url.into(), headers.into()));
        Err(transport::Error::Network)
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
    let mut harness = crate::gpu::builder()
        .with_size(egui::vec2(1280.0, 800.0))
        .build_ui_state(|ui, editor: &mut Editor| editor.ui(ui), editor);
    harness.run_steps(2);
    harness
        .get(By::new().predicate(|node| node.author_id() == Some("window.menu")))
        .click();
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
        let mut harness = crate::gpu::builder()
            .with_size(egui::vec2(1280.0, 800.0))
            .with_pixels_per_point(scale)
            .build_ui_state(|ui, editor: &mut Editor| editor.ui(ui), editor);
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            harness.run_steps(1);
            if !harness
                .state()
                .inspect()
                .unwrap()
                .models
                .as_ref()
                .is_some_and(|models| models.checking)
            {
                break;
            }
            assert!(Instant::now() < deadline, "model check did not finish");
            std::thread::sleep(Duration::from_millis(2));
        }
        harness.run_steps(4);
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

#[test]
fn project_windows_share_one_model_service_across_editor_close() {
    use vibecut::workspace::Workspace;
    fn idle(app: &mut Workspace) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while app.busy() {
            app.tick();
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    let root = tempfile::tempdir().unwrap();
    let cache = root.path().join("model-cache");
    let http = Arc::new(Http::default());
    let models = manager(
        &cache,
        vec![model("fixture-shared", "MIT", &[1; 64])],
        http.clone(),
        Arc::new(Io {
            available: u64::MAX,
        }),
    );
    let mut app =
        Workspace::new(root.path().join("recovery"), Preferences::default(), None).unwrap();
    idle(&mut app);
    app.create().unwrap();
    idle(&mut app);
    let first = app.focused_project().unwrap();
    app.create().unwrap();
    idle(&mut app);
    let second = app.focused_project().unwrap();
    app.editor_mut(first).unwrap().set_model_manager(models);
    app.editor_mut(first)
        .unwrap()
        .open_model("fixture-shared")
        .unwrap();
    app.focus(first).unwrap();
    let mut h = crate::gpu::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .build_ui_state(
            |ui, a: &mut Workspace| {
                a.tick();
                if let Some(id) = a.focused_project() {
                    a.window_ui(id, ui);
                }
            },
            app,
        );
    h.run_steps(6);
    assert!(
        h.query_all(By::new().predicate(|n| n.author_id() == Some("models.row.fixture-shared")))
            .count()
            == 1
    );
    h.state_mut()
        .windows
        .get_mut(&first)
        .unwrap()
        .persist(None, true, true, true)
        .unwrap();
    idle(h.state_mut());
    h.state_mut().focus(second).unwrap();
    h.run_steps(6);
    assert!(!h.state().windows.contains_key(&first));
    assert!(
        h.query_all(By::new().predicate(|n| n.author_id() == Some("models.row.fixture-shared")))
            .count()
            == 1,
        "the injected app model service survives the window that first presented it"
    );
    assert_eq!(
        http.count(),
        0,
        "moving between windows never grants transfer consent"
    );
    assert!(
        ArtifactCache::open(&cache, ByteBudget::new(64 << 20).unwrap(), 32).is_err(),
        "the app still owns the cache lease"
    );
    drop(h);
    assert!(ArtifactCache::open(&cache, ByteBudget::new(64 << 20).unwrap(), 32).is_ok());
}

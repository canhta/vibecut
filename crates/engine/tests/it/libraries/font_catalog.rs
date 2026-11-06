#![allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "font catalog integration fixtures"
)]
//! Font catalog behaviour through the store, the job runner and `text` layout.

use ai::transport::{DownloadTransport, HttpStreamResponse};
use engine::{
    ByteBudget, EditError,
    fonts::{
        FontEntry, FontError, FontLoadState, FontOrigin, FontState, FontStore, GoogleFamily,
        GoogleFile, ScannedFace, curated_google_families, scan_directory,
    },
    jobs::{Clock, JobId, JobRunner, JobState, RunnerConfig, SystemClock},
};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

pub const FIXTURE: &[u8] = include_bytes!("../../fixtures/fonts/VibeCutFixtureSans-Regular.ttf");
const SERIF: &[u8] = include_bytes!("../../../../../assets/fonts/NotoSerif-Regular.ttf");
const LICENSE: &[u8] = b"SIL Open Font License 1.1 (test copy)\n";

fn sha(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// A catalog family whose single file is `payload` and whose licence is [`LICENSE`].
pub fn family(name: &str, slug: &str, payload: &[u8]) -> GoogleFamily {
    GoogleFamily {
        family: name.into(),
        slug: slug.into(),
        author: "Test Author".into(),
        source: format!("https://fonts.test/{slug}"),
        license: "OFL-1.1".into(),
        license_url: format!("https://fonts.test/{slug}/OFL.txt"),
        license_sha256: sha(LICENSE),
        license_size: LICENSE.len() as u64,
        files: vec![GoogleFile {
            name: format!("{slug}-Regular.ttf"),
            url: format!("https://fonts.test/{slug}/{slug}-Regular.ttf"),
            sha256: sha(payload),
            size: payload.len() as u64,
            weight: 400,
            italic: false,
        }],
    }
}

/// Serves URLs from a map and counts requests; unknown URLs fail like an offline network.
#[derive(Default)]
pub struct Server {
    pub bodies: Mutex<BTreeMap<String, Vec<u8>>>,
    pub status: Mutex<BTreeMap<String, u16>>,
    pub calls: AtomicUsize,
    /// When set, bodies yield one byte, announce it, then wait for the release.
    pub gate: Mutex<Option<Arc<Gate>>>,
}
pub struct Gate {
    pub started: std::sync::mpsc::SyncSender<()>,
    pub release: Mutex<std::sync::mpsc::Receiver<()>>,
}
impl Server {
    pub fn serving(family: &GoogleFamily, payload: &[u8]) -> Arc<Self> {
        let server = Arc::new(Self::default());
        server
            .bodies
            .lock()
            .unwrap()
            .insert(family.files[0].url.clone(), payload.to_vec());
        server
            .bodies
            .lock()
            .unwrap()
            .insert(family.license_url.clone(), LICENSE.to_vec());
        server
    }
}
struct Body {
    data: io::Cursor<Vec<u8>>,
    gate: Option<Arc<Gate>>,
    first: bool,
}
impl Read for Body {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if let Some(gate) = &self.gate {
            if self.first {
                self.first = false;
                let read = self.data.read(&mut out[..1])?;
                let _ = gate.started.send(());
                return Ok(read);
            }
            let _ = gate
                .release
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(10));
        }
        self.data.read(out)
    }
}
impl DownloadTransport for Server {
    fn get(&self, url: &str, _: &[(String, String)]) -> Result<HttpStreamResponse, ai::Error> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let data = self
            .bodies
            .lock()
            .unwrap()
            .get(url)
            .cloned()
            .ok_or(ai::Error::Network)?;
        let status = self.status.lock().unwrap().get(url).copied().unwrap_or(200);
        Ok(HttpStreamResponse {
            status,
            headers: Vec::new(),
            body: Box::new(Body {
                data: io::Cursor::new(data),
                gate: self.gate.lock().unwrap().clone(),
                first: true,
            }),
        })
    }
}

/// A store with one downloadable family, `Lobster`, served from memory and cached in a
/// directory that outlives the test.
pub fn example_store() -> Arc<FontStore> {
    let catalog = family("Lobster", "lobster", FIXTURE);
    let server = Server::serving(&catalog, FIXTURE);
    let root = tempfile::tempdir().unwrap().keep();
    Arc::new(FontStore::new(root, vec![catalog], Vec::new, server).unwrap())
}

fn runner() -> JobRunner {
    JobRunner::new(
        RunnerConfig {
            concurrency: [1, 1, 1],
            queue: 16,
            retained: 16,
            network_attempts: 1,
            backoff: Duration::from_millis(1),
            progress_interval: Duration::from_millis(10),
        },
        ByteBudget::new(512 << 20).unwrap(),
        Arc::new(SystemClock::default()) as Arc<dyn Clock>,
    )
    .unwrap()
}
fn finish(runner: &mut JobRunner, id: JobId) -> JobState {
    for _ in 0..20_000 {
        runner.tick().unwrap();
        if let Some(snapshot) = runner.snapshot(id)
            && matches!(
                snapshot.state,
                JobState::Completed | JobState::Failed | JobState::Cancelled
            )
        {
            return snapshot.state;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    panic!("job deadline");
}

/// Drive the runner (it admits queued work only when ticked) until the body reports it began.
fn wait_started(runner: &mut JobRunner, started: &std::sync::mpsc::Receiver<()>) {
    for _ in 0..20_000 {
        runner.tick().unwrap();
        if started.try_recv().is_ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    panic!("download never started");
}

fn store(
    root: &Path,
    google: Vec<GoogleFamily>,
    system: Vec<ScannedFace>,
    transport: Arc<dyn DownloadTransport>,
) -> FontStore {
    FontStore::new(
        root.to_path_buf(),
        google,
        move || system.clone(),
        transport,
    )
    .unwrap()
}
fn names(entries: &[FontEntry]) -> Vec<&str> {
    entries.iter().map(|e| e.family.as_str()).collect()
}
fn download(store: &FontStore, family: &str) -> JobState {
    let mut runner = runner();
    let (id, _) = store.submit_download(&mut runner, family, true).unwrap();
    let state = finish(&mut runner, id);
    runner.shutdown();
    state
}
fn entries_below(root: &Path) -> Vec<String> {
    let mut found: Vec<String> = fs::read_dir(root)
        .map(|dir| {
            dir.map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    found.sort();
    found
}
fn laid_out_faces(family: &str) -> (bool, Vec<usize>) {
    let layout = text::layout(
        "Hello Aa",
        &text::LayoutOptions {
            family,
            ..Default::default()
        },
    )
    .unwrap();
    (
        layout.missing_font,
        layout.glyphs.iter().map(|g| g.face).collect(),
    )
}
fn offline() -> Arc<Server> {
    Arc::new(Server::default())
}

#[test]
fn directory_scan_finds_nested_fonts_and_skips_junk() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("VibeCutFixtureSans-Regular.ttf"), FIXTURE).unwrap();
    fs::create_dir(dir.path().join("nested")).unwrap();
    fs::write(dir.path().join("nested").join("Serif.ttf"), SERIF).unwrap();
    fs::write(dir.path().join("broken.ttf"), b"not a font").unwrap();
    fs::write(dir.path().join("notes.txt"), b"hello").unwrap();

    let faces = scan_directory(dir.path());
    let found: Vec<(&str, u16, bool)> = faces
        .iter()
        .map(|f| (f.family.as_str(), f.weight, f.italic))
        .collect();
    assert_eq!(
        found,
        [
            ("Noto Serif", 400, false),
            ("VibeCut Fixture Sans", 400, false)
        ]
    );
    assert!(faces.iter().all(|f| f.path.starts_with(dir.path())));
    assert!(scan_directory(&dir.path().join("missing")).is_empty());
}

#[test]
fn search_orders_prefix_matches_first_and_one_entry_per_family() {
    let root = tempfile::tempdir().unwrap();
    let system = |family: &str| ScannedFace {
        family: family.into(),
        weight: 400,
        italic: false,
        path: PathBuf::from("unused.ttf"),
        index: 0,
    };
    let store = store(
        root.path(),
        vec![
            family("Sansa", "sansa", FIXTURE),
            family("Alpha Sans", "alphasans", FIXTURE),
            family("Zeta", "zeta", FIXTURE),
        ],
        // Duplicates of bundled and catalog names collapse into the stronger origin.
        vec![
            system("Mono Sans"),
            system("inter"),
            system("Zeta"),
            system("Mono Sans"),
        ],
        offline(),
    );
    let sans = store.list(Some("  SANS ")).unwrap();
    assert_eq!(
        names(&sans),
        ["Sansa", "Alpha Sans", "Mono Sans", "VibeCut Sans CJK"]
    );
    assert_eq!(sans[0].state, FontState::NotDownloaded);
    assert_eq!(sans[0].origin, FontOrigin::Google);
    assert_eq!(sans[0].license.as_deref(), Some("OFL-1.1"));
    assert_eq!(sans[0].source.as_deref(), Some("https://fonts.test/sansa"));
    assert_eq!(
        sans[0].download_bytes,
        Some(FIXTURE.len() as u64 + LICENSE.len() as u64)
    );
    assert_eq!(sans[2].origin, FontOrigin::System);

    let all = store.list(None).unwrap();
    assert_eq!(
        names(&all),
        [
            "Alpha Sans",
            "Inter",
            "Mono Sans",
            "Noto Serif",
            "Sansa",
            "VibeCut Sans CJK",
            "Zeta"
        ]
    );
    let origin = |name: &str| all.iter().find(|e| e.family == name).unwrap().origin;
    assert_eq!(origin("Inter"), FontOrigin::Bundled);
    assert_eq!(origin("Zeta"), FontOrigin::System);
    let inter = all.iter().find(|e| e.family == "Inter").unwrap();
    assert_eq!(
        inter.styles,
        ["Regular", "Italic", "Medium", "SemiBold", "Bold"]
    );
    assert_eq!(inter.license.as_deref(), Some("OFL-1.1"));
    assert!(store.list(Some("no such family")).unwrap().is_empty());
    assert_eq!(
        store.list(Some("")).unwrap().len(),
        all.len(),
        "an empty query lists everything"
    );
}

#[test]
fn system_fonts_load_lazily_and_unknown_families_fall_back_deterministically() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("VibeCutFixtureSans-Regular.ttf");
    fs::write(&file, FIXTURE).unwrap();
    let scans = Arc::new(AtomicUsize::new(0));
    let counter = scans.clone();
    let scan_dir = dir.path().to_path_buf();
    let root = tempfile::tempdir().unwrap();
    let store = FontStore::new(
        root.path().to_path_buf(),
        Vec::new(),
        move || {
            counter.fetch_add(1, Ordering::SeqCst);
            scan_directory(&scan_dir)
        },
        offline(),
    )
    .unwrap();
    assert_eq!(scans.load(Ordering::SeqCst), 0, "no scan before first use");

    let listed = store.list(Some("fixture")).unwrap();
    assert_eq!(names(&listed), ["VibeCut Fixture Sans"]);
    assert_eq!(scans.load(Ordering::SeqCst), 1);

    // Listing read no font bytes: damage the file and loading it is what fails.
    fs::write(&file, b"damaged").unwrap();
    assert!(matches!(
        store.load("VibeCut Fixture Sans"),
        Err(EditError::Font(FontError::Face(
            text::FontError::Unreadable
        )))
    ));
    assert!(!text::is_available("VibeCut Fixture Sans"));
    fs::write(&file, FIXTURE).unwrap();

    let loaded = store.load("vibecut fixture sans").unwrap();
    assert_eq!(loaded.state, FontLoadState::Loaded);
    assert_eq!(loaded.origin, Some(FontOrigin::System));
    assert_eq!(loaded.effective_family, "VibeCut Fixture Sans");
    assert_eq!(loaded.styles, ["Regular"]);
    assert_eq!(scans.load(Ordering::SeqCst), 1, "one scan per store");
    let (missing, faces) = laid_out_faces("VibeCut Fixture Sans");
    assert!(!missing);
    let registered = text::resolve("VibeCut Fixture Sans", "Regular").face;
    assert!(faces.contains(&registered));

    // A bundled family needs no registration; an unknown one reports a typed state and the
    // layout substitutes the default family, identically every time.
    let bundled = store.load("Noto Serif").unwrap();
    assert_eq!(
        (bundled.state, bundled.origin),
        (FontLoadState::Loaded, Some(FontOrigin::Bundled))
    );
    let unknown = store.load("No Such Family").unwrap();
    assert_eq!(unknown.state, FontLoadState::Unknown);
    assert_eq!(unknown.effective_family, "Inter");
    let inter = text::resolve("Inter", "Regular").face;
    for _ in 0..3 {
        let (missing, faces) = laid_out_faces("No Such Family");
        assert!(missing);
        assert!(faces.iter().all(|face| *face == inter));
    }
    for hostile in ["", "  ", &"x".repeat(257)] {
        assert!(matches!(store.load(hostile), Err(EditError::Invalid(_))));
    }
}

#[test]
fn warm_cache_renders_offline_and_cold_cache_reports_not_downloaded() {
    let root = tempfile::tempdir().unwrap();
    let catalog = family("Offline Sample", "offlinesample", FIXTURE);

    // Cold: a typed state, the default family in layout, and no network access.
    let cold_server = offline();
    let cold = store(
        root.path(),
        vec![catalog.clone()],
        Vec::new(),
        cold_server.clone(),
    );
    let loaded = cold.load("Offline Sample").unwrap();
    assert_eq!(loaded.state, FontLoadState::NotDownloaded);
    assert_eq!(loaded.origin, Some(FontOrigin::Google));
    assert_eq!(loaded.effective_family, "Inter");
    assert!(laid_out_faces("Offline Sample").0);
    assert_eq!(cold_server.calls.load(Ordering::SeqCst), 0);

    // Explicit download.
    let server = Server::serving(&catalog, FIXTURE);
    let online = store(
        root.path(),
        vec![catalog.clone()],
        Vec::new(),
        server.clone(),
    );
    assert_eq!(download(&online, "offline sample"), JobState::Completed);
    assert_eq!(server.calls.load(Ordering::SeqCst), 2);
    assert_eq!(entries_below(root.path()), ["offlinesample"]);
    let entry = root.path().join("offlinesample");
    assert_eq!(
        entries_below(&entry),
        [
            "OFL.txt",
            "offlinesample-Regular.ttf",
            "offlinesample-Regular.ttf.attribution"
        ]
    );
    let record = fs::read_to_string(entry.join("offlinesample-Regular.ttf.attribution")).unwrap();
    for expected in [
        "asset: offlinesample-Regular.ttf",
        "source: https://fonts.test/offlinesample/offlinesample-Regular.ttf",
        "license: OFL-1.1",
        "license-file: OFL.txt",
        "added: p1-parity-28",
    ] {
        assert!(record.contains(expected), "{expected} in {record}");
    }
    assert_eq!(fs::read(entry.join("OFL.txt")).unwrap(), LICENSE);

    // Warm and offline: a new store whose transport fails and counts any request.
    let dead = offline();
    let warm = store(root.path(), vec![catalog.clone()], Vec::new(), dead.clone());
    let listed = warm.list(Some("offline")).unwrap();
    assert_eq!(listed[0].state, FontState::Available);
    assert_eq!(listed[0].download_bytes, None);
    let loaded = warm.load("Offline Sample").unwrap();
    assert_eq!(loaded.state, FontLoadState::Loaded);
    assert_eq!(loaded.effective_family, "Offline Sample");
    let (missing, faces) = laid_out_faces("Offline Sample");
    assert!(!missing);
    let registered = text::resolve("Offline Sample", "Regular").face;
    assert!(faces.contains(&registered));
    assert_eq!(
        dead.calls.load(Ordering::SeqCst),
        0,
        "render never uses the network"
    );

    // Repeating the download of a valid cache is a no-op.
    let before = server.calls.load(Ordering::SeqCst);
    assert_eq!(download(&online, "Offline Sample"), JobState::Completed);
    assert_eq!(server.calls.load(Ordering::SeqCst), before);
}

#[test]
fn cache_entries_are_invalidated_by_hash_size_and_missing_records() {
    let root = tempfile::tempdir().unwrap();
    let catalog = family("Hash Sample", "hashsample", FIXTURE);
    let server = Server::serving(&catalog, FIXTURE);
    let store = store(root.path(), vec![catalog], Vec::new(), server.clone());
    assert_eq!(download(&store, "Hash Sample"), JobState::Completed);
    assert_eq!(
        store.load("Hash Sample").unwrap().state,
        FontLoadState::Loaded
    );

    let entry = root.path().join("hashsample");
    let font = entry.join("hashsample-Regular.ttf");
    let mut flipped = FIXTURE.to_vec();
    flipped[100] ^= 0xff;
    type Damage<'a> = Box<dyn Fn() + 'a>;
    let tamper: [(&str, Damage); 5] = [
        (
            "same-size byte flip",
            Box::new(|| fs::write(&font, &flipped).unwrap()),
        ),
        (
            "truncation",
            Box::new(|| fs::write(&font, &FIXTURE[..100]).unwrap()),
        ),
        (
            "missing font file",
            Box::new(|| fs::remove_file(&font).unwrap()),
        ),
        (
            "missing licence file",
            Box::new(|| fs::remove_file(entry.join("OFL.txt")).unwrap()),
        ),
        (
            "missing attribution record",
            Box::new(|| fs::remove_file(entry.join("hashsample-Regular.ttf.attribution")).unwrap()),
        ),
    ];
    for (label, damage) in tamper {
        damage();
        let loaded = store.load("Hash Sample").unwrap();
        assert_eq!(loaded.state, FontLoadState::CacheInvalid, "{label}");
        assert_eq!(loaded.effective_family, "Inter", "{label}");
        let listed = store.list(Some("hash sample")).unwrap();
        assert_eq!(listed[0].state, FontState::NotDownloaded, "{label}");
        // The next explicit download replaces the entry.
        assert_eq!(
            download(&store, "Hash Sample"),
            JobState::Completed,
            "{label}"
        );
        assert_eq!(
            store.load("Hash Sample").unwrap().state,
            FontLoadState::Loaded,
            "{label}"
        );
    }
}

#[test]
fn download_needs_acceptance_a_catalog_family_and_one_job_per_family() {
    let root = tempfile::tempdir().unwrap();
    let catalog = family("Gate Sample", "gatesample", FIXTURE);
    let server = Server::serving(&catalog, FIXTURE);
    let store = store(root.path(), vec![catalog], Vec::new(), server.clone());
    let mut runner = runner();

    assert!(matches!(
        store.submit_download(&mut runner, "Gate Sample", false),
        Err(EditError::ConsentRequired)
    ));
    for refused in ["Inter", "Not In Catalog", ""] {
        assert!(matches!(
            store.submit_download(&mut runner, refused, true),
            Err(EditError::Font(FontError::NotDownloadable))
        ));
    }
    assert_eq!(server.calls.load(Ordering::SeqCst), 0);
    assert!(entries_below(root.path()).is_empty());

    let (started, started_rx) = std::sync::mpsc::sync_channel(1);
    let (release, release_rx) = std::sync::mpsc::channel();
    *server.gate.lock().unwrap() = Some(Arc::new(Gate {
        started,
        release: Mutex::new(release_rx),
    }));
    let (id, _) = store
        .submit_download(&mut runner, "Gate Sample", true)
        .unwrap();
    wait_started(&mut runner, &started_rx);
    assert!(matches!(
        store.submit_download(&mut runner, "gate sample", true),
        Err(EditError::Font(FontError::DownloadActive))
    ));
    drop(release);
    assert_eq!(finish(&mut runner, id), JobState::Completed);
    // The slot frees once the job ends.
    *server.gate.lock().unwrap() = None;
    assert!(
        store
            .submit_download(&mut runner, "Gate Sample", true)
            .is_ok()
    );
    runner.shutdown();
}

#[test]
fn failed_or_cancelled_downloads_leave_no_cache_entry() {
    let corrupt = {
        let mut bytes = FIXTURE.to_vec();
        bytes[10] ^= 1;
        bytes
    };
    type Rig = Box<dyn Fn(&GoogleFamily, &Server)>;
    let cases: [(&str, Rig); 6] = [
        (
            "wrong font bytes",
            Box::new(move |g, s| {
                s.bodies
                    .lock()
                    .unwrap()
                    .insert(g.files[0].url.clone(), corrupt.clone());
            }),
        ),
        (
            "truncated font",
            Box::new(|g, s| {
                s.bodies.lock().unwrap().insert(
                    g.files[0].url.clone(),
                    FIXTURE[..FIXTURE.len() - 1].to_vec(),
                );
            }),
        ),
        (
            "oversized font",
            Box::new(|g, s| {
                let mut long = FIXTURE.to_vec();
                long.push(0);
                s.bodies
                    .lock()
                    .unwrap()
                    .insert(g.files[0].url.clone(), long);
            }),
        ),
        (
            "licence fails after the font was written",
            Box::new(|g, s| {
                s.bodies
                    .lock()
                    .unwrap()
                    .insert(g.license_url.clone(), b"altered".to_vec());
            }),
        ),
        (
            "server error",
            Box::new(|g, s| {
                s.status.lock().unwrap().insert(g.files[0].url.clone(), 500);
            }),
        ),
        (
            "unreachable licence",
            Box::new(|g, s| {
                s.bodies.lock().unwrap().remove(&g.license_url);
            }),
        ),
    ];
    for (label, rig) in cases {
        let root = tempfile::tempdir().unwrap();
        let catalog = family("Failure Sample", "failuresample", FIXTURE);
        let server = Server::serving(&catalog, FIXTURE);
        rig(&catalog, &server);
        let store = store(root.path(), vec![catalog], Vec::new(), server);
        assert_eq!(
            download(&store, "Failure Sample"),
            JobState::Failed,
            "{label}"
        );
        assert!(entries_below(root.path()).is_empty(), "{label}");
        assert_eq!(
            store.load("Failure Sample").unwrap().state,
            FontLoadState::NotDownloaded,
            "{label}"
        );
    }

    // Cancelled while the body is streaming.
    let root = tempfile::tempdir().unwrap();
    let catalog = family("Cancel Sample", "cancelsample", FIXTURE);
    let server = Server::serving(&catalog, FIXTURE);
    let (started, started_rx) = std::sync::mpsc::sync_channel(1);
    let (release, release_rx) = std::sync::mpsc::channel();
    *server.gate.lock().unwrap() = Some(Arc::new(Gate {
        started,
        release: Mutex::new(release_rx),
    }));
    let store = store(root.path(), vec![catalog], Vec::new(), server);
    let mut runner = runner();
    let (id, token) = store
        .submit_download(&mut runner, "Cancel Sample", true)
        .unwrap();
    wait_started(&mut runner, &started_rx);
    assert!(
        entries_below(root.path())
            .iter()
            .all(|name| name.starts_with(".partial-")),
        "only private staging exists mid-download"
    );
    token.cancel();
    drop(release);
    assert_eq!(finish(&mut runner, id), JobState::Cancelled);
    assert!(entries_below(root.path()).is_empty());
    runner.shutdown();
}

#[test]
fn curated_catalog_pins_licensed_files_and_is_accepted_by_the_store() {
    let curated = curated_google_families();
    assert!(curated.len() >= 6);
    for g in &curated {
        assert_eq!(g.license, "OFL-1.1", "{}", g.family);
        assert!(
            g.source
                .starts_with("https://github.com/google/fonts/tree/")
        );
        assert!(g.license_url.ends_with("/OFL.txt"));
        assert!(
            g.files
                .iter()
                .all(|f| f.url.starts_with("https://raw.githubusercontent.com/"))
        );
    }
    let root = tempfile::tempdir().unwrap();
    let store = store(root.path(), curated.clone(), Vec::new(), offline());
    let lobster = store.list(Some("lobster")).unwrap();
    assert_eq!(lobster[0].state, FontState::NotDownloaded);
    assert!(lobster[0].download_bytes.unwrap() > 400_000);

    // Hostile catalog data is refused at construction.
    let mut path_escape = curated[0].clone();
    path_escape.files[0].name = "../escape.ttf".into();
    let mut insecure = curated[0].clone();
    insecure.files[0].url = "http://fonts.test/a.ttf".into();
    let mut short_hash = curated[0].clone();
    short_hash.files[0].sha256 = "abc".into();
    let mut bad_slug = curated[0].clone();
    bad_slug.slug = "Not/Ok".into();
    let duplicate = vec![curated[0].clone(), curated[0].clone()];
    for bad in [
        vec![path_escape],
        vec![insecure],
        vec![short_hash],
        vec![bad_slug],
        duplicate,
    ] {
        assert!(FontStore::new(root.path().to_path_buf(), bad, Vec::new, offline()).is_err());
    }
}

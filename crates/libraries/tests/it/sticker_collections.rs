#![allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "sticker collection integration fixtures"
)]
//! Downloadable sticker collections: catalog pins, the SVG admission check, cache states, the
//! download job, search and the provenance of bundled and cached files.

use engine::{
    ByteBudget, StickerError,
    control::Error as EngineError,
    jobs::{Clock, JobId, JobRunner, JobState, RunnerConfig, SystemClock},
};
use libraries::stickers::{
    Category, Collection, CollectionState, Entry, MAX_SVG_BYTES, StickerStore, check_svg,
    curated_collections,
};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::{self, Read},
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use transport::{DownloadTransport, HttpStreamResponse};

const SVG_NS: &str = "http://www.w3.org/2000/svg";
const LICENSE: &[u8] = b"MIT License (test copy)\n";
const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

fn sha(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn clean(shape: &str) -> Vec<u8> {
    format!(
        "<svg xmlns=\"{SVG_NS}\" viewBox=\"0 0 24 24\"><defs><linearGradient id=\"a\"><stop offset=\"0\" stop-color=\"#fff\"/></linearGradient></defs><path d=\"{shape}\" fill=\"url(#a)\" style=\"fill:url(#a);\"/><use href=\"#a\"/></svg>"
    )
    .into_bytes()
}

/// A collection of the given files; each is pinned by its own bytes.
fn demo(files: &[(&str, &[u8])]) -> Collection {
    Collection {
        id: "demo".into(),
        name: "Demo Icons".into(),
        category: Category::Icons,
        author: "Demo Author".into(),
        repo: "owner/demo".into(),
        commit: COMMIT.into(),
        license: "MIT".into(),
        license_path: "LICENSE".into(),
        license_size: LICENSE.len() as u64,
        license_sha256: sha(LICENSE),
        entries: files
            .iter()
            .map(|(value, bytes)| Entry {
                value: (*value).into(),
                name: format!("Demo {value}"),
                keywords: vec![format!("kw-{value}")],
                path: format!("icons/{value}.svg"),
                size: bytes.len() as u64,
                sha256: sha(bytes),
            })
            .collect(),
    }
}

/// Serves bodies by URL and counts requests; an unknown URL fails like an offline network.
#[derive(Default)]
struct Server {
    bodies: Mutex<BTreeMap<String, Vec<u8>>>,
    calls: AtomicUsize,
}

impl Server {
    fn serving(collection: &Collection, files: &[(&str, &[u8])]) -> Arc<Self> {
        let server = Arc::new(Self::default());
        {
            let mut bodies = server.bodies.lock().unwrap();
            bodies.insert(
                collection.raw_url(&collection.license_path),
                LICENSE.to_vec(),
            );
            for (value, bytes) in files {
                bodies.insert(
                    collection.raw_url(&format!("icons/{value}.svg")),
                    bytes.to_vec(),
                );
            }
        }
        server
    }
}

impl DownloadTransport for Server {
    fn get(
        &self,
        url: &str,
        _: &[(String, String)],
    ) -> Result<HttpStreamResponse, transport::Error> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let data = self
            .bodies
            .lock()
            .unwrap()
            .get(url)
            .cloned()
            .ok_or(transport::Error::Network)?;
        let body: Box<dyn Read + Send> = Box::new(io::Cursor::new(data));
        Ok(HttpStreamResponse {
            status: 200,
            headers: Vec::new(),
            body,
        })
    }
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

fn store(root: &Path, collection: Collection, server: Arc<Server>) -> StickerStore {
    StickerStore::new(root.to_path_buf(), vec![collection], server).unwrap()
}

fn download(store: &StickerStore, runner: &mut JobRunner) -> JobState {
    let (id, _) = store.submit_download(runner, "demo", true).unwrap();
    finish(runner, id)
}

fn names_below(root: &Path) -> Vec<String> {
    let mut found: Vec<String> = fs::read_dir(root)
        .map(|dir| {
            dir.map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    found.sort();
    found
}

/// Files below `dir` without a complete sidecar, sidecars without a file and licence files that
/// do not resolve. `repo_root` resolves `license-file` for bundled assets; cached sidecars name
/// the licence beside them.
fn unrecorded(dir: &Path, repo_root: Option<&Path>) -> Vec<String> {
    const KEYS: [&str; 10] = [
        "asset",
        "title",
        "author",
        "source",
        "license",
        "license-file",
        "added",
        "notes",
        "collection",
        "commit",
    ];
    let mut problems = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(folder) = stack.pop() {
        for item in fs::read_dir(&folder).unwrap() {
            let path = item.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if name == "README.md" {
                continue;
            }
            if let Some(base) = name.strip_suffix(".attribution") {
                if !folder.join(base).is_file() {
                    problems.push(format!("{name}: sidecar without a file"));
                }
                continue;
            }
            let mut sidecar = path.as_os_str().to_os_string();
            sidecar.push(".attribution");
            let Ok(text) = fs::read_to_string(&sidecar) else {
                problems.push(format!("{name}: no sidecar"));
                continue;
            };
            let value = |key: &str| {
                text.lines()
                    .find_map(|line| line.strip_prefix(&format!("{key}:")))
                    .map(|v| v.trim().to_owned())
                    .filter(|v| !v.is_empty())
            };
            for key in KEYS {
                if value(key).is_none() {
                    problems.push(format!("{name}: empty or missing {key}"));
                }
            }
            if let Some(license_file) = value("license-file") {
                let resolved = repo_root.map_or_else(
                    || folder.join(&license_file),
                    |root| root.join(&license_file),
                );
                if !resolved.is_file() {
                    problems.push(format!("{name}: licence file {license_file} is missing"));
                }
            }
            if value("commit").is_some_and(|commit| {
                commit.len() != 40 || !commit.bytes().all(|b| b.is_ascii_hexdigit())
            }) {
                problems.push(format!("{name}: commit is not a full hash"));
            }
        }
    }
    problems.sort();
    problems
}

fn is_unsafe(result: Result<(), StickerError>) -> bool {
    matches!(result, Err(StickerError::UnsafeSvg(_)))
}

#[test]
fn curated_collections_are_pinned_licensed_and_credited() {
    let collections = curated_collections();
    assert_eq!(
        collections
            .iter()
            .map(|c| c.id.as_str())
            .collect::<Vec<_>>(),
        ["noto", "lucide", "phosphor"]
    );
    assert_eq!(
        collections
            .iter()
            .map(|c| c.license.as_str())
            .collect::<Vec<_>>(),
        ["Apache-2.0", "ISC", "MIT"],
        "only Apache-2.0, ISC and MIT collections are admitted; no CC BY set"
    );
    assert_eq!(
        collections.iter().map(|c| c.entries.len()).sum::<usize>(),
        420
    );
    let dir = tempfile::tempdir().unwrap();
    let store = StickerStore::new(
        dir.path().to_path_buf(),
        collections.clone(),
        Arc::new(Server::default()),
    )
    .unwrap();
    for collection in &collections {
        assert!(matches!(
            collection.category,
            Category::Emoji | Category::Icons
        ));
        assert_eq!(collection.commit.len(), 40);
        assert!(
            collection
                .raw_url(&collection.license_path)
                .contains(&collection.commit)
        );
        for entry in &collection.entries {
            assert_eq!(entry.sha256.len(), 64);
            assert!(entry.size > 0 && entry.size <= MAX_SVG_BYTES as u64);
            assert!(entry.path.ends_with(".svg"));
        }
        // Every collection has a credit line naming it, its author, its licence and its home.
        let first = &collection.entries[0].value;
        let credit = project::StickerParams::new(format!("{}:{first}", collection.id), 256, 256)
            .credit()
            .unwrap_or_else(|| panic!("{} has no export credit", collection.id));
        for part in [
            collection.name.as_str(),
            collection.author.as_str(),
            collection.license.as_str(),
            collection.source().as_str(),
        ] {
            assert!(credit.contains(part), "{credit} lacks {part}");
        }
        assert!(
            store
                .resolve(&format!("{}:{first}", collection.id))
                .is_some()
        );
    }
    assert!(
        project::StickerParams::new("flags:VN", 640, 480)
            .credit()
            .is_some_and(|line| line.contains("MIT"))
    );
    assert!(
        project::StickerParams::new("shapes:star", 8, 8)
            .credit()
            .is_none()
    );
    // A malformed catalog is refused: bad hash, duplicate value, duplicate id.
    let mut bad_hash = collections[1].clone();
    bad_hash.entries[0].sha256 = "xyz".into();
    let mut duplicate_value = collections[1].clone();
    duplicate_value.entries[1].value = duplicate_value.entries[0].value.clone();
    for bad in [vec![bad_hash], vec![duplicate_value]] {
        assert!(matches!(
            StickerStore::new(dir.path().to_path_buf(), bad, Arc::new(Server::default())),
            Err(EngineError::Sticker(StickerError::Catalog))
        ));
    }
    assert!(matches!(
        StickerStore::new(
            dir.path().to_path_buf(),
            vec![collections[1].clone(), collections[1].clone()],
            Arc::new(Server::default())
        ),
        Err(EngineError::Sticker(StickerError::Catalog))
    ));
}

#[test]
fn admission_check_refuses_hostile_svg_and_keeps_clean_bytes() {
    assert!(check_svg(&clean("M0 0h24v24H0z")).is_ok());
    let wrap = |inner: &str| format!("<svg xmlns=\"{SVG_NS}\">{inner}</svg>").into_bytes();
    let deep = format!("{}{}", "<g>".repeat(70), "</g>".repeat(70));
    let big = format!(
        "<svg xmlns=\"{SVG_NS}\"><!-- {} --></svg>",
        "a".repeat(MAX_SVG_BYTES)
    );
    let hostile: Vec<(&str, Vec<u8>)> = vec![
        ("script element", wrap("<script>alert(1)</script>")),
        ("foreignObject", wrap("<foreignObject><p/></foreignObject>")),
        ("image element", wrap("<image href=\"https://evil.example/a.png\"/>")),
        ("style element", wrap("<style>@import url(https://evil.example/a.css);</style>")),
        ("animate element", wrap("<animate attributeName=\"href\" values=\"javascript:alert(1)\"/>")),
        ("event handler", wrap("<path onclick=\"x()\" d=\"M0 0\"/>")),
        ("external href", wrap("<use href=\"https://evil.example/x.svg#a\"/>")),
        (
            "xlink file href",
            format!(
                "<svg xmlns=\"{SVG_NS}\" xmlns:xlink=\"http://www.w3.org/1999/xlink\"><use xlink:href=\"file:///etc/passwd\"/></svg>"
            )
            .into_bytes(),
        ),
        ("external url()", wrap("<path fill=\"url(https://evil.example/a)\"/>")),
        ("quoted external url()", wrap("<path style=\"fill:url('https://evil.example/a')\"/>")),
        ("javascript value", wrap("<path style=\"background:javascript:alert(1)\"/>")),
        ("data value", wrap("<path fill=\"data:text/plain,x\"/>")),
        (
            "entity declaration",
            format!(
                "<!DOCTYPE svg [<!ENTITY x \"y\">]><svg xmlns=\"{SVG_NS}\">&x;</svg>"
            )
            .into_bytes(),
        ),
        (
            "stylesheet instruction",
            format!("<?xml-stylesheet href=\"evil.css\"?><svg xmlns=\"{SVG_NS}\"/>").into_bytes(),
        ),
        ("no namespace", b"<svg><path d=\"M0 0\"/></svg>".to_vec()),
        (
            "html root",
            b"<html xmlns=\"http://www.w3.org/1999/xhtml\"/>".to_vec(),
        ),
        (
            "foreign namespace child",
            format!(
                "<svg xmlns=\"{SVG_NS}\" xmlns:h=\"http://www.w3.org/1999/xhtml\"><h:script/></svg>"
            )
            .into_bytes(),
        ),
        ("nesting", wrap(&deep)),
        ("oversize", big.into_bytes()),
        ("not UTF-8", vec![0xff, 0xfe, 0x00]),
        ("not XML", b"plain text".to_vec()),
    ];
    for (name, bytes) in hostile {
        assert!(is_unsafe(check_svg(&bytes)), "{name} must be refused");
    }
}

#[test]
fn bundled_and_cached_files_must_be_recorded() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let bundled = repo.join("assets/stickers");
    assert_eq!(unrecorded(&bundled, Some(&repo)), Vec::<String>::new());

    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("loose.svg"), clean("M0 0")).unwrap();
    fs::write(dir.path().join("orphan.svg.attribution"), "asset: x\n").unwrap();
    fs::write(dir.path().join("half.svg"), clean("M1 1")).unwrap();
    fs::write(dir.path().join("half.svg.attribution"), "asset: half.svg\n").unwrap();
    let found = unrecorded(dir.path(), None);
    assert!(
        found.contains(&"loose.svg: no sidecar".to_owned()),
        "{found:?}"
    );
    assert!(found.contains(&"orphan.svg.attribution: sidecar without a file".to_owned()));
    assert!(found.contains(&"half.svg: empty or missing commit".to_owned()));
}

#[test]
fn download_verifies_records_publishes_and_searches() {
    let heart_svg = clean("M1 1h8");
    let star_svg = clean("M2 2h9");
    let files: [(&str, &[u8]); 2] = [("heart", &heart_svg), ("star", &star_svg)];
    let files_owned: Vec<(&str, Vec<u8>)> = files.iter().map(|(n, b)| (*n, b.to_vec())).collect();
    let collection = demo(&files);
    let server = Server::serving(&collection, &files);
    let root = tempfile::tempdir().unwrap();
    let store = store(root.path(), collection.clone(), server.clone());
    let mut runner = runner();

    // Nothing is fetched or created before the user acts; search reports the collection.
    assert_eq!(store.state("demo").unwrap(), CollectionState::NotDownloaded);
    let page = store.search("heart", Some(Category::Icons), 10).unwrap();
    assert!(page.items.is_empty());
    assert_eq!(page.unavailable.len(), 1);
    assert_eq!(page.unavailable[0].collection, "demo");
    assert_eq!(page.unavailable[0].state, CollectionState::NotDownloaded);
    assert_eq!(server.calls.load(Ordering::SeqCst), 0);
    assert!(names_below(root.path()).is_empty());

    // Consent, an unknown collection and a second job for the same collection are refused.
    assert!(matches!(
        store.submit_download(&mut runner, "demo", false),
        Err(EngineError::Providers(
            engine::providers::Error::ConsentRequired
        ))
    ));
    assert!(matches!(
        store.submit_download(&mut runner, "nope", true),
        Err(EngineError::Sticker(StickerError::NotDownloadable))
    ));
    assert_eq!(server.calls.load(Ordering::SeqCst), 0);
    let (id, _) = store.submit_download(&mut runner, "demo", true).unwrap();
    assert!(matches!(
        store.submit_download(&mut runner, "demo", true),
        Err(EngineError::Sticker(StickerError::DownloadActive))
    ));
    assert_eq!(finish(&mut runner, id), JobState::Completed);

    // Published: exactly the pinned files with complete records, bytes read back verified.
    assert_eq!(store.state("demo").unwrap(), CollectionState::Ready);
    let cache = root.path().join("demo");
    assert_eq!(
        names_below(&cache),
        [
            "LICENSE",
            "LICENSE.attribution",
            "heart.svg",
            "heart.svg.attribution",
            "star.svg",
            "star.svg.attribution"
        ]
    );
    assert_eq!(unrecorded(&cache, None), Vec::<String>::new());
    assert_eq!(store.read("demo:heart").unwrap(), files_owned[0].1);
    let sidecar = fs::read_to_string(cache.join("star.svg.attribution")).unwrap();
    assert!(sidecar.contains(&format!("commit: {COMMIT}")));
    assert!(sidecar.contains("collection: demo"));
    assert!(sidecar.contains("license: MIT"));
    assert!(sidecar.contains(&format!("SHA-256 {}", sha(&files_owned[1].1))));
    assert_eq!(
        names_below(root.path()),
        ["demo"],
        "no staging directory is left"
    );

    // A Ready collection is searched by name and keyword; the slot frees after the job.
    for (query, expected) in [
        ("", 2),
        ("heart", 1),
        ("kw-star", 1),
        ("demo STAR", 1),
        ("zzz", 0),
    ] {
        let page = store.search(query, Some(Category::Icons), 10).unwrap();
        assert_eq!(page.total, expected, "{query}");
        assert!(page.unavailable.is_empty());
    }
    let capped = store.search("", Some(Category::Icons), 1).unwrap();
    assert_eq!((capped.items.len(), capped.total), (1, 2));
    assert_eq!(capped.items[0].id, "demo:heart");
    assert_eq!(capped.items[0].category, Category::Icons);
    assert_eq!((capped.items[0].width, capped.items[0].height), (256, 256));
    assert_eq!(download(&store, &mut runner), JobState::Completed);
    assert_eq!(
        server.calls.load(Ordering::SeqCst),
        3,
        "a verified cache is not downloaded again"
    );

    // Bytes changed at the same size, a lost record, a foreign file and a missing file.
    let heart = cache.join("heart.svg");
    let mut changed = fs::read(&heart).unwrap();
    changed[10] ^= 1;
    fs::write(&heart, changed).unwrap();
    assert!(matches!(
        store.read("demo:heart"),
        Err(EngineError::Sticker(StickerError::Integrity))
    ));
    assert!(matches!(
        store.read("demo:missing"),
        Err(EngineError::Sticker(StickerError::NotCached))
    ));
    fs::remove_file(cache.join("star.svg.attribution")).unwrap();
    assert_eq!(store.state("demo").unwrap(), CollectionState::Invalid);
    assert!(matches!(
        store.read("demo:star"),
        Err(EngineError::Sticker(StickerError::Integrity))
    ));
    let page = store.search("", Some(Category::Icons), 10).unwrap();
    assert_eq!(page.total, 0);
    assert_eq!(page.unavailable[0].state, CollectionState::Invalid);
    // The next explicit download replaces the damaged directory.
    assert_eq!(download(&store, &mut runner), JobState::Completed);
    assert_eq!(store.state("demo").unwrap(), CollectionState::Ready);
    assert_eq!(store.read("demo:heart").unwrap(), files_owned[0].1);
    fs::write(cache.join("extra.svg"), clean("M0 0")).unwrap();
    assert_eq!(store.state("demo").unwrap(), CollectionState::Invalid);
    runner.shutdown();
}

#[test]
fn failed_cancelled_and_offline_downloads_leave_no_cache_entry() {
    let heart_svg = clean("M1 1h8");
    let good: [(&str, &[u8]); 1] = [("heart", &heart_svg)];
    let collection = demo(&good);
    let root = tempfile::tempdir().unwrap();
    let mut runner = runner();

    // Wrong bytes at a pinned URL.
    let server = Server::serving(&collection, &good);
    server
        .bodies
        .lock()
        .unwrap()
        .insert(collection.raw_url("icons/heart.svg"), clean("M9 9h8"));
    assert_eq!(
        download(&store(root.path(), collection.clone(), server), &mut runner),
        JobState::Failed
    );
    assert!(names_below(root.path()).is_empty());

    // Bytes that match their pin but fail the admission check never reach the cache.
    let script_svg = format!("<svg xmlns=\"{SVG_NS}\"><script>alert(1)</script></svg>");
    let hostile: [(&str, &[u8]); 1] = [("heart", script_svg.as_bytes())];
    let poisoned = demo(&hostile);
    let server = Server::serving(&poisoned, &hostile);
    assert_eq!(
        download(&store(root.path(), poisoned, server), &mut runner),
        JobState::Failed
    );
    assert!(names_below(root.path()).is_empty());

    // A transport that cannot reach the server: a transient failure, the offline state.
    let offline = Arc::new(Server::default());
    let offline_store = store(root.path(), collection.clone(), offline.clone());
    let (id, _) = offline_store
        .submit_download(&mut runner, "demo", true)
        .unwrap();
    assert_eq!(finish(&mut runner, id), JobState::Failed);
    let snapshot = runner.snapshot(id).unwrap();
    assert!(snapshot.error.is_some_and(|error| error.transient()));
    assert_eq!(
        offline_store.state("demo").unwrap(),
        CollectionState::NotDownloaded
    );
    assert!(names_below(root.path()).is_empty());
    assert!(libraries::stickers::is_offline(
        &engine::RuntimeError::NetworkTransient.into()
    ));
    assert!(!libraries::stickers::is_offline(
        &StickerError::Integrity.into()
    ));

    // Cancelled before it starts: no request is made.
    let server = Server::serving(&collection, &good);
    let cancel_store = store(root.path(), collection, server.clone());
    let (id, cancel) = cancel_store
        .submit_download(&mut runner, "demo", true)
        .unwrap();
    cancel.cancel();
    assert_eq!(finish(&mut runner, id), JobState::Cancelled);
    assert_eq!(server.calls.load(Ordering::SeqCst), 0);
    assert!(names_below(root.path()).is_empty());
    runner.shutdown();
}

#[test]
fn search_spans_flags_and_reports_collections_that_are_not_downloaded() {
    let root = tempfile::tempdir().unwrap();
    let store = StickerStore::new(
        root.path().to_path_buf(),
        curated_collections(),
        Arc::new(Server::default()),
    )
    .unwrap();
    let all = store.search("", None, 5).unwrap();
    assert_eq!(all.items.len(), 5);
    assert_eq!(
        all.total, 271,
        "only the bundled flags are available offline"
    );
    assert!(
        all.items
            .iter()
            .all(|item| item.category == Category::Flags)
    );
    assert_eq!(
        all.unavailable
            .iter()
            .map(|u| (u.collection.as_str(), u.state))
            .collect::<Vec<_>>(),
        [
            ("noto", CollectionState::NotDownloaded),
            ("lucide", CollectionState::NotDownloaded),
            ("phosphor", CollectionState::NotDownloaded)
        ]
    );
    let flags = store.search("asia", Some(Category::Flags), 3).unwrap();
    assert!(flags.total > 3 && flags.items.len() == 3);
    assert!(flags.unavailable.is_empty());
    let shapes = store.search("", Some(Category::Shapes), 10).unwrap();
    assert_eq!((shapes.total, shapes.unavailable.len()), (0, 0));
    let emoji = store.search("smile", Some(Category::Emoji), 10).unwrap();
    assert_eq!(emoji.unavailable.len(), 1);
    let icons = store.search("heart", Some(Category::Icons), 10).unwrap();
    assert_eq!(icons.unavailable.len(), 2);
    // Ids resolve from the catalog whatever the cache holds; flags resolve elsewhere.
    let heart = store.resolve("lucide:heart").unwrap();
    assert_eq!(heart.name, "Heart");
    assert_eq!((heart.width, heart.height), (256, 256));
    assert!(store.resolve("noto:1f600").is_some());
    assert!(store.resolve("noto:zzzz").is_none());
    assert!(store.resolve("flags:VN").is_none());
    assert!(matches!(
        store.state("nope"),
        Err(EngineError::Sticker(StickerError::NotDownloadable))
    ));
}

/// One solid square, so a rendered pixel names the file it came from.
fn square(color: &str) -> Vec<u8> {
    format!(
        "<svg xmlns=\"{SVG_NS}\" viewBox=\"0 0 24 24\"><rect width=\"24\" height=\"24\" fill=\"{color}\"/></svg>"
    )
    .into_bytes()
}

fn centre(sticker: &str) -> Result<[u8; 4], render::StickerError> {
    let raster = render::rasterize_sticker(sticker, 32, 32)?;
    let at = (16 * 32 + 16) * 4;
    Ok(raster.pixels[at..at + 4].try_into().unwrap())
}

#[test]
fn control_and_render_serve_every_collection_and_refuse_until_downloaded() {
    use engine::control::StickerCategory;
    use libraries::stickers::{insert_sticker, resolve_sticker, search_catalog};
    let specs = [
        ("noto", Category::Emoji, "grin", "#ff0000"),
        ("lucide", Category::Icons, "heart", "#00ff00"),
        ("phosphor", Category::Icons, "star", "#0000ff"),
    ];
    let server = Arc::new(Server::default());
    let mut collections = Vec::new();
    for (id, category, value, color) in specs {
        let bytes = square(color);
        let mut collection = demo(&[(value, &bytes)]);
        collection.id = id.into();
        collection.category = category;
        {
            let mut bodies = server.bodies.lock().unwrap();
            bodies.insert(
                collection.raw_url(&collection.license_path),
                LICENSE.to_vec(),
            );
            bodies.insert(collection.raw_url(&format!("icons/{value}.svg")), bytes);
        }
        collections.push(collection);
    }
    let root = tempfile::tempdir().unwrap();
    let store =
        Arc::new(StickerStore::new(root.path().to_path_buf(), collections, server).unwrap());
    render::lend_sticker_source(store.clone());
    let handle: &StickerStore = &store;
    let total = |category| {
        search_catalog(Some(handle), "", category, Some(1000))
            .unwrap()
            .total
    };

    // Nothing downloaded: search lists flags only, every id still resolves with the fetched size
    // and inserts, and the renderer refuses typed.
    assert_eq!(total(StickerCategory::All), 271);
    assert_eq!(total(StickerCategory::Emoji), 0);
    assert_eq!(total(StickerCategory::Icons), 0);
    let grin = resolve_sticker(Some(handle), " noto:grin ").unwrap();
    assert_eq!(
        (grin.id.as_str(), grin.width, grin.height),
        ("noto:grin", 256, 256)
    );
    assert_eq!(
        resolve_sticker(Some(handle), "flags:vn").unwrap().id,
        "flags:VN"
    );
    assert!(resolve_sticker(Some(handle), "noto:nope").is_none());
    assert!(resolve_sticker(None, "noto:grin").is_none());
    assert_eq!(
        centre("noto:grin"),
        Err(render::StickerError::NotDownloaded {
            sticker: "noto:grin".into()
        })
    );
    let mut session = engine::Session::new(
        project::Project::empty(project::ProjectId(1), "stickers".into()).unwrap(),
        10,
    )
    .unwrap();
    let outcome = insert_sticker(
        &mut session,
        Some(handle),
        "noto:grin",
        engine::Placement::Auto,
        time::Tick::ZERO,
        time::Tick(1000),
    )
    .unwrap();
    assert!(outcome.changed);
    let stored: Vec<_> = session
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| track.items.iter())
        .filter_map(|item| match &item.content {
            project::ItemParams::Sticker(params) => Some(params.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(stored, [project::StickerParams::new("noto:grin", 256, 256)]);
    assert!(
        insert_sticker(
            &mut session,
            Some(handle),
            "noto:nope",
            engine::Placement::Auto,
            time::Tick::ZERO,
            time::Tick(1000),
        )
        .is_err()
    );

    // Downloaded: each collection lists under its category and draws its own pixels.
    let mut jobs = runner();
    for (id, ..) in specs {
        let (job, _) = store.submit_download(&mut jobs, id, true).unwrap();
        assert_eq!(finish(&mut jobs, job), JobState::Completed);
    }
    assert_eq!(total(StickerCategory::Emoji), 1);
    assert_eq!(total(StickerCategory::Icons), 2);
    assert_eq!(total(StickerCategory::All), 274);
    let ids = |category, query| {
        search_catalog(Some(handle), query, category, None)
            .unwrap()
            .results
            .into_iter()
            .map(|entry| entry.id)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        ids(StickerCategory::Icons, ""),
        ["lucide:heart", "phosphor:star"]
    );
    assert_eq!(ids(StickerCategory::All, "kw-grin"), ["noto:grin"]);
    assert_eq!(centre("noto:grin"), Ok([255, 0, 0, 255]));
    assert_eq!(centre("lucide:heart"), Ok([0, 255, 0, 255]));

    // A file changed after the download is never drawn.
    fs::write(root.path().join("phosphor/star.svg"), square("#ffffff")).unwrap();
    assert!(matches!(
        centre("phosphor:star"),
        Err(render::StickerError::Invalid { .. })
    ));
}

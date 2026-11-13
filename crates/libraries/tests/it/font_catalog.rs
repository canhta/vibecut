#![allow(
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "font catalog integration fixtures"
)]
//! Font catalog behaviour through the store, the job runner and `text` layout.

use engine::FontError;
use engine::control::{FontEntry, FontLoadState, FontOrigin, FontState};
use engine::{
    ByteBudget,
    control::Error as EngineError,
    jobs::{Clock, JobId, JobRunner, JobState, RunnerConfig, SystemClock},
};
use libraries::fonts::{
    FontStore, GoogleFamily, GoogleFile, ScannedFace, curated_google_families, scan_directory,
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
use transport::{DownloadTransport, HttpStreamResponse};

pub const FIXTURE: &[u8] = include_bytes!("../fixtures/fonts/VibeCutFixtureSans-Regular.ttf");
const SERIF: &[u8] = include_bytes!("../../../../assets/fonts/NotoSerif-Regular.ttf");
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
    interrupted: Mutex<BTreeMap<String, bool>>,
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
    interrupted: bool,
    data: io::Cursor<Vec<u8>>,
    gate: Option<Arc<Gate>>,
    first: bool,
}
impl Read for Body {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if self.interrupted && self.data.position() > 0 {
            return Err(io::Error::new(
                io::ErrorKind::ConnectionReset,
                "interrupted",
            ));
        }
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
        let status = self.status.lock().unwrap().get(url).copied().unwrap_or(200);
        Ok(HttpStreamResponse {
            status,
            headers: Vec::new(),
            body: Box::new(Body {
                interrupted: self
                    .interrupted
                    .lock()
                    .unwrap()
                    .get(url)
                    .copied()
                    .unwrap_or(false),
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
    Arc::new(
        FontStore::new(
            root,
            vec![catalog],
            Vec::new,
            server,
            Arc::new(libraries::fonts::LocalFontFile),
        )
        .unwrap(),
    )
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
        Arc::new(libraries::fonts::LocalFontFile),
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

// Test-only WOFF2 container with null table transforms, encoded from our admitted
// fixture bytes. Brotli is an independent encoder; production only decodes.
fn woff2(bytes: &[u8]) -> Vec<u8> {
    use std::io::Write;
    let tables = u16::from_be_bytes(bytes[4..6].try_into().unwrap()) as usize;
    let mut directory = Vec::new();
    let mut body = Vec::new();
    for table in bytes[12..12 + tables * 16].as_chunks::<16>().0 {
        let tag = &table[..4];
        directory.push(if tag == b"glyf" || tag == b"loca" {
            255
        } else {
            63
        });
        directory.extend_from_slice(tag);
        let offset = u32::from_be_bytes(table[8..12].try_into().unwrap()) as usize;
        let length = u32::from_be_bytes(table[12..16].try_into().unwrap()) as usize;
        let mut encoded = vec![(length & 127) as u8];
        let mut n = length >> 7;
        while n != 0 {
            encoded.push((n as u8 & 127) | 128);
            n >>= 7;
        }
        directory.extend(encoded.into_iter().rev());
        body.extend_from_slice(&bytes[offset..offset + length]);
    }
    let mut compressed = Vec::new();
    {
        let mut encoder = brotli::CompressorWriter::new(&mut compressed, 4096, 5, 20);
        encoder.write_all(&body).unwrap();
    }
    let mut out = b"wOF2".to_vec();
    out.extend_from_slice(&bytes[..4]);
    out.extend_from_slice(&((48 + directory.len() + compressed.len()) as u32).to_be_bytes());
    out.extend_from_slice(&(tables as u16).to_be_bytes());
    out.extend_from_slice(&0u16.to_be_bytes());
    out.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    out.extend_from_slice(&(compressed.len() as u32).to_be_bytes());
    out.resize(48, 0);
    out.extend(directory);
    out.extend(compressed);
    out.resize((out.len() + 3) & !3, 0);
    let size = out.len() as u32;
    out[8..12].copy_from_slice(&size.to_be_bytes());
    out
}

fn renamed(bytes: &[u8], from: &str, to: &str) -> Vec<u8> {
    let from: Vec<u8> = from.encode_utf16().flat_map(u16::to_be_bytes).collect();
    let to: Vec<u8> = to.encode_utf16().flat_map(u16::to_be_bytes).collect();
    assert_eq!(from.len(), to.len());
    let mut out = bytes.to_vec();
    let mut replaced = 0;
    for offset in 0..=out.len() - from.len() {
        if out[offset..offset + from.len()] == from {
            out[offset..offset + from.len()].copy_from_slice(&to);
            replaced += 1;
        }
    }
    assert!(replaced != 0);
    out
}

#[test]
fn custom_import_copies_valid_formats_and_deduplicates_without_source_dependency() {
    let cjk = include_bytes!("../../../../assets/fonts/VibeCutSansCJK-Regular.otf");
    let otf = renamed(cjk, "VibeCut Sans CJK", "Custom  Sans CJK");
    let compressed = woff2(&renamed(
        FIXTURE,
        "VibeCut Fixture Sans",
        "WibeCut Fixture Sans",
    ));
    for (format, bytes, family) in [
        ("ttf", FIXTURE.to_vec(), "VibeCut Fixture Sans"),
        ("otf", otf, "Custom  Sans CJK"),
        ("woff2", compressed, "WibeCut Fixture Sans"),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join(format!("original.{format}"));
        fs::write(&input, &bytes).unwrap();
        let root = dir.path().join("fonts");
        let store = store(&root, vec![], vec![], offline());
        let mut runner = runner();
        assert!(matches!(
            store.submit_import(&mut runner, input.clone(), false),
            Err(EngineError::Control(
                engine::control::ControlError::Permission
            ))
        ));
        let (job, _) = store
            .submit_import(&mut runner, input.clone(), true)
            .unwrap();
        assert_eq!(
            finish(&mut runner, job),
            JobState::Completed,
            "{format}: {:?}",
            runner.snapshot(job).unwrap().error
        );
        let before = entries_below(&root);
        let (job, _) = store
            .submit_import(&mut runner, input.clone(), true)
            .unwrap();
        assert_eq!(finish(&mut runner, job), JobState::Completed);
        assert_eq!(entries_below(&root), before);
        fs::rename(&input, dir.path().join("moved-original")).unwrap();
        let reopened = super::font_catalog::store(&root, vec![], vec![], offline());
        let entries = reopened.list(Some(family)).unwrap();
        let entry = entries.iter().find(|entry| entry.family == family).unwrap();
        assert_eq!(entry.origin, FontOrigin::Custom);
        assert!(entry.source.as_ref().unwrap().starts_with("local:sha256:"));
        assert_eq!(reopened.load(family).unwrap().state, FontLoadState::Loaded);
        let resolved = text::resolve(family, "Regular");
        assert!(!resolved.missing);
        assert_eq!(text::face(resolved.face).family, family);
        if format != "otf" {
            // Independent fixture table values: head.unitsPerEm=2048,
            // cmap('H')=3, hmtx[3].advanceWidth=1522, empty kerning tables.
            let layout = text::layout(
                "HH",
                &text::LayoutOptions {
                    family,
                    size: 64.0,
                    ..Default::default()
                },
            )
            .unwrap();
            assert_eq!(
                layout
                    .glyphs
                    .iter()
                    .map(|glyph| glyph.id)
                    .collect::<Vec<_>>(),
                [3, 3]
            );
            assert!((layout.glyphs[1].x - 47.5625).abs() < 0.001);
            assert!((layout.lines[0].width - 95.125).abs() < 0.001);
            assert_eq!(text::fallback_for('H', resolved.face, &[]), resolved.face);
            assert_eq!(
                text::face(text::fallback_for('ữ', resolved.face, &[])).family,
                "Inter"
            );
        }
        // Once damaged on disk, a new store cannot advertise or load the copy.
        let published = fs::read_dir(&root)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| path.is_dir())
            .unwrap();
        fs::write(published.join("font.sfnt"), b"tampered").unwrap();
        assert!(
            !reopened
                .list(Some(family))
                .unwrap()
                .iter()
                .any(|entry| entry.origin == FontOrigin::Custom)
        );
        assert_eq!(
            reopened.load(family).unwrap().state,
            FontLoadState::CacheInvalid
        );
        runner.shutdown();
    }
}

#[test]
fn custom_import_refuses_hostile_data_conflicts_and_cancel_without_publication() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("fonts");
    let store = store(&root, vec![], vec![], offline());
    let mut runner = runner();
    for bytes in [
        Vec::new(),
        b"not a font".to_vec(),
        b"wOFFunsupported".to_vec(),
        b"wOF2".to_vec(),
        FIXTURE[..FIXTURE.len() / 2].to_vec(),
        {
            let mut bytes = FIXTURE.to_vec();
            bytes[20..24].copy_from_slice(&u32::MAX.to_be_bytes());
            bytes
        },
        {
            let mut bytes = woff2(FIXTURE);
            bytes[16..20].copy_from_slice(&u32::MAX.to_be_bytes());
            bytes
        },
        {
            let mut bytes = woff2(FIXTURE);
            // Lie about the header's size, then demand an oversized first table.
            bytes[16..20].copy_from_slice(&1024u32.to_be_bytes());
            bytes[53..58].copy_from_slice(&[0x8f, 0xff, 0xff, 0xff, 0x7f]);
            bytes
        },
    ] {
        let input = dir.path().join("bad.ttf");
        fs::write(&input, bytes).unwrap();
        let (job, _) = store.submit_import(&mut runner, input, true).unwrap();
        assert_eq!(finish(&mut runner, job), JobState::Failed);
        assert!(entries_below(&root).is_empty());
    }
    for (label, size) in [
        ("missing", None),
        ("oversize", Some(text::MAX_FONT_BYTES as u64 + 1)),
    ] {
        let input = dir.path().join(label);
        if let Some(size) = size {
            fs::File::create(&input).unwrap().set_len(size).unwrap();
        }
        let (job, _) = store.submit_import(&mut runner, input, true).unwrap();
        assert_eq!(finish(&mut runner, job), JobState::Failed, "{label}");
        assert!(entries_below(&root).is_empty());
    }
    let input = dir.path().join("valid.ttf");
    fs::write(&input, FIXTURE).unwrap();
    let (job, token) = store
        .submit_import(&mut runner, input.clone(), true)
        .unwrap();
    token.cancel();
    assert_eq!(finish(&mut runner, job), JobState::Cancelled);
    assert!(entries_below(&root).is_empty());
    let (job, _) = store
        .submit_import(&mut runner, input.clone(), true)
        .unwrap();
    assert_eq!(finish(&mut runner, job), JobState::Completed);
    let before = entries_below(&root);
    let mut changed = FIXTURE.to_vec();
    changed.push(0);
    fs::write(&input, changed).unwrap();
    let (job, _) = store.submit_import(&mut runner, input, true).unwrap();
    assert_eq!(finish(&mut runner, job), JobState::Failed);
    assert_eq!(entries_below(&root), before);
    let error = runner.snapshot(job).unwrap().error.unwrap();
    assert!(matches!(
        (error.as_ref() as &dyn std::error::Error).downcast_ref::<EngineError>(),
        Some(EngineError::Font(FontError::Conflict))
    ));
    runner.shutdown();
}

#[test]
fn custom_import_cancels_during_source_read_and_preserves_foreign_store_entries() {
    struct Gated {
        started: std::sync::mpsc::SyncSender<()>,
        release: Mutex<std::sync::mpsc::Receiver<()>>,
    }
    impl libraries::fonts::FontFile for Gated {
        fn read(
            &self,
            _: &Path,
            _: u64,
            ctx: &engine::jobs::JobContext,
        ) -> engine::control::Result<Vec<u8>> {
            self.started.send(()).unwrap();
            self.release
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(10))
                .unwrap();
            ctx.check_cancelled()?;
            Ok(FIXTURE.to_vec())
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let foreign = dir.path().join("unrelated.bin");
    fs::write(&foreign, b"foreign bytes").unwrap();
    let (started, started_rx) = std::sync::mpsc::sync_channel(1);
    let (release, release_rx) = std::sync::mpsc::channel();
    let store = FontStore::new(
        dir.path().to_owned(),
        vec![],
        Vec::new,
        offline(),
        Arc::new(Gated {
            started,
            release: Mutex::new(release_rx),
        }),
    )
    .unwrap();
    let mut runner = runner();
    let (job, token) = store
        .submit_import(&mut runner, PathBuf::from("injected.ttf"), true)
        .unwrap();
    wait_started(&mut runner, &started_rx);
    assert!(matches!(
        store.submit_import(&mut runner, PathBuf::from("second.ttf"), true),
        Err(EngineError::Font(FontError::ImportActive))
    ));
    token.cancel();
    release.send(()).unwrap();
    assert_eq!(finish(&mut runner, job), JobState::Cancelled);
    assert_eq!(entries_below(dir.path()), ["unrelated.bin"]);
    assert_eq!(fs::read(foreign).unwrap(), b"foreign bytes");
    runner.shutdown();
}

#[test]
fn custom_import_accented_family_conflicts_and_resolution_share_one_identity() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("fonts");
    let input = directory.path().join("accented.ttf");
    let store = store(&root, vec![], vec![], offline());
    let mut runner = runner();
    fs::write(
        &input,
        renamed(FIXTURE, "VibeCut Fixture Sans", "ÉibeCut Fixture Sans"),
    )
    .unwrap();
    let (job, _) = store
        .submit_import(&mut runner, input.clone(), true)
        .unwrap();
    assert_eq!(finish(&mut runner, job), JobState::Completed);
    let before = entries_below(&root);
    fs::write(
        &input,
        renamed(FIXTURE, "VibeCut Fixture Sans", "éibeCut Fixture Sans"),
    )
    .unwrap();
    let (job, _) = store.submit_import(&mut runner, input, true).unwrap();
    assert_eq!(finish(&mut runner, job), JobState::Failed);
    let error = runner.snapshot(job).unwrap().error.unwrap();
    assert!(matches!(
        (error.as_ref() as &dyn std::error::Error).downcast_ref::<EngineError>(),
        Some(EngineError::Font(FontError::Conflict))
    ));
    assert_eq!(entries_below(&root), before);
    assert_eq!(
        store.load("éIBECUT FIXTURE SANS").unwrap().state,
        FontLoadState::Loaded
    );
    assert!(!text::resolve("éIBECUT FIXTURE SANS", "Regular").missing);
    runner.shutdown();
}

#[test]
fn custom_store_metadata_quotas_bound_reopen_before_face_reads() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("seed.ttf");
    fs::write(&input, FIXTURE).unwrap();
    let seed = dir.path().join("seed");
    let seed_store = store(&seed, vec![], vec![], offline());
    let mut runner = runner();
    let (job, _) = seed_store.submit_import(&mut runner, input, true).unwrap();
    assert_eq!(finish(&mut runner, job), JobState::Completed);
    let template: serde_json::Value = serde_json::from_slice(
        &fs::read(
            seed.join(format!("custom-{}", sha(FIXTURE)))
                .join("import.json"),
        )
        .unwrap(),
    )
    .unwrap();
    for (label, count, size, same_family, exceeds) in [
        ("bytes-at", 8, text::MAX_FONT_BYTES as u64, false, false),
        ("bytes-over", 9, text::MAX_FONT_BYTES as u64, false, true),
        ("faces-at", 64, 1, true, false),
        ("faces-over", 65, 1, true, true),
    ] {
        let root = dir.path().join(label);
        for index in 0..count {
            let hash = format!("{index:064x}");
            let entry = root.join(format!("custom-{hash}"));
            fs::create_dir_all(&entry).unwrap();
            let mut metadata = template.clone();
            metadata["sha256"] = hash.into();
            metadata["size"] = size.into();
            metadata["family"] = if same_family {
                "Quota same".to_owned()
            } else {
                format!("Quota {index}")
            }
            .into();
            fs::write(
                entry.join("import.json"),
                serde_json::to_vec(&metadata).unwrap(),
            )
            .unwrap();
        }
        // These generated records deliberately have no face files: quota admission
        // must happen before opening any advertised data or allocating its length.
        let result = store(&root, vec![], vec![], offline()).list(Some("Quota"));
        if exceeds {
            assert!(
                matches!(
                    result,
                    Err(EngineError::Control(engine::control::ControlError::Limit))
                ),
                "{label}"
            );
        } else {
            assert!(
                result.unwrap().is_empty(),
                "{label}: missing faces are not advertised"
            );
        }
    }
    runner.shutdown();
}

#[test]
fn custom_import_preserves_weight_order_for_multiple_faces_of_one_family() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("fonts");
    let store = store(&root, vec![], vec![], offline());
    let mut runner = runner();
    // Import bold first; catalog/resolve order must follow font weight, not input
    // order or alphabetic style names. Both are admitted OFL Inter assets.
    for (name, bytes) in [
        (
            "bold",
            include_bytes!("../../../../assets/fonts/Inter-Bold.ttf").as_slice(),
        ),
        (
            "regular",
            include_bytes!("../../../../assets/fonts/Inter-Regular.ttf").as_slice(),
        ),
    ] {
        let input = dir.path().join(format!("{name}.ttf"));
        fs::write(&input, renamed(bytes, "Inter", "Scope")).unwrap();
        let (job, _) = store.submit_import(&mut runner, input, true).unwrap();
        assert_eq!(finish(&mut runner, job), JobState::Completed);
    }
    let listed = store.list(Some("Scope")).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].styles, ["Regular", "Bold"]);
    assert_eq!(store.load("Scope").unwrap().styles, ["Regular", "Bold"]);
    for (style, weight) in [("Regular", 400), ("Bold", 700)] {
        let resolved = text::resolve("Scope", style);
        assert!(!resolved.missing);
        assert_eq!(text::face(resolved.face).weight, weight);
    }
    runner.shutdown();
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
    // The text registry is process-wide and other tests load the fixture family, so this
    // test owns a family of its own: only its loads can make it available.
    let fixture = renamed(FIXTURE, "VibeCut Fixture Sans", "VibeCut LazySys Sans");
    let file = dir.path().join("VibeCutLazySysSans-Regular.ttf");
    fs::write(&file, &fixture).unwrap();
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
        Arc::new(libraries::fonts::LocalFontFile),
    )
    .unwrap();
    assert_eq!(scans.load(Ordering::SeqCst), 0, "no scan before first use");

    let listed = store.list(Some("lazysys")).unwrap();
    assert_eq!(names(&listed), ["VibeCut LazySys Sans"]);
    assert_eq!(scans.load(Ordering::SeqCst), 1);

    // Listing read no font bytes: damage the file and loading it is what fails.
    fs::write(&file, b"damaged").unwrap();
    assert!(matches!(
        store.load("VibeCut LazySys Sans"),
        Err(EngineError::Font(FontError::Face(
            text::FontError::Unreadable
        )))
    ));
    assert!(!text::is_available("VibeCut LazySys Sans"));
    fs::write(&file, &fixture).unwrap();

    let loaded = store.load("vibecut lazysys sans").unwrap();
    assert_eq!(loaded.state, FontLoadState::Loaded);
    assert_eq!(loaded.origin, Some(FontOrigin::System));
    assert_eq!(loaded.effective_family, "VibeCut LazySys Sans");
    assert_eq!(loaded.styles, ["Regular"]);
    assert_eq!(scans.load(Ordering::SeqCst), 1, "one scan per store");
    let (missing, faces) = laid_out_faces("VibeCut LazySys Sans");
    assert!(!missing);
    let registered = text::resolve("VibeCut LazySys Sans", "Regular").face;
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
        assert!(matches!(
            store.load(hostile),
            Err(EngineError::Edit(engine::EditError::Invalid(_)))
        ));
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
        Err(EngineError::Providers(
            engine::providers::Error::ConsentRequired
        ))
    ));
    for refused in ["Inter", "Not In Catalog", ""] {
        assert!(matches!(
            store.submit_download(&mut runner, refused, true),
            Err(EngineError::Font(FontError::NotDownloadable))
        ));
    }
    assert_eq!(server.calls.load(Ordering::SeqCst), 0);
    assert!(entries_below(root.path()).is_empty());

    let key = engine::cache::CacheKey::new(engine::cache::KeyInputs {
        kind: engine::cache::ArtifactKind::FontDownload,
        source: [0; 32],
        provider: "fixture",
        model: "queue",
        version: "1",
        settings: &[],
        transcript: [0; 32],
        cues: [0; 32],
        clip_range: [0; 32],
        translation: [0; 32],
    })
    .unwrap();
    let mut fillers = Vec::new();
    for _ in 0..16 {
        let (_, cancel) = runner
            .submit(
                engine::jobs::JobSpec {
                    kind: engine::jobs::JobKind::DownloadFont,
                    class: engine::jobs::ResourceClass::Network,
                    input: key,
                    bytes: engine::jobs::JobBytes::default(),
                },
                |_| -> Result<Vec<u8>, engine::control::Error> { panic!("cancelled filler ran") },
            )
            .unwrap();
        fillers.push(cancel);
    }
    assert!(matches!(
        store.submit_download(&mut runner, "Gate Sample", true),
        Err(EngineError::Runtime(engine::RuntimeError::QueueFull))
    ));
    assert_eq!(runner.counts().0, 16);
    assert_eq!(server.calls.load(Ordering::SeqCst), 0);
    assert!(entries_below(root.path()).is_empty());
    for token in fillers {
        token.cancel();
    }
    runner.tick().unwrap();
    assert_eq!(runner.counts().0, 0);

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
        Err(EngineError::Font(FontError::DownloadActive))
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
    let cases: [(&str, Rig); 7] = [
        (
            "interrupted font",
            Box::new(|g, s| {
                s.interrupted
                    .lock()
                    .unwrap()
                    .insert(g.files[0].url.clone(), true);
            }),
        ),
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
        let store = store(root.path(), vec![catalog], Vec::new(), server.clone());
        let mut runner = runner();
        let (id, _) = store
            .submit_download(&mut runner, "Failure Sample", true)
            .unwrap();
        assert_eq!(finish(&mut runner, id), JobState::Failed, "{label}");
        assert_eq!(runner.snapshot(id).unwrap().attempts, 1);
        assert!(runner.take_output(id).is_none());
        let snapshot = runner.snapshot(id).unwrap();
        let raw = snapshot.error.as_deref().unwrap();
        let network = matches!(label, "unreachable licence" | "interrupted font");
        assert_eq!(raw.transient(), network, "{label}");
        if network {
            assert!(matches!(
                (raw as &(dyn std::error::Error + 'static)).downcast_ref::<EngineError>(),
                Some(EngineError::Runtime(engine::RuntimeError::NetworkTransient))
            ));
            assert_eq!(raw.code(), engine::control::ErrorCode::Job);
            assert!(raw.source().is_none());
        }
        assert_eq!(
            server.calls.load(Ordering::SeqCst),
            if matches!(
                label,
                "unreachable licence" | "licence fails after the font was written"
            ) {
                2
            } else {
                1
            }
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
        assert!(
            FontStore::new(
                root.path().to_path_buf(),
                bad,
                Vec::new,
                offline(),
                Arc::new(libraries::fonts::LocalFontFile)
            )
            .is_err()
        );
    }
}

fn native_probe(mode: &str) -> std::process::Command {
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", "font_catalog::native_cache_child", "--nocapture"])
        .env("VIBECUT_TEST_FONT_CACHE", mode)
        .env_remove("HOME")
        .env_remove("LOCALAPPDATA")
        .env_remove("XDG_CACHE_HOME");
    command
}

fn native_base(command: &mut std::process::Command, root: &Path) {
    #[cfg(target_os = "windows")]
    command
        .env("LOCALAPPDATA", root)
        .env("VIBECUT_TEST_FONT_BASE", root);
    #[cfg(target_os = "macos")]
    command
        .env("HOME", root)
        .env("VIBECUT_TEST_FONT_BASE", root.join("Library/Caches"));
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    command
        .env("XDG_CACHE_HOME", root)
        .env("VIBECUT_TEST_FONT_BASE", root);
}

fn run_native(command: &mut std::process::Command) {
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn native_discovery_is_lazy_and_uses_the_exact_font_cache() {
    let dir = tempfile::tempdir().unwrap();
    let mut command = native_probe("path");
    native_base(&mut command, &dir.path().join("Phông chữ cache"));
    run_native(&mut command);
}

#[test]
fn native_refuses_unavailable_cache_as_runtime_unsupported() {
    let dir = tempfile::tempdir().unwrap();
    for selected in [None, Some(""), Some("relative fonts")] {
        let mut command = native_probe("unavailable");
        command.current_dir(dir.path());
        if let Some(selected) = selected {
            native_base(&mut command, Path::new(selected));
        }
        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        if selected.is_some() {
            command.env("HOME", dir.path());
        }
        run_native(&mut command);
    }
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[test]
fn native_cache_child() {
    let Ok(mode) = std::env::var("VIBECUT_TEST_FONT_CACHE") else {
        return;
    };
    if mode == "unavailable" {
        // CacheRoot::discover composes the refusal through the runtime owner.
        assert!(matches!(
            FontStore::native(),
            Err(EngineError::Runtime(engine::RuntimeError::Edit(
                engine::EditError::Unsupported("OS cache location unavailable")
            )))
        ));
        return;
    }
    let base = PathBuf::from(std::env::var_os("VIBECUT_TEST_FONT_BASE").unwrap());
    let store = FontStore::native().unwrap();
    assert!(!base.exists(), "native composition performs no cache IO");
    // A synthetic inaccessible entry refuses before system scanning, font reads or
    // transport. It distinguishes the real native root from models/fonts.
    let fonts = base.join("VibeCut/fonts");
    fs::create_dir_all(fonts.parent().unwrap()).unwrap();
    #[cfg(windows)]
    let hold = {
        use std::os::windows::fs::OpenOptionsExt;
        let family = curated_google_families()
            .into_iter()
            .find(|g| g.family == "Lobster")
            .unwrap();
        let entry = fonts.join(&family.slug);
        fs::create_dir_all(&entry).unwrap();
        let file = family.files.first().unwrap();
        let sentry = entry.join(&file.name);
        fs::File::create(&sentry)
            .unwrap()
            .set_len(file.size)
            .unwrap();
        fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(sentry)
            .unwrap()
    };
    #[cfg(unix)]
    std::os::unix::fs::symlink(&fonts, &fonts).unwrap();
    assert!(matches!(store.load("Lobster"), Err(EngineError::Io(_))));
    #[cfg(windows)]
    {
        drop(hold);
        assert_eq!(fs::read_dir(fonts).unwrap().count(), 1);
    }
    assert!(!base.join("VibeCut/models").exists());
}

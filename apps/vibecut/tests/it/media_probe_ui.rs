//! Media library status, thumbnail cache and placeholder behaviour.
#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::expect_used
)]
use engine::{ByteBudget, Session};
use media::{
    AssetData, AssetId, Decodability, ImportedSource, MediaError, ProbeCapabilities, SourceAsset,
    StreamCapability, StreamKind, Thumbnail, UnsupportedReason,
};
use project::{Project, ProjectId};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use time::Tick;
use vibecut::{
    Editor, egui,
    import::{ImportProbe, ThumbnailSource},
    playback::{Files, Playback},
    preferences::Preferences,
};

const SECOND: i64 = time::TICKS_PER_SECOND;

fn asset(id: u64, hash: u8, video: bool, path: impl Into<PathBuf>) -> SourceAsset {
    let kind = if video {
        StreamKind::Video
    } else {
        StreamKind::Audio
    };
    let mut stream = testkit::source::stream(kind).duration(Tick(4 * SECOND));
    if video {
        stream = stream.size(320, 180);
    }
    testkit::source::asset(id, path.into())
        .format("mov,mp4")
        .duration(Tick(4 * SECOND))
        .hash([hash; 32])
        .stream(stream.build())
        .build()
}

fn capabilities(asset: &SourceAsset, decodable: bool) -> ProbeCapabilities {
    ProbeCapabilities {
        streams: asset
            .probe()
            .streams
            .iter()
            .map(|stream| StreamCapability {
                index: stream.index,
                kind: stream.kind,
                codec: stream.codec.clone(),
                decodability: if decodable {
                    Decodability::Decodable
                } else {
                    Decodability::Unsupported(UnsupportedReason::DecoderUnavailable)
                },
            })
            .collect(),
    }
}

struct FixedProbe {
    id: u64,
    hash: u8,
    decodable: bool,
}
impl ImportProbe for FixedProbe {
    fn import(
        &self,
        _id: AssetId,
        path: PathBuf,
        _cancelled: Arc<AtomicBool>,
    ) -> media::Result<ImportedSource> {
        let asset = asset(self.id, self.hash, true, path);
        Ok(ImportedSource {
            capabilities: capabilities(&asset, self.decodable),
            asset,
        })
    }
}

struct Thumbs {
    calls: AtomicUsize,
    release: Option<Arc<AtomicBool>>,
    started: AtomicBool,
    fail: bool,
}
impl Thumbs {
    fn ready() -> Self {
        Self {
            calls: AtomicUsize::new(0),
            release: None,
            started: AtomicBool::new(false),
            fail: false,
        }
    }
    fn gated(release: Arc<AtomicBool>) -> Self {
        Self {
            release: Some(release),
            ..Self::ready()
        }
    }
    fn failing() -> Self {
        Self {
            fail: true,
            ..Self::ready()
        }
    }
    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
    fn started(&self) -> bool {
        self.started.load(Ordering::Acquire)
    }
}
impl ThumbnailSource for Thumbs {
    fn thumbnail(
        &self,
        _asset: &SourceAsset,
        _at: Tick,
        _max_edge: u32,
        cancelled: Arc<AtomicBool>,
    ) -> media::Result<Thumbnail> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.started.store(true, Ordering::Release);
        if let Some(release) = &self.release {
            while !release.load(Ordering::Acquire) {
                if cancelled.load(Ordering::Acquire) {
                    return Err(MediaError::Cancelled);
                }
                std::thread::sleep(Duration::from_millis(1));
            }
        }
        if self.fail {
            return Err(MediaError::Unsupported("test thumbnail failure"));
        }
        Ok(Thumbnail {
            width: 2,
            height: 1,
            frame: Tick::ZERO,
            pixels: vec![255, 0, 0, 255, 0, 255, 0, 255],
        })
    }
}

fn new_editor() -> Editor {
    let session = Session::new(Project::empty(ProjectId(41), "Media".into()).unwrap(), 50).unwrap();
    let mut playback = Playback::new(
        &session,
        ByteBudget::new(512 << 20).unwrap(),
        Arc::new(Files),
    )
    .unwrap();
    playback.set_device_available(true);
    Editor::new(session, playback, Preferences::default())
}

fn tick_until(editor: &mut Editor, mut done: impl FnMut(&Editor) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        editor.tick();
        if done(editor) {
            return;
        }
        assert!(Instant::now() < deadline, "condition not reached");
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn finish(editor: &mut Editor) {
    tick_until(editor, |editor| !editor.importing());
}

fn state(editor: &Editor, index: usize) -> String {
    editor.inspect().unwrap().assets[index].state.to_owned()
}

#[test]
fn probing_badge_becomes_ready_with_a_thumbnail_and_iso_audio_finishes_immediately() {
    let release = Arc::new(AtomicBool::new(false));
    let thumbs = Arc::new(Thumbs::gated(release.clone()));
    let mut editor = new_editor();
    editor
        .set_import_probe(Arc::new(FixedProbe {
            id: 1,
            hash: 3,
            decodable: true,
        }))
        .unwrap();
    editor.set_thumbnailer(thumbs.clone()).unwrap();
    editor
        .start_import(vec![std::env::temp_dir().join("vibecut-test/probing.mp4")])
        .unwrap();
    tick_until(&mut editor, |editor| {
        !editor.inspect().unwrap().assets.is_empty()
    });
    tick_until(&mut editor, |_| thumbs.started());
    assert_eq!(state(&editor, 0), "probing");
    assert!(!editor.inspect().unwrap().assets[0].thumbnail);
    release.store(true, Ordering::Release);
    finish(&mut editor);
    let inspection = editor.inspect().unwrap();
    assert_eq!(inspection.assets[0].state, "ready");
    assert!(inspection.assets[0].thumbnail);
    assert_eq!(thumbs.calls(), 1);

    // Image/video thumbnails are cached; a second editor hits the same cache.
    let root = tempfile::tempdir().unwrap();
    let first = Arc::new(Thumbs::ready());
    let mut editor = new_editor();
    editor.set_thumbnail_root(root.path().to_path_buf());
    editor
        .set_import_probe(Arc::new(FixedProbe {
            id: 1,
            hash: 9,
            decodable: true,
        }))
        .unwrap();
    editor.set_thumbnailer(first.clone()).unwrap();
    editor
        .start_import(vec![std::env::temp_dir().join("vibecut-test/reused.mp4")])
        .unwrap();
    finish(&mut editor);
    assert_eq!(first.calls(), 1);
    drop(editor);
    let second = Arc::new(Thumbs::ready());
    let mut editor = new_editor();
    editor.set_thumbnail_root(root.path().to_path_buf());
    editor
        .set_import_probe(Arc::new(FixedProbe {
            id: 1,
            hash: 9,
            decodable: true,
        }))
        .unwrap();
    editor.set_thumbnailer(second.clone()).unwrap();
    editor
        .start_import(vec![std::env::temp_dir().join("vibecut-test/reused.mp4")])
        .unwrap();
    finish(&mut editor);
    assert_eq!(second.calls(), 0, "cached frame must not decode again");
    assert!(editor.inspect().unwrap().assets[0].thumbnail);
    let bytes = serde_json::to_vec(editor.session.project().as_ref()).unwrap();
    drop(editor);
    let session = Session::new(serde_json::from_slice(&bytes).unwrap(), 50).unwrap();
    let playback = Playback::new(
        &session,
        ByteBudget::new(512 << 20).unwrap(),
        Arc::new(Files),
    )
    .unwrap();
    let mut reopened = Editor::new(session, playback, Preferences::default());
    reopened.set_thumbnail_root(root.path().to_path_buf());
    reopened
        .set_import_probe(Arc::new(FixedProbe {
            id: 1,
            hash: 9,
            decodable: true,
        }))
        .unwrap();
    let restored_thumbs = Arc::new(Thumbs::ready());
    reopened.set_thumbnailer(restored_thumbs.clone()).unwrap();
    assert_eq!(state(&reopened, 0), "probing");
    reopened.tick();
    finish(&mut reopened);
    assert_eq!(state(&reopened, 0), "ready");
    assert!(reopened.inspect().unwrap().assets[0].thumbnail);
    assert_eq!(
        restored_thumbs.calls(),
        0,
        "reopen restores the content-keyed image"
    );
    drop(reopened);
}

#[test]
fn unsupported_codec_is_typed_before_timeline_use_and_gets_a_placeholder() {
    let thumbs = Arc::new(Thumbs::ready());
    let mut editor = new_editor();
    editor
        .set_import_probe(Arc::new(FixedProbe {
            id: 1,
            hash: 4,
            decodable: false,
        }))
        .unwrap();
    editor.set_thumbnailer(thumbs.clone()).unwrap();
    editor
        .start_import(vec![
            std::env::temp_dir().join("vibecut-test/unsupported.mp4"),
        ])
        .unwrap();
    finish(&mut editor);
    let inspection = editor.inspect().unwrap();
    assert_eq!(inspection.assets[0].state, "unsupported");
    assert!(!inspection.assets[0].thumbnail);
    assert!(
        inspection.assets[0]
            .detail
            .as_deref()
            .is_some_and(|detail| detail.contains("decoder"))
    );
    assert_eq!(thumbs.calls(), 0, "unsupported media is not decoded");
    assert_eq!(editor.session.project().data().assets.len(), 1);
}

#[test]
fn unsupported_audio_in_a_decodable_video_keeps_the_unsupported_badge() {
    struct MixedProbe;
    impl ImportProbe for MixedProbe {
        fn import(
            &self,
            id: AssetId,
            path: PathBuf,
            _cancelled: Arc<AtomicBool>,
        ) -> media::Result<ImportedSource> {
            let mut data = AssetData::from(asset(id.0, 17, true, path.clone()));
            let mut audio = asset(id.0, 17, false, path).probe().streams[0].clone();
            audio.index = 1;
            data.probe.streams.push(audio);
            let asset = SourceAsset::try_from(data)?;
            let mut capabilities = capabilities(&asset, true);
            capabilities.streams[1].decodability =
                Decodability::Unsupported(UnsupportedReason::DecoderUnavailable);
            Ok(ImportedSource {
                asset,
                capabilities,
            })
        }
    }
    let thumbs = Arc::new(Thumbs::ready());
    let mut editor = new_editor();
    editor.set_import_probe(Arc::new(MixedProbe)).unwrap();
    editor.set_thumbnailer(thumbs.clone()).unwrap();
    editor
        .start_import(vec![std::env::temp_dir().join("vibecut-test/mixed.mp4")])
        .unwrap();
    finish(&mut editor);
    assert_eq!(state(&editor, 0), "unsupported");
    assert!(!editor.inspect().unwrap().assets[0].thumbnail);
    assert_eq!(thumbs.calls(), 0);
}

#[test]
fn failed_thumbnail_leaves_the_asset_ready_with_a_placeholder() {
    let thumbs = Arc::new(Thumbs::failing());
    let mut editor = new_editor();
    editor
        .set_import_probe(Arc::new(FixedProbe {
            id: 1,
            hash: 6,
            decodable: true,
        }))
        .unwrap();
    editor.set_thumbnailer(thumbs).unwrap();
    editor
        .start_import(vec![std::env::temp_dir().join("vibecut-test/no-thumb.mp4")])
        .unwrap();
    finish(&mut editor);
    let inspection = editor.inspect().unwrap();
    assert_eq!(inspection.assets[0].state, "ready");
    assert!(!inspection.assets[0].thumbnail);
    assert_eq!(editor.session.project().data().assets.len(), 1);
}

#[test]
fn media_panel_renders_probing_ready_and_unsupported_badges() {
    let root = tempfile::tempdir().unwrap();
    let release = Arc::new(AtomicBool::new(false));
    let thumbs = Arc::new(Thumbs::gated(release.clone()));
    let mut editor = new_editor();
    editor.set_thumbnail_root(root.path().to_path_buf());
    editor
        .set_import_probe(Arc::new(FixedProbe {
            id: 1,
            hash: 11,
            decodable: true,
        }))
        .unwrap();
    editor.set_thumbnailer(thumbs.clone()).unwrap();
    editor
        .start_import(vec![std::env::temp_dir().join("vibecut-test/shot.mp4")])
        .unwrap();
    tick_until(&mut editor, |_| thumbs.started());
    let mut harness = crate::gpu::builder()
        .with_size(egui::vec2(1728.0, 1200.0))
        .build_ui_state(|ui, editor: &mut Editor| editor.ui(ui), editor);
    harness.run_steps(3);
    let probing = harness.state().inspect().unwrap();
    assert_eq!(probing.assets[0].state, "probing");
    release.store(true, Ordering::Release);
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        harness.run_steps(2);
        if !harness.state().importing() {
            break;
        }
        assert!(Instant::now() < deadline, "import did not finish");
        std::thread::sleep(Duration::from_millis(2));
    }
    harness.run_steps(3);
    let ready = harness.state().inspect().unwrap();
    assert_eq!(ready.assets[0].state, "ready");
    assert!(ready.assets[0].thumbnail);
    for view in ["list", "grid"] {
        harness
            .state_mut()
            .set_preference("media_view", serde_json::json!(view))
            .unwrap();
        harness.run_steps(3);
        let widgets = harness.state().inspect().unwrap().widgets;
        for suffix in ["thumbnail", "status"] {
            assert!(
                widgets
                    .iter()
                    .any(|widget| widget.id == format!("media.asset.1.{suffix}")),
                "the {view} view must present the published {suffix}"
            );
        }
    }
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/p1-10-media.png");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    harness.render().unwrap().save(&path).unwrap();
    println!("media screenshot: {}", path.display());
}

struct ProbeGate {
    release: AtomicBool,
    started: AtomicBool,
    decodable: bool,
}
impl ImportProbe for ProbeGate {
    fn import(
        &self,
        id: AssetId,
        path: PathBuf,
        cancelled: Arc<AtomicBool>,
    ) -> media::Result<ImportedSource> {
        self.started.store(true, Ordering::Release);
        while !self.release.load(Ordering::Acquire) {
            if cancelled.load(Ordering::Acquire) {
                return Err(MediaError::Cancelled);
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        let asset = asset(id.0, 29, false, path);
        Ok(ImportedSource {
            capabilities: capabilities(&asset, self.decodable),
            asset,
        })
    }
}

#[test]
fn blocked_probe_has_a_pending_row_and_publishes_only_validated_metadata() {
    for decodable in [true, false] {
        let probe = Arc::new(ProbeGate {
            release: AtomicBool::new(false),
            started: AtomicBool::new(false),
            decodable,
        });
        let mut editor = new_editor();
        editor.set_import_probe(probe.clone()).unwrap();
        editor
            .start_import(vec![std::env::temp_dir().join("vibecut-test/pending.wav")])
            .unwrap();
        tick_until(&mut editor, |_| probe.started.load(Ordering::Acquire));
        editor.tick();
        assert!(editor.session.project().data().assets.is_empty());
        assert_eq!(state(&editor, 0), "probing");
        let mut harness = crate::gpu::builder()
            .with_size(egui::vec2(1728.0, 1200.0))
            .build_ui_state(|ui, editor: &mut Editor| editor.ui(ui), editor);
        harness.run_steps(2);
        assert!(
            harness
                .state()
                .inspect()
                .unwrap()
                .widgets
                .iter()
                .any(|w| w.id == "media.asset.1.status")
        );
        probe.release.store(true, Ordering::Release);
        let deadline = Instant::now() + Duration::from_secs(10);
        while harness.state().importing() {
            harness.run_steps(1);
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(harness.state().session.project().data().assets.len(), 1);
        assert_eq!(
            state(harness.state(), 0),
            if decodable { "ready" } else { "unsupported" }
        );
    }
}

#[test]
fn undo_redo_and_reopen_revalidate_capabilities_before_timeline_use() {
    let mut editor = new_editor();
    editor
        .set_import_probe(Arc::new(FixedProbe {
            id: 1,
            hash: 3,
            decodable: false,
        }))
        .unwrap();
    editor
        .start_import(vec![std::env::temp_dir().join("vibecut-test/refused.mp4")])
        .unwrap();
    finish(&mut editor);
    assert_eq!(state(&editor, 0), "unsupported");
    editor.session.undo().unwrap();
    editor.tick();
    editor.session.redo().unwrap();
    assert_eq!(state(&editor, 0), "probing");
    editor.select_media([AssetId(1)]);
    editor.activate("media.insert").unwrap();
    assert!(
        editor
            .session
            .project()
            .data()
            .scene()
            .timeline
            .iter()
            .all(|track| track.items.is_empty())
    );
    editor.tick();
    finish(&mut editor);
    assert_eq!(state(&editor, 0), "unsupported");
    let bytes = serde_json::to_vec(editor.session.project().as_ref()).unwrap();
    let restored: Project = serde_json::from_slice(&bytes).unwrap();
    let session = Session::new(restored, 50).unwrap();
    let playback = Playback::new(
        &session,
        ByteBudget::new(512 << 20).unwrap(),
        Arc::new(Files),
    )
    .unwrap();
    let mut reopened = Editor::new(session, playback, Preferences::default());
    reopened
        .set_import_probe(Arc::new(FixedProbe {
            id: 1,
            hash: 3,
            decodable: false,
        }))
        .unwrap();
    assert_eq!(state(&reopened, 0), "probing");
    reopened.select_media([AssetId(1)]);
    reopened.activate("media.insert").unwrap();
    assert!(
        reopened
            .session
            .project()
            .data()
            .scene()
            .timeline
            .iter()
            .all(|track| track.items.is_empty())
    );
    reopened.tick();
    finish(&mut reopened);
    assert_eq!(state(&reopened, 0), "unsupported");
}

#[test]
fn undo_releases_retained_thumbnail_and_texture_allowances() {
    let settled_bytes = |editor: &mut Editor| {
        let mut used = 0;
        tick_until(editor, |editor| {
            let snapshot = editor.playback.budget().snapshot().unwrap();
            used = snapshot.used;
            // Preview publishes its frame before dropping temporary compositor
            // buffers. Measure retained storage only after those leases drain.
            editor.playback.frame().is_some() && snapshot.owners[1] == 0
        });
        used
    };
    let mut editor = new_editor();
    let before = settled_bytes(&mut editor);
    struct SharedProbe;
    impl ImportProbe for SharedProbe {
        fn import(
            &self,
            id: AssetId,
            path: PathBuf,
            _: Arc<AtomicBool>,
        ) -> media::Result<ImportedSource> {
            let asset = asset(id.0, 3, true, path);
            Ok(ImportedSource {
                capabilities: capabilities(&asset, true),
                asset,
            })
        }
    }
    editor.set_import_probe(Arc::new(SharedProbe)).unwrap();
    let thumbs = Arc::new(Thumbs::ready());
    editor.set_thumbnailer(thumbs.clone()).unwrap();
    editor
        .start_import(vec![
            std::env::temp_dir().join("vibecut-test/pixels.mp4"),
            std::env::temp_dir().join("vibecut-test/same-pixels.mp4"),
        ])
        .unwrap();
    finish(&mut editor);
    let charge = settled_bytes(&mut editor) - before;
    assert!(
        (32..1024).contains(&charge),
        "pixels/uploads plus bounded probe metadata must be accounted"
    );
    assert_eq!(
        thumbs.calls(),
        1,
        "the duplicate content must reuse the retained image"
    );
    let texture_count = Arc::new(AtomicUsize::new(0));
    let uploaded = texture_count.clone();
    let mut harness = crate::gpu::builder()
        .with_size(egui::vec2(1728.0, 1200.0))
        .build_ui_state(
            move |ui, editor: &mut Editor| {
                editor.ui(ui);
                let count = ui
                    .ctx()
                    .tex_manager()
                    .read()
                    .allocated()
                    .filter(|(_, meta)| meta.name.starts_with("media.thumbnail."))
                    .count();
                uploaded.store(count, Ordering::SeqCst);
            },
            editor,
        );
    harness.run_steps(2);
    assert_eq!(
        texture_count.load(Ordering::SeqCst),
        1,
        "shared pixels must upload one texture"
    );
    assert_eq!(
        settled_bytes(harness.state_mut()) - before,
        charge,
        "upload uses the existing allowance"
    );
    harness.state_mut().session.undo().unwrap();
    harness.state_mut().tick();
    harness.run_steps(2);
    assert_eq!(
        settled_bytes(harness.state_mut()) - before,
        charge,
        "one alias still owns the allowance"
    );
    harness.state_mut().session.undo().unwrap();
    harness.state_mut().tick();
    harness.run_steps(2);
    assert_eq!(texture_count.load(Ordering::SeqCst), 0);
    assert!(harness.state().inspect().unwrap().assets.is_empty());
    assert_eq!(settled_bytes(harness.state_mut()), before);
}

#[test]
fn cancelling_thumbnail_work_keeps_validated_assets_ready_with_a_placeholder() {
    let mut editor = new_editor();
    let release = Arc::new(AtomicBool::new(false));
    let thumbs = Arc::new(Thumbs::gated(release));
    editor
        .set_import_probe(Arc::new(FixedProbe {
            id: 1,
            hash: 3,
            decodable: true,
        }))
        .unwrap();
    editor.set_thumbnailer(thumbs.clone()).unwrap();
    editor
        .start_import(vec![
            std::env::temp_dir().join("vibecut-test/cancel-thumb.mp4"),
        ])
        .unwrap();
    tick_until(&mut editor, |_| thumbs.started());
    editor.activate("import.cancel").unwrap();
    finish(&mut editor);
    assert!(editor.inspect().unwrap().import_cancelled);
    assert_eq!(state(&editor, 0), "ready");
    assert!(!editor.inspect().unwrap().assets[0].thumbnail);
    editor.select_media([AssetId(1)]);
    editor.activate("media.insert").unwrap();
    assert_eq!(
        editor
            .session
            .project()
            .data()
            .scene()
            .timeline
            .iter()
            .flat_map(|track| track.items.iter())
            .filter(|item| item.source_asset == Some(AssetId(1)))
            .count(),
        1
    );
}

#[test]
fn undo_during_probe_keeps_later_pending_and_thumbnail_identities() {
    struct SecondGate {
        started: AtomicBool,
        release: AtomicBool,
    }
    impl ImportProbe for SecondGate {
        fn import(
            &self,
            id: AssetId,
            path: PathBuf,
            cancelled: Arc<AtomicBool>,
        ) -> media::Result<ImportedSource> {
            if id == AssetId(2) {
                self.started.store(true, Ordering::Release);
                while !self.release.load(Ordering::Acquire) {
                    if cancelled.load(Ordering::Acquire) {
                        return Err(MediaError::Cancelled);
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
            }
            let asset = asset(id.0, id.0 as u8, true, path);
            Ok(ImportedSource {
                capabilities: capabilities(&asset, true),
                asset,
            })
        }
    }
    let gate = Arc::new(SecondGate {
        started: AtomicBool::new(false),
        release: AtomicBool::new(false),
    });
    let mut editor = new_editor();
    editor.set_import_probe(gate.clone()).unwrap();
    editor.set_thumbnailer(Arc::new(Thumbs::ready())).unwrap();
    editor
        .start_import(vec![
            std::env::temp_dir().join("vibecut-test/first.mp4"),
            std::env::temp_dir().join("vibecut-test/second.mp4"),
        ])
        .unwrap();
    tick_until(&mut editor, |editor| {
        gate.started.load(Ordering::Acquire) && editor.session.project().data().assets.len() == 1
    });
    editor.session.undo().unwrap();
    editor.tick();
    assert!(editor.session.project().data().assets.is_empty());
    let pending = editor.inspect().unwrap().assets;
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].id, AssetId(2));
    assert_eq!(pending[0].state, "probing");
    gate.release.store(true, Ordering::Release);
    finish(&mut editor);
    let inspected = editor.inspect().unwrap().assets;
    assert_eq!(inspected.len(), 1);
    assert_eq!(inspected[0].id, AssetId(2));
    assert_eq!(inspected[0].state, "ready");
    assert!(
        inspected[0].thumbnail,
        "the later thumbnail must reach the same asset identity"
    );
}

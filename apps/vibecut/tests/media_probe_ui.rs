//! Media library status, thumbnail cache and placeholder behaviour.
#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::expect_used
)]
use egui_kittest::Harness;
use engine::{ByteBudget, Session};
use media::{
    AssetData, AssetId, ColorInfo, ContentHash, Decodability, ImportedSource, MediaError,
    ProbeCapabilities, SourceAsset, SourceProbe, SourceStream, StreamCapability, StreamKind,
    Thumbnail, TimeBase, UnsupportedReason, VideoInfo,
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
use time::{FrameRate, Tick};
use vibecut::{
    Editor,
    catalog::Preferences,
    egui,
    import::{ImportProbe, ThumbnailSource},
    playback::{Files, Playback},
};

const SECOND: i64 = time::TICKS_PER_SECOND;

fn asset(id: u64, hash: u8, video: bool, path: impl Into<PathBuf>) -> SourceAsset {
    let kind = if video {
        StreamKind::Video
    } else {
        StreamKind::Audio
    };
    SourceAsset::try_from(AssetData {
        id: AssetId(id),
        path: path.into(),
        content_hash: ContentHash([hash; 32]),
        probe: SourceProbe {
            format: "mov,mp4".into(),
            duration: Some(Tick(4 * SECOND)),
            streams: vec![SourceStream {
                index: 0,
                kind,
                codec: if video { "h264".into() } else { "aac".into() },
                time_base: TimeBase::new(1, 1_000).unwrap(),
                duration: Some(Tick(4 * SECOND)),
                start: None,
                frame_rate: video.then_some(FrameRate::FPS_30),
                frame_count: None,
                variable_frame_rate: None,
                video: video.then_some(VideoInfo {
                    width: 320,
                    height: 180,
                    color: ColorInfo::default(),
                    bit_depth: Some(8),
                }),
                audio: (!video).then_some(media::AudioInfo {
                    sample_rate: 48_000,
                    channels: 2,
                    initial_padding: 0,
                    trailing_padding: 0,
                }),
            }],
        },
    })
    .unwrap()
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
        .start_import(vec!["C:/vibecut-test/probing.mp4".into()])
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
    let root = std::env::temp_dir().join(format!(
        "vibecut-thumbnail-test-{}-reuse",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let first = Arc::new(Thumbs::ready());
    let mut editor = new_editor();
    editor.set_thumbnail_root(root.clone());
    editor
        .set_import_probe(Arc::new(FixedProbe {
            id: 1,
            hash: 9,
            decodable: true,
        }))
        .unwrap();
    editor.set_thumbnailer(first.clone()).unwrap();
    editor
        .start_import(vec!["C:/vibecut-test/reused.mp4".into()])
        .unwrap();
    finish(&mut editor);
    assert_eq!(first.calls(), 1);
    drop(editor);
    let second = Arc::new(Thumbs::ready());
    let mut editor = new_editor();
    editor.set_thumbnail_root(root.clone());
    editor
        .set_import_probe(Arc::new(FixedProbe {
            id: 1,
            hash: 9,
            decodable: true,
        }))
        .unwrap();
    editor.set_thumbnailer(second.clone()).unwrap();
    editor
        .start_import(vec!["C:/vibecut-test/reused.mp4".into()])
        .unwrap();
    finish(&mut editor);
    assert_eq!(second.calls(), 0, "cached frame must not decode again");
    assert!(editor.inspect().unwrap().assets[0].thumbnail);
    let _ = std::fs::remove_dir_all(root);
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
        .start_import(vec!["C:/vibecut-test/unsupported.mp4".into()])
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
        .start_import(vec!["C:/vibecut-test/no-thumb.mp4".into()])
        .unwrap();
    finish(&mut editor);
    let inspection = editor.inspect().unwrap();
    assert_eq!(inspection.assets[0].state, "ready");
    assert!(!inspection.assets[0].thumbnail);
    assert_eq!(editor.session.project().data().assets.len(), 1);
}

#[test]
fn media_panel_renders_probing_ready_and_unsupported_badges() {
    let root = std::env::temp_dir().join(format!(
        "vibecut-thumbnail-test-{}-shot",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let release = Arc::new(AtomicBool::new(false));
    let thumbs = Arc::new(Thumbs::gated(release.clone()));
    let mut editor = new_editor();
    editor.set_thumbnail_root(root.clone());
    editor
        .set_import_probe(Arc::new(FixedProbe {
            id: 1,
            hash: 11,
            decodable: true,
        }))
        .unwrap();
    editor.set_thumbnailer(thumbs.clone()).unwrap();
    editor
        .start_import(vec!["C:/vibecut-test/shot.mp4".into()])
        .unwrap();
    tick_until(&mut editor, |_| thumbs.started());
    let mut harness = Harness::builder()
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
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/p1-10-media.png");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    harness.render().unwrap().save(&path).unwrap();
    println!("media screenshot: {}", path.display());
    let _ = std::fs::remove_dir_all(root);
}

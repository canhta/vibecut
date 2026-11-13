#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::expect_used
)]
use egui_kittest::{
    Harness,
    kittest::{By, Queryable},
};
use engine::{ByteBudget, Command, Session, jobs::Clock};
use media::{
    AssetData, AssetId, ContentHash, Decodability, ImportedSource, MediaError, ProbeCapabilities,
    SourceAsset, SourceStream, StreamCapability, StreamKind, Thumbnail,
};
use project::{ItemParams, Project, ProjectId};
use std::{
    path::PathBuf,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use time::{SourceRange, TICKS_PER_SECOND, Tick};
use vibecut::{
    AppCommand, Editor, ImportProbe, MediaSort, MediaSortKey, MediaSortOrder, MediaView,
    ThumbnailSource, egui,
    preferences::{PreferenceStore, Preferences},
    services::playback::{Files, Playback},
};

const SECOND: i64 = TICKS_PER_SECOND;

fn stream(kind: StreamKind, duration: Tick) -> SourceStream {
    testkit::source::stream(kind).duration(duration).build()
}

fn source_asset(id: u64, name: &str, duration: Option<Tick>) -> SourceAsset {
    let mut builder = testkit::source::asset(
        id,
        std::env::temp_dir()
            .join("vibecut-media-library")
            .join(name),
    )
    .format("mov,mp4")
    .content_hash(ContentHash::from_bytes(&[id as u8; 8]))
    .stream(stream(StreamKind::Video, duration.unwrap_or(Tick::ZERO)));
    if let Some(duration) = duration {
        builder = builder.duration(duration);
    }
    builder.build()
}

fn editor_with(preferences: Preferences, assets: &[(u64, &str)]) -> Editor {
    let mut session =
        Session::new(Project::empty(ProjectId(90), "Media".into()).unwrap(), 50).unwrap();
    for (id, name) in assets {
        session
            .apply(Command::AddAsset {
                asset: source_asset(*id, name, Some(Tick(4 * SECOND))),
            })
            .unwrap();
    }
    let mut playback = Playback::new(
        &session,
        ByteBudget::new(512 << 20).unwrap(),
        Arc::new(Files),
    )
    .unwrap();
    playback.set_device_available(true);
    let records = session.project().data().assets.clone();
    let mut editor = Editor::new(session, playback, preferences);
    editor
        .set_import_probe(Arc::new(RestoredProbe(records)))
        .unwrap();
    editor.set_thumbnailer(Arc::new(NoThumbnails)).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        editor.tick();
        if !editor.importing()
            && editor
                .inspect()
                .unwrap()
                .assets
                .iter()
                .all(|asset| asset.state == "ready")
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "fixture restoration did not finish"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    editor
}

struct RestoredProbe(Vec<SourceAsset>);
impl ImportProbe for RestoredProbe {
    fn import(
        &self,
        id: AssetId,
        _path: PathBuf,
        _cancelled: Arc<AtomicBool>,
    ) -> media::Result<ImportedSource> {
        let asset = self
            .0
            .iter()
            .find(|asset| asset.id() == id)
            .cloned()
            .ok_or(MediaError::Invalid("fixture asset"))?;
        Ok(imported(asset))
    }
}
struct NoThumbnails;
impl ThumbnailSource for NoThumbnails {
    fn thumbnail(
        &self,
        _asset: &SourceAsset,
        _at: Tick,
        _max_edge: u32,
        _cancelled: Arc<AtomicBool>,
    ) -> media::Result<Thumbnail> {
        Err(MediaError::Unsupported("fixture thumbnail"))
    }
}
fn imported(asset: SourceAsset) -> ImportedSource {
    let capabilities = ProbeCapabilities {
        streams: asset
            .probe()
            .streams
            .iter()
            .map(|stream| StreamCapability {
                index: stream.index,
                kind: stream.kind,
                codec: stream.codec.clone(),
                decodability: Decodability::Decodable,
            })
            .collect(),
    };
    ImportedSource {
        asset,
        capabilities,
    }
}

fn editor() -> Editor {
    editor_with(
        Preferences::default(),
        &[(1, "Bravo.mp4"), (2, "alpha.mp4"), (3, "charlie.mp4")],
    )
}

fn build(editor: Editor) -> Harness<'static, Editor> {
    crate::gpu::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .build_ui_state(|ui, editor: &mut Editor| editor.ui(ui), editor)
}

fn click(harness: &mut Harness<Editor>, id: &str) {
    harness
        .get(By::new().predicate(|node| node.author_id() == Some(id)))
        .click();
    harness.run_steps(2);
}

fn click_modifiers(harness: &mut Harness<Editor>, id: &str, modifiers: egui::Modifiers) {
    harness
        .get(By::new().predicate(|node| node.author_id() == Some(id)))
        .click_modifiers(modifiers);
    harness.run_steps(2);
}

fn has_widget(harness: &Harness<Editor>, id: &str) -> bool {
    harness
        .state()
        .inspect()
        .unwrap()
        .widgets
        .iter()
        .any(|widget| widget.id == id)
}

/// A clock the test advances explicitly, so timed UI state never depends on
/// how long a frame took on a loaded host.
#[derive(Default)]
struct ManualClock(AtomicU64);
impl ManualClock {
    fn advance(&self, duration: Duration) {
        self.0
            .fetch_add(duration.as_millis() as u64, Ordering::SeqCst);
    }
}
impl Clock for ManualClock {
    fn now(&self) -> Duration {
        Duration::from_millis(self.0.load(Ordering::SeqCst))
    }
}

fn widget_rect(harness: &Harness<Editor>, id: &str) -> egui::Rect {
    harness
        .state()
        .inspect()
        .unwrap()
        .widgets
        .into_iter()
        .find(|widget| widget.id == id)
        .unwrap_or_else(|| panic!("missing widget {id}"))
        .rect
}

fn insert_video(editor: &mut Editor, asset: AssetId) -> project::ItemId {
    let main = editor.session.project().data().scene().timeline.main.id;
    let outcome = editor
        .session
        .apply(Command::InsertItem {
            placement: engine::Placement::Track { track: main },
            item: engine::NewItem {
                retime: Default::default(),
                animations: Default::default(),
                source_asset: Some(asset),
                source_range: SourceRange::new(Tick::ZERO, Tick(SECOND)).unwrap(),
                start: Tick::ZERO,
                content: ItemParams::Video(project::VideoParams {
                    fades: Default::default(),
                    source_audio_enabled: true,
                    volume: 0.0,
                    muted: false,
                    crop: None,
                    cleanup: Default::default(),
                    reduce_noise: false,
                }),
            },
        })
        .unwrap();
    match outcome.created.first() {
        Some(engine::CreatedId::Item(id)) => *id,
        _ => panic!("item"),
    }
}

#[test]
fn view_and_sort_toggle_through_real_widgets_and_round_trip_through_the_store() {
    let dir = tempfile::tempdir().unwrap();
    let store = PreferenceStore::new(dir.path().join("preferences.json"));
    let mut harness = build(editor());
    harness.run_steps(3);
    assert_eq!(harness.state().preferences.media_view, MediaView::Grid);
    assert_eq!(
        harness.state().inspect().unwrap().media_order,
        vec![AssetId(2), AssetId(1), AssetId(3)],
        "default name ascending order"
    );

    click(&mut harness, "media.view");
    assert_eq!(harness.state().preferences.media_view, MediaView::List);
    assert!(has_widget(&harness, "media.asset.2"));

    click(&mut harness, "media.sort");
    click(&mut harness, "media.sort.name");
    assert_eq!(
        harness.state().preferences.media_sort,
        MediaSort {
            key: MediaSortKey::Name,
            order: MediaSortOrder::Desc
        },
        "clicking the active key flips the order"
    );
    assert_eq!(
        harness.state().inspect().unwrap().media_order,
        vec![AssetId(3), AssetId(1), AssetId(2)]
    );

    click(&mut harness, "media.sort");
    click(&mut harness, "media.sort.size");
    assert_eq!(
        harness.state().preferences.media_sort,
        MediaSort {
            key: MediaSortKey::Size,
            order: MediaSortOrder::Asc
        },
        "a different key starts ascending"
    );

    store.save(&harness.state().preferences).unwrap();
    let restored = store.load_with_warning().0;
    assert_eq!(restored.media_view, MediaView::List);
    assert_eq!(restored.media_sort.key, MediaSortKey::Size);
    assert_eq!(restored.media_sort.order, MediaSortOrder::Asc);

    let mut restarted = build(editor_with(
        restored,
        &[(1, "Bravo.mp4"), (2, "alpha.mp4"), (3, "charlie.mp4")],
    ));
    restarted.run_steps(3);
    assert_eq!(restarted.state().preferences.media_view, MediaView::List);
    assert_eq!(
        restarted.state().preferences.media_sort.key,
        MediaSortKey::Size
    );
}

#[test]
fn shift_range_marquee_blank_click_and_escape_follow_the_sorted_order() {
    let mut harness = build(editor());
    harness.run_steps(3);
    click(&mut harness, "media.asset.2");
    assert_eq!(
        harness.state().inspect().unwrap().media_selection,
        vec![AssetId(2)]
    );
    click_modifiers(&mut harness, "media.asset.3", egui::Modifiers::SHIFT);
    assert_eq!(
        harness.state().inspect().unwrap().media_selection,
        vec![AssetId(1), AssetId(2), AssetId(3)],
        "shift extends over the displayed order"
    );
    click_modifiers(&mut harness, "media.asset.1", egui::Modifiers::SHIFT);
    assert_eq!(
        harness.state().inspect().unwrap().media_selection,
        vec![AssetId(1), AssetId(2)],
        "reversing a Shift range retains the original anchor"
    );

    let card = widget_rect(&harness, "media.asset.3");
    let right = (1..=3)
        .map(|id| widget_rect(&harness, &format!("media.asset.{id}")).right())
        .fold(0.0_f32, f32::max);
    let start = egui::pos2(right + 8.0, card.center().y);
    let end = widget_rect(&harness, "media.marquee").left_top() + egui::vec2(2.0, 2.0);
    harness.hover_at(start);
    harness.drag_at(start);
    harness.run_steps(1);
    harness.hover_at(end);
    harness.run_steps(1);
    harness.hover_at(start);
    harness.run_steps(1);
    assert!(
        harness
            .state()
            .inspect()
            .unwrap()
            .media_selection
            .is_empty(),
        "reversing the marquee removes assets outside its current bounds"
    );
    harness.hover_at(end);
    harness.run_steps(1);
    harness.drop_at(end);
    harness.run_steps(2);
    assert_eq!(
        harness.state().inspect().unwrap().media_selection,
        vec![AssetId(1), AssetId(2), AssetId(3)],
        "marquee intersects every card"
    );

    harness.hover_at(start);
    harness.drag_at(start);
    harness.run_steps(1);
    harness.hover_at(end);
    harness.run_steps(1);
    harness.key_press(egui::Key::Escape);
    harness.run_steps(2);
    assert!(
        harness
            .state()
            .inspect()
            .unwrap()
            .media_selection
            .is_empty(),
        "Escape cancels an active marquee and clears media selection"
    );
    harness.drop_at(end);
    harness.run_steps(2);
    assert!(
        harness
            .state()
            .inspect()
            .unwrap()
            .media_selection
            .is_empty(),
        "release after cancellation must not reapply the marquee"
    );

    let blank = start;
    harness.hover_at(blank);
    harness.drag_at(blank);
    harness.run_steps(1);
    harness.drop_at(blank);
    harness.run_steps(2);
    assert!(
        harness
            .state()
            .inspect()
            .unwrap()
            .media_selection
            .is_empty(),
        "blank click clears"
    );

    click(&mut harness, "media.asset.1");
    harness.key_press(egui::Key::Escape);
    harness.run_steps(2);
    assert!(
        harness
            .state()
            .inspect()
            .unwrap()
            .media_selection
            .is_empty(),
        "Escape clears"
    );
}

#[test]
fn blank_space_below_media_clears_selection_in_both_views() {
    for view in [MediaView::Grid, MediaView::List] {
        let preferences = Preferences {
            media_view: view,
            ..Preferences::default()
        };
        let mut harness = build(editor_with(preferences, &[(1, "one.mp4")]));
        harness.run_steps(3);
        click(&mut harness, "media.asset.1");
        let row = widget_rect(&harness, "media.asset.1");
        let blank = egui::pos2(row.center().x, row.bottom() + 24.0);
        harness.hover_at(blank);
        harness.drag_at(blank);
        harness.run_steps(1);
        harness.drop_at(blank);
        harness.run_steps(2);
        assert!(
            harness
                .state()
                .inspect()
                .unwrap()
                .media_selection
                .is_empty(),
            "blank space below the last asset must clear {view:?} selection"
        );
    }
}

#[test]
fn delete_confirmation_refuses_in_use_media_then_removes_unused_media_with_undo() {
    let mut editor = editor();
    editor.preferences.media_view = MediaView::List;
    let item = insert_video(&mut editor, AssetId(1));
    let mut harness = build(editor);
    harness.run_steps(3);

    click(&mut harness, "media.asset.1");
    harness
        .get(By::new().predicate(|node| node.author_id() == Some("media.asset.1")))
        .click_secondary();
    harness.run_steps(2);
    click(&mut harness, "media.delete");
    assert_eq!(
        harness.state().inspect().unwrap().media_delete_pending,
        vec![AssetId(1)]
    );
    click(&mut harness, "media.confirm_delete");
    let inspection = harness.state().inspect().unwrap();
    assert_eq!(
        harness.state().session.project().data().assets.len(),
        3,
        "in-use removal changes nothing"
    );
    assert!(
        inspection
            .import_notice
            .as_deref()
            .is_some_and(|notice| notice.contains("used by timeline items")),
        "typed refusal lists the usage: {:?}",
        inspection.import_notice
    );
    assert!(
        harness
            .state()
            .session
            .project()
            .data()
            .assets
            .iter()
            .any(|a| a.id() == AssetId(1))
    );
    assert_eq!(
        harness
            .state()
            .session
            .project()
            .data()
            .scene()
            .timeline
            .main
            .items[0]
            .id,
        item
    );

    click(&mut harness, "media.asset.2");
    click_modifiers(&mut harness, "media.asset.3", egui::Modifiers::COMMAND);
    harness
        .get(By::new().predicate(|node| node.author_id() == Some("media.asset.2")))
        .click_secondary();
    harness.run_steps(2);
    click(&mut harness, "media.delete");
    assert_eq!(
        harness.state().inspect().unwrap().media_delete_pending,
        vec![AssetId(2), AssetId(3)],
        "the context menu deletes the whole selection"
    );
    click(&mut harness, "media.confirm_delete");
    assert_eq!(
        harness
            .state()
            .session
            .project()
            .data()
            .assets
            .iter()
            .map(SourceAsset::id)
            .collect::<Vec<_>>(),
        vec![AssetId(1)]
    );

    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
    harness.run_steps(2);
    assert_eq!(
        harness.state().session.project().data().assets.len(),
        3,
        "undo restores both removed records"
    );
    harness.key_press_modifiers(
        egui::Modifiers::COMMAND.plus(egui::Modifiers::SHIFT),
        egui::Key::Z,
    );
    harness.run_steps(2);
    assert_eq!(
        harness.state().session.project().data().assets.len(),
        1,
        "redo removes them again"
    );
}

#[test]
fn reveal_switches_tab_scrolls_and_expires_without_touching_project_json() {
    let assets: Vec<(u64, String)> = (1..=30)
        .map(|id| (id, format!("asset-{id:02}.mp4")))
        .collect();
    let assets: Vec<(u64, &str)> = assets
        .iter()
        .map(|(id, name)| (*id, name.as_str()))
        .collect();
    let clock = Arc::new(ManualClock::default());
    let mut editor = editor_with(Preferences::default(), &assets);
    editor.set_clock(clock.clone());
    let mut harness = build(editor);
    harness.run_steps(3);
    click(&mut harness, "assets.tab.sounds");
    let before = serde_json::to_vec(harness.state().session.project().data()).unwrap();
    harness
        .state_mut()
        .dispatch(AppCommand::RevealMedia(AssetId(30)))
        .unwrap();
    harness.run_steps(3);
    let inspection = harness.state().inspect().unwrap();
    assert_eq!(
        harness.state().preferences.assets_tab,
        vibecut::AssetsTab::Media
    );
    assert_eq!(inspection.media_highlight, Some(AssetId(30)));
    let media_panel = harness.state().panels()["media"];
    assert!(
        widget_rect(&harness, "media.asset.30").intersects(media_panel),
        "reveal scrolls the asset into the media panel"
    );
    let after = serde_json::to_vec(harness.state().session.project().data()).unwrap();
    assert_eq!(before, after, "reveal never changes project JSON");

    // The 1.5 s highlight is driven by the injected clock, so this asserts the
    // deadline boundary itself rather than racing a sleep against frame time.
    clock.advance(Duration::from_millis(1499));
    harness.run_steps(2);
    assert_eq!(
        harness.state().inspect().unwrap().media_highlight,
        Some(AssetId(30)),
        "the highlight is still visible just before the 1.5 s deadline"
    );
    clock.advance(Duration::from_millis(1));
    harness.state_mut().tick();
    harness.run_steps(2);
    assert_eq!(
        harness.state().inspect().unwrap().media_highlight,
        None,
        "the 1.5 s highlight expires at its deadline"
    );
}

#[test]
fn timeline_clip_context_menu_reveals_the_source_media() {
    let mut editor = editor_with(
        Preferences::default(),
        &[(1, "Bravo.mp4"), (2, "alpha.mp4"), (3, "charlie.mp4")],
    );
    let item = insert_video(&mut editor, AssetId(3));
    let mut harness = build(editor);
    harness.run_steps(3);
    click(&mut harness, "assets.tab.sounds");
    harness
        .get(
            By::new()
                .predicate(|node| node.author_id() == Some(&format!("timeline.item.{}", item.0))),
        )
        .click_secondary();
    harness.run_steps(2);
    click(&mut harness, &format!("timeline.item.{}.reveal", item.0));
    harness.run_steps(3);
    assert_eq!(
        harness.state().preferences.assets_tab,
        vibecut::AssetsTab::Media
    );
    assert_eq!(
        harness.state().inspect().unwrap().media_highlight,
        Some(AssetId(3))
    );
}

struct GateProbe {
    calls: AtomicUsize,
    second: (Mutex<bool>, Condvar),
    release: (Mutex<bool>, Condvar),
}
impl GateProbe {
    fn new() -> Self {
        Self {
            calls: AtomicUsize::new(0),
            second: (Mutex::new(false), Condvar::new()),
            release: (Mutex::new(false), Condvar::new()),
        }
    }
    fn second_started(&self) -> bool {
        *self.second.0.lock().unwrap()
    }
    fn release(&self) {
        *self.release.0.lock().unwrap() = true;
        self.release.1.notify_all();
    }
}
impl ImportProbe for GateProbe {
    fn import(
        &self,
        id: AssetId,
        path: PathBuf,
        _cancelled: Arc<AtomicBool>,
    ) -> media::Result<ImportedSource> {
        if self.calls.fetch_add(1, Ordering::SeqCst) == 1 {
            *self.second.0.lock().unwrap() = true;
            self.second.1.notify_all();
            let mut released = self.release.0.lock().unwrap();
            while !*released {
                released = self.release.1.wait(released).unwrap();
            }
        }
        let name = path
            .file_name()
            .map_or_else(String::new, |name| name.to_string_lossy().into_owned());
        let asset = source_asset(id.0, &name, Some(Tick(4 * SECOND)));
        Ok(imported(asset))
    }
}

#[test]
fn import_batch_shows_progress_and_disables_view_sort_import_and_insert() {
    let mut editor = editor();
    editor.select_media([AssetId(1)]);
    let probe = Arc::new(GateProbe::new());
    editor.set_import_probe(probe.clone()).unwrap();
    editor
        .drop_files(
            (1..=3)
                .map(|index| {
                    std::env::temp_dir()
                        .join("vibecut-media-library")
                        .join(format!("gate-{index}.mp4"))
                })
                .collect(),
        )
        .unwrap();
    let mut harness = build(editor);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        harness.run_steps(1);
        if probe.second_started() && harness.state().inspect().unwrap().import_completed >= 1 {
            break;
        }
        assert!(Instant::now() < deadline, "import never progressed");
        std::thread::sleep(Duration::from_millis(2));
    }
    let inspection = harness.state().inspect().unwrap();
    assert!(inspection.importing);
    assert!(inspection.import_total >= inspection.import_completed);
    for id in ["media.view", "media.sort", "file.import", "media.insert"] {
        let state = inspection
            .widgets
            .iter()
            .find(|widget| widget.id == id)
            .unwrap_or_else(|| panic!("missing {id}"));
        assert!(!state.enabled, "{id} is disabled during a batch");
    }
    assert!(
        inspection
            .widgets
            .iter()
            .any(|widget| widget.id == "import.cancel" && widget.enabled)
    );
    click(&mut harness, "import.cancel");
    probe.release();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        harness.run_steps(1);
        if !harness.state().importing() {
            break;
        }
        assert!(Instant::now() < deadline, "cancel did not finish");
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(harness.state().inspect().unwrap().import_cancelled);
}

/// The captured grid has one thumbnail followed by a single filename line.
#[test]
fn grid_card_fits_its_measured_thumbnail_and_filename() {
    let mut harness = build(editor());
    harness.run_steps(3);
    let card = widget_rect(&harness, "media.asset.1");
    assert_eq!(card.width(), 120.0);
    assert_eq!(card.height(), 73.0 + 6.0 + 16.0);
}

/// Offscreen screenshot evidence; wgpu rendering stays out of CI.
#[test]
#[ignore = "renders through wgpu; run with --ignored to refresh target/p1-09-media.png"]
fn media_library_renders_a_representative_grid() {
    let mut harness = build(editor());
    harness.run_steps(3);
    click(&mut harness, "media.asset.2");
    let image = harness.render().expect("offscreen render");
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let target = root.join("target");
    std::fs::create_dir_all(&target).unwrap();
    image.save(target.join("p1-09-media.png")).unwrap();
    assert!(image.width() > 0 && image.height() > 0);
}

#[test]
fn search_and_type_filter_change_visible_assets_without_editing_the_project() {
    for (query, expected) in [
        ("BRAVO", vec![1]),
        ("alpha", vec![2]),
        ("missing", vec![]),
        ("ĐÀ", vec![1]),
    ] {
        let mut harness = build(editor_with(
            Preferences::default(),
            &[
                (1, "Bravo-Đà Nẵng.mp4"),
                (2, "alpha.mp4"),
                (3, "charlie.mp4"),
            ],
        ));
        harness.run_steps(3);
        let before = harness.state().session.project().clone();
        click(&mut harness, "assets.search");
        harness
            .get(By::new().predicate(|node| node.author_id() == Some("assets.search")))
            .type_text(query);
        harness.run_steps(3);
        for id in 1..=3 {
            assert_eq!(
                has_widget(&harness, &format!("media.asset.{id}")),
                expected.contains(&id),
                "query {query}, asset {id}"
            );
        }
        assert!(
            Arc::ptr_eq(&before, harness.state().session.project()),
            "search is presentation only"
        );
    }
    let mut harness = build(editor());
    harness.run_steps(3);
    let before = harness.state().session.project().clone();
    for (filter, visible) in [
        ("audio", false),
        ("video", true),
        ("image", false),
        ("all", true),
    ] {
        click(&mut harness, "assets.filter");
        click(&mut harness, &format!("assets.filter.{filter}"));
        for id in 1..=3 {
            assert_eq!(
                has_widget(&harness, &format!("media.asset.{id}")),
                visible,
                "filter {filter}, asset {id}"
            );
        }
        assert!(
            Arc::ptr_eq(&before, harness.state().session.project()),
            "filter is presentation only"
        );
    }
}

#[test]
fn local_audio_row_adds_its_source_through_the_existing_insert_command() {
    let mut editor = editor();
    let source = editor.session.project().data().assets[0].clone();
    let mut data: AssetData = source.into();
    data.id = AssetId(4);
    data.path = std::env::temp_dir().join("audio-music.wav");
    data.content_hash = ContentHash::from_bytes(&[4; 8]);
    data.probe.streams = vec![stream(StreamKind::Audio, Tick(4 * SECOND))];
    let asset = SourceAsset::try_from(data).unwrap();
    editor.session.apply(Command::AddAsset { asset }).unwrap();
    editor
        .set_import_probe(Arc::new(RestoredProbe(
            editor.session.project().data().assets.clone(),
        )))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        editor.tick();
        if !editor.importing()
            && editor
                .inspect()
                .unwrap()
                .assets
                .iter()
                .all(|asset| asset.state == "ready")
        {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
    editor.preferences.assets_tab = vibecut::AssetsTab::Media;
    let mut harness = build(editor);
    harness.run_steps(3);
    assert!(has_widget(&harness, "media.asset.4"));
    assert!(has_widget(&harness, "media.asset.1"));
    click(&mut harness, "media.asset.4");
    click(&mut harness, "media.insert");
    let items: Vec<_> = harness
        .state()
        .session
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| &track.items)
        .collect();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].source_asset, Some(AssetId(4)));
    assert!(matches!(items[0].content, ItemParams::Audio(_)));
}

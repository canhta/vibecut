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
    services::playback::{Files, Playback, Sources},
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
    editor_with_sources(preferences, assets, Arc::new(Files))
}

fn editor_with_sources(
    preferences: Preferences,
    assets: &[(u64, &str)],
    sources: Arc<dyn Sources>,
) -> Editor {
    let mut session =
        Session::new(Project::empty(ProjectId(90), "Media".into()).unwrap(), 50).unwrap();
    for (id, name) in assets {
        session
            .apply(Command::AddAsset {
                asset: source_asset(*id, name, Some(Tick(4 * SECOND))),
            })
            .unwrap();
    }
    let mut playback =
        Playback::new(&session, ByteBudget::new(512 << 20).unwrap(), sources).unwrap();
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
                    source_audio_enabled: true,
                    ..project::VideoParams::plain()
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
fn feature_tabs_never_touch_and_the_media_sub_navigation_lists_two_rows() {
    for locale in [vibecut::i18n::Locale::En, vibecut::i18n::Locale::Vi] {
        let editor = editor_with(
            Preferences {
                locale,
                ..Preferences::default()
            },
            &[(1, "alpha.mp4")],
        );
        // Wide enough for the 120 px sub-navigation column.
        let mut harness = crate::gpu::builder()
            .with_size(egui::vec2(1727.0, 1029.0))
            .build_ui_state(|ui, editor: &mut Editor| editor.ui(ui), editor);
        harness.run_steps(3);
        let tabs = [
            "media",
            "sounds",
            "text",
            "stickers",
            "effects",
            "transitions",
            "captions",
            "adjustment",
        ]
        .map(|tab| widget_rect(&harness, &format!("assets.tab.{tab}")));
        for pair in tabs.windows(2) {
            assert!(
                pair[0].right() <= pair[1].left(),
                "{locale:?}: tab cells overlap: {pair:?}"
            );
        }
        // Import heads the group and Media is the shown view: two separate rows.
        let import = widget_rect(&harness, "assets.subnav");
        let media = widget_rect(&harness, "assets.subnav.media");
        assert!(import.bottom() <= media.top(), "{locale:?}");
    }
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

/// No-match results offer a clear action that restores every asset; the project is untouched.
#[test]
fn search_no_matches_clear_restores_every_asset() {
    let mut harness = build(editor_with(
        Preferences::default(),
        &[(1, "bravo.mp4"), (2, "alpha.mp4")],
    ));
    harness.run_steps(3);
    let before = harness.state().session.project().clone();
    click(&mut harness, "assets.search");
    harness
        .get(By::new().predicate(|node| node.author_id() == Some("assets.search")))
        .type_text("missing");
    harness.run_steps(3);
    assert!(!has_widget(&harness, "media.asset.1"));
    click(&mut harness, "media.search.clear");
    harness.run_steps(3);
    assert!(has_widget(&harness, "media.asset.1"));
    assert!(has_widget(&harness, "media.asset.2"));
    assert!(Arc::ptr_eq(&before, harness.state().session.project()));
}

/// Export clips from the Media tab. The job's source is held on a gate, so the running window is
/// deterministic: the strip and the busy reason show while it runs, Cancel ends it, and a
/// missing folder refuses the start with no job. The non-video reason is carried on the item.
#[test]
fn clip_export_menu_refuses_with_reason_and_cancel_ends_the_running_job() {
    let gate = Arc::new(ExportGate::default());
    let _release = ReleaseOnDrop(gate.clone());
    let dir = tempfile::tempdir().unwrap();
    let mut harness = build(gated_export_editor(gate.clone()));
    harness.run_steps(3);
    let before = harness.state().session.project().clone();

    // A non-video target disables the item and carries its reason.
    open_menu(&mut harness, "media.asset.9");
    let (enabled, reason) = export_item(&harness);
    assert!(!enabled);
    assert_eq!(reason.as_deref(), Some("Select only video clips to export"));
    harness.key_press(egui::Key::Escape);
    harness.run_steps(2);

    // A folder that does not exist refuses the start; no job runs.
    queue_selection_export(&mut harness);
    harness
        .state_mut()
        .export_clips_picked(dir.path().join("missing"))
        .unwrap();
    let refusal = harness.state().inspect().unwrap().import_toast.unwrap();
    assert!(
        refusal.starts_with("Clip export did not start"),
        "{refusal}"
    );
    assert!(!has_widget(&harness, "media.export.cancel"));

    // A real folder: the job starts and holds its source on the gate.
    queue_selection_export(&mut harness);
    gate.arm();
    harness
        .state_mut()
        .export_clips_picked(dir.path().to_path_buf())
        .unwrap();
    gate.wait_entered(&mut harness);
    harness.run_steps(2);
    assert!(
        has_widget(&harness, "media.export.cancel"),
        "strip while running"
    );
    open_menu(&mut harness, "media.asset.1");
    let (enabled, reason) = export_item(&harness);
    assert!(!enabled);
    assert_eq!(reason.as_deref(), Some("An export is already running"));
    harness.key_press(egui::Key::Escape);
    harness.run_steps(2);

    // Cancel, then let the held source go: the job stops and reports cancelled.
    click(&mut harness, "media.export.cancel");
    harness.run_steps(1);
    gate.release();
    let toast = await_clip_outcome(&mut harness);
    assert!(toast.starts_with("Clip export cancelled"), "{toast}");
    assert!(Arc::ptr_eq(&before, harness.state().session.project()));
}

/// The same held job released without a cancel reports the failure copy, which names no
/// per-clip count because the job snapshot carries no per-file results.
#[test]
fn clip_export_failure_reports_the_failure_copy() {
    let gate = Arc::new(ExportGate::default());
    let _release = ReleaseOnDrop(gate.clone());
    let dir = tempfile::tempdir().unwrap();
    let mut harness = build(gated_export_editor(gate.clone()));
    harness.run_steps(3);
    queue_selection_export(&mut harness);
    gate.arm();
    harness
        .state_mut()
        .export_clips_picked(dir.path().to_path_buf())
        .unwrap();
    gate.wait_entered(&mut harness);
    gate.release();
    let toast = await_clip_outcome(&mut harness);
    assert!(toast.starts_with("Clip export failed"), "{toast}");
    assert!(
        toast.contains("per-clip results are not reported"),
        "{toast}"
    );
}

/// Holds the export job's render source once armed. Playback's own workers are unarmed and
/// read the files directly, so only the export's source blocks.
#[derive(Default)]
struct ExportGate {
    armed: AtomicBool,
    /// (the export reached its source, the test released it)
    state: Mutex<(bool, bool)>,
    wake: Condvar,
}
impl ExportGate {
    fn arm(&self) {
        self.armed.store(true, Ordering::SeqCst);
    }
    /// Step the editor until the export holds its source. The clip-export runner starts a
    /// queued job only on the frame's tick, so the wait has to drive frames.
    fn wait_entered(&self, harness: &mut Harness<Editor>) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !self.lock().0 {
            assert!(Instant::now() < deadline, "export never reached its source");
            harness.run_steps(1);
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    fn release(&self) {
        self.lock().1 = true;
        self.wake.notify_all();
    }
    /// The state survives a panicking holder, so the release in Drop never panics twice.
    fn lock(&self) -> std::sync::MutexGuard<'_, (bool, bool)> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}
impl Sources for ExportGate {
    fn create(&self) -> Box<dyn render::SourceProvider> {
        if self.armed.load(Ordering::SeqCst) {
            let mut state = self.lock();
            state.0 = true;
            self.wake.notify_all();
            while !state.1 {
                state = self
                    .wake
                    .wait(state)
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
            }
        }
        Box::new(render::FileSources::default())
    }
}
// Release a held source before Playback's joining Drop, including when an assertion fails.
struct ReleaseOnDrop(Arc<ExportGate>);
impl Drop for ReleaseOnDrop {
    fn drop(&mut self) {
        self.0.release();
    }
}

/// Three video fixtures (ids 1..=3) and one audio-only asset (id 9), with the export's source
/// read through `gate`.
fn gated_export_editor(gate: Arc<ExportGate>) -> Editor {
    let mut editor = editor_with_sources(
        Preferences::default(),
        &[(1, "Bravo.mp4"), (2, "alpha.mp4"), (3, "charlie.mp4")],
        gate,
    );
    let audio = testkit::source::asset(
        9,
        std::env::temp_dir()
            .join("vibecut-media-library")
            .join("sound.wav"),
    )
    .format("wav")
    .content_hash(ContentHash::from_bytes(&[9; 8]))
    .duration(Tick(4 * SECOND))
    .stream(stream(StreamKind::Audio, Tick(4 * SECOND)))
    .build();
    editor
        .session
        .apply(Command::AddAsset { asset: audio })
        .unwrap();
    editor
}

/// Select the two video fixtures, open the menu on one, and take its export request.
fn queue_selection_export(harness: &mut Harness<Editor>) {
    click(harness, "media.asset.1");
    click_modifiers(harness, "media.asset.2", egui::Modifiers::COMMAND);
    open_menu(harness, "media.asset.2");
    let (enabled, _) = export_item(harness);
    assert!(enabled);
    click(harness, "media.export_clip");
    harness.run_steps(2);
    assert!(matches!(
        harness.state_mut().take_lifecycle_action(),
        Some(vibecut::services::lifecycle::EditorAction::ExportClips)
    ));
}

/// Step until the export's outcome toast shows and its strip is gone.
fn await_clip_outcome(harness: &mut Harness<Editor>) -> String {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        harness.run_steps(1);
        if !has_widget(harness, "media.export.cancel")
            && let Some(toast) = clip_toast(harness)
        {
            return toast;
        }
        assert!(
            Instant::now() < deadline,
            "export never reported an outcome"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn open_menu(harness: &mut Harness<Editor>, id: &str) {
    harness
        .get(By::new().predicate(|node| node.author_id() == Some(id)))
        .click_secondary();
    harness.run_steps(2);
}

/// Whether the export item is enabled, and the refusal reason it carries.
fn export_item(harness: &Harness<Editor>) -> (bool, Option<String>) {
    harness
        .state()
        .inspect()
        .unwrap()
        .widgets
        .into_iter()
        .find(|widget| widget.id == "media.export_clip")
        .map(|widget| (widget.enabled, widget.tooltip))
        .expect("export clip item")
}

/// An outcome toast of the clip export, not the refusal of its start.
fn clip_toast(harness: &Harness<Editor>) -> Option<String> {
    harness
        .state()
        .inspect()
        .unwrap()
        .import_toast
        .filter(|toast| {
            toast.starts_with("Clip export") && !toast.starts_with("Clip export did not start")
        })
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
#[ignore = "renders through wgpu; run with --ignored to refresh target/p1-09-media.png and target/p1-79-media-gaps.png"]
fn media_library_renders_a_representative_grid() {
    let mut harness = build(editor());
    harness.run_steps(3);
    click(&mut harness, "media.asset.2");
    let image = harness.render().expect("offscreen render");
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let target = root.join("target");
    std::fs::create_dir_all(&target).unwrap();
    image.save(target.join("p1-09-media.png")).unwrap();
    // The no-match state with its clear row, from the same fixture.
    click(&mut harness, "assets.search");
    harness
        .get(By::new().predicate(|node| node.author_id() == Some("assets.search")))
        .type_text("missing");
    harness.run_steps(3);
    let gaps = harness.render().expect("offscreen render");
    gaps.save(target.join("p1-79-media-gaps.png")).unwrap();
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

/// One timeline item as the three Media insert paths leave it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Placed {
    track: project::TrackId,
    item: project::ItemId,
    asset: Option<AssetId>,
    start: Tick,
}

fn placed(project: &Project) -> Vec<Placed> {
    project
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| {
            track.items.iter().map(move |item| Placed {
                track: track.id,
                item: item.id,
                asset: item.source_asset,
                start: item.start,
            })
        })
        .collect()
}

fn main_track(harness: &Harness<Editor>) -> project::TrackId {
    harness
        .state()
        .session
        .project()
        .data()
        .scene()
        .timeline
        .main
        .id
}

/// A point in the lanes 80 px right of a track header, on that track's row.
fn lane_point(harness: &Harness<Editor>, track: project::TrackId) -> egui::Pos2 {
    let header = widget_rect(harness, &format!("track.{}.header", track.0));
    egui::pos2(header.right() + 80.0, header.center().y)
}

/// Press on an asset card, drag it onto `target` and release there.
fn drag_asset_to(harness: &mut Harness<Editor>, asset: AssetId, target: egui::Pos2) {
    let card = widget_rect(harness, &format!("media.asset.{}", asset.0)).center();
    harness.hover_at(card);
    harness.drag_at(card);
    harness.run_steps(1);
    harness.hover_at(target);
    harness.run_steps(1);
    harness.drop_at(target);
    harness.run_steps(2);
}

/// Imports whatever path it is handed as a fresh video asset, as a real probe would.
struct FreshProbe;
impl ImportProbe for FreshProbe {
    fn import(
        &self,
        id: AssetId,
        path: PathBuf,
        _cancelled: Arc<AtomicBool>,
    ) -> media::Result<ImportedSource> {
        let name = path.file_name().map_or_else(
            || "pasted.mp4".to_owned(),
            |name| name.to_string_lossy().into_owned(),
        );
        Ok(imported(source_asset(id.0, &name, Some(Tick(4 * SECOND)))))
    }
}

fn await_imports(harness: &mut Harness<Editor>) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        harness.run_steps(1);
        let ready = harness
            .state()
            .inspect()
            .unwrap()
            .assets
            .iter()
            .all(|asset| asset.state == "ready");
        if !harness.state().importing() && ready {
            break;
        }
        assert!(Instant::now() < deadline, "import never finished");
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn plus_inserts_the_selected_asset_at_the_playhead_and_undo_redo_keep_it() {
    let mut harness = build(editor());
    harness.run_steps(3);
    click(&mut harness, "media.asset.2");
    click(&mut harness, "media.insert");
    let inserted = placed(harness.state().session.project());
    assert_eq!(inserted.len(), 1);
    assert_eq!(inserted[0].asset, Some(AssetId(2)));
    assert_eq!(inserted[0].track, main_track(&harness));
    assert_eq!(inserted[0].start, Tick::ZERO);

    harness.state_mut().activate("edit.undo").unwrap();
    harness.run_steps(2);
    assert!(placed(harness.state().session.project()).is_empty());
    harness.state_mut().activate("edit.redo").unwrap();
    harness.run_steps(2);
    assert_eq!(placed(harness.state().session.project()), inserted);
}

#[test]
fn dragging_an_asset_onto_a_track_inserts_it_there_and_undo_redo_restore_it_exactly() {
    let mut harness = build(editor());
    harness.run_steps(3);
    let main = main_track(&harness);
    let target = lane_point(&harness, main);
    drag_asset_to(&mut harness, AssetId(2), target);
    let inserted = placed(harness.state().session.project());
    assert_eq!(inserted.len(), 1);
    assert_eq!(
        (inserted[0].track, inserted[0].asset),
        (main, Some(AssetId(2)))
    );

    harness.state_mut().activate("edit.undo").unwrap();
    harness.run_steps(2);
    assert!(placed(harness.state().session.project()).is_empty());
    harness.state_mut().activate("edit.redo").unwrap();
    harness.run_steps(2);
    assert_eq!(placed(harness.state().session.project()), inserted);
}

#[test]
fn dropping_an_asset_on_empty_space_below_the_tracks_creates_a_compatible_track() {
    let mut harness = build(editor());
    harness.run_steps(3);
    let main = main_track(&harness);
    let tracks_before = harness
        .state()
        .session
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .count();
    let header = widget_rect(&harness, &format!("track.{}.header", main.0));
    let empty = egui::pos2(header.right() + 80.0, header.bottom() + 120.0);
    drag_asset_to(&mut harness, AssetId(3), empty);

    let project = harness.state().session.project();
    assert_eq!(
        project.data().scene().timeline.iter().count(),
        tracks_before + 1
    );
    let created = placed(project);
    assert_eq!(created.len(), 1);
    assert_ne!(created[0].track, main);
    assert_eq!(created[0].asset, Some(AssetId(3)));
}

#[test]
fn ctrl_v_with_files_on_the_os_clipboard_imports_them_and_inserts_at_the_playhead() {
    let mut editor = editor();
    editor.set_import_probe(Arc::new(FreshProbe)).unwrap();
    editor.set_os_clipboard_files(|| vec![PathBuf::from("pasted-clip.mp4")]);
    let mut harness = build(editor);
    harness.run_steps(3);
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::V);
    await_imports(&mut harness);

    let pasted = placed(harness.state().session.project());
    assert_eq!(pasted.len(), 1);
    assert_eq!(pasted[0].track, main_track(&harness));
    assert_eq!(pasted[0].start, Tick::ZERO);
    assert_eq!(harness.state().session.project().data().assets.len(), 4);

    harness.state_mut().activate("edit.undo").unwrap();
    harness.run_steps(2);
    assert!(placed(harness.state().session.project()).is_empty());
}

#[test]
fn ctrl_v_without_files_on_the_os_clipboard_leaves_the_media_library_alone() {
    let mut harness = build(editor());
    harness.run_steps(3);
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::V);
    harness.run_steps(3);
    assert_eq!(harness.state().session.project().data().assets.len(), 3);
    assert!(!harness.state().importing());
}

#[test]
fn ctrl_v_with_files_while_a_text_field_is_focused_pastes_text_and_imports_nothing() {
    let mut editor = editor();
    editor.set_import_probe(Arc::new(FreshProbe)).unwrap();
    editor.set_os_clipboard_files(|| vec![PathBuf::from("focused-clip.mp4")]);
    let mut harness = build(editor);
    harness.run_steps(3);
    click(&mut harness, "assets.search");
    harness
        .get(By::new().predicate(|node| node.author_id() == Some("assets.search")))
        .type_text("alpha");
    harness.run_steps(3);
    assert!(has_widget(&harness, "media.asset.2"));
    assert!(!has_widget(&harness, "media.asset.1"));

    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::V);
    harness.run_steps(3);
    assert!(!harness.state().importing());
    assert_eq!(harness.state().session.project().data().assets.len(), 3);
    assert!(placed(harness.state().session.project()).is_empty());

    // The paste reached the focused field: "alphazz" matches no asset.
    harness.event(egui::Event::Paste("zz".into()));
    harness.run_steps(3);
    for id in 1..=3 {
        assert!(
            !has_widget(&harness, &format!("media.asset.{id}")),
            "asset {id}"
        );
    }
}

#[test]
fn inserted_media_round_trips_through_save_and_load_after_insert_and_after_undo() {
    let mut harness = build(editor());
    harness.run_steps(3);
    let target = lane_point(&harness, main_track(&harness));
    drag_asset_to(&mut harness, AssetId(1), target);
    let inserted = placed(harness.state().session.project());
    assert_eq!(inserted.len(), 1);

    let bytes = project::encode(harness.state().session.project(), false).unwrap();
    let loaded = project::decode(&bytes).unwrap();
    assert_eq!(placed(&loaded.project), inserted);

    harness.state_mut().activate("edit.undo").unwrap();
    harness.run_steps(2);
    let bytes = project::encode(harness.state().session.project(), false).unwrap();
    let loaded = project::decode(&bytes).unwrap();
    assert!(placed(&loaded.project).is_empty());
}

#[test]
fn media_drag_and_plus_screenshot_for_the_combined_build() {
    let mut harness = build(editor());
    harness.run_steps(3);
    let target = lane_point(&harness, main_track(&harness));
    drag_asset_to(&mut harness, AssetId(2), target);
    click(&mut harness, "media.asset.3");
    click(&mut harness, "media.insert");
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/p1-11-dnd.png");
    harness.render().unwrap().save(&path).unwrap();
}

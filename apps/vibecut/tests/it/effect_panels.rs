#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::expect_used
)]
use egui_kittest::{
    Harness,
    kittest::{By, NodeT, Queryable},
};
use engine::{ByteBudget, Command, CreatedId, NewItem, Placement, Session};
use media::{AssetId, ContentHash, SourceAsset, SourceStream};
use project::{Color, EffectValue, ItemId, ItemParams, Project, ProjectId, TrackId, TrackKind};
use render::effects::EffectGeometry;
use std::{path::PathBuf, sync::Arc};
use time::{SourceRange, TICKS_PER_SECOND, Tick};
use vibecut::{
    Editor, EffectPreviewStatus, SETTLE_BUDGET, egui,
    preferences::{PreferenceStore, Preferences},
    services::playback::{Files, Playback},
};

const SECOND: i64 = TICKS_PER_SECOND;

#[derive(Default)]
struct PreviewClock(std::sync::atomic::AtomicU64);
impl PreviewClock {
    fn set(&self, ms: u64) {
        self.0.store(ms, std::sync::atomic::Ordering::SeqCst);
    }
}
impl engine::jobs::Clock for PreviewClock {
    fn now(&self) -> std::time::Duration {
        std::time::Duration::from_millis(self.0.load(std::sync::atomic::Ordering::SeqCst))
    }
}

/// Reads the real painter output, including tile names drawn without widgets.
fn paints_text(h: &Harness<Editor>, expected: &str, region: egui::Rect) -> bool {
    let mut pending: Vec<_> = h.output().shapes.iter().map(|s| &s.shape).collect();
    while let Some(shape) = pending.pop() {
        match shape {
            egui::Shape::Vec(children) => pending.extend(children),
            egui::Shape::Text(text)
                if text.galley.text() == expected && region.contains(text.pos) =>
            {
                return true;
            }
            _ => {}
        }
    }
    false
}

#[test]
fn bilingual_effect_tiles_search_headers_parameters_and_add_feedback_follow_locale() {
    use vibecut::i18n::Locale;
    type Row<'a> = (&'a str, &'a str, &'a str, &'a [(&'a str, &'a str, &'a str)]);
    let rows: &[Row] = &[
        (
            "directional_blur",
            "Streak Blur",
            "Mờ theo hướng",
            &[
                ("direction", "Direction", "Hướng"),
                ("length", "Length", "Độ dài"),
            ],
        ),
        (
            "find_edges",
            "Edge Lines",
            "Đường viền",
            &[("blend", "Blend with original", "Trộn với gốc")],
        ),
        (
            "color_emboss",
            "Colour Relief",
            "Phù điêu màu",
            &[
                ("direction", "Direction", "Hướng"),
                ("relief", "Relief", "Độ nổi"),
                ("contrast", "Contrast", "Độ tương phản"),
                ("blend", "Blend with original", "Trộn với gốc"),
            ],
        ),
        ("black_white", "Black & white", "Đen trắng", &[]),
        (
            "channel_mix",
            "Channel mixer",
            "Trộn kênh",
            &[
                ("rr", "Red from red", "Đỏ từ đỏ"),
                ("rg", "Red from green", "Đỏ từ xanh lá"),
                ("rb", "Red from blue", "Đỏ từ xanh dương"),
                ("rc", "Red constant", "Hằng số đỏ"),
                ("gr", "Green from red", "Xanh lá từ đỏ"),
                ("gg", "Green from green", "Xanh lá từ xanh lá"),
                ("gb", "Green from blue", "Xanh lá từ xanh dương"),
                ("gc", "Green constant", "Hằng số xanh lá"),
                ("br", "Blue from red", "Xanh dương từ đỏ"),
                ("bg", "Blue from green", "Xanh dương từ xanh lá"),
                ("bb", "Blue from blue", "Xanh dương từ xanh dương"),
                ("bc", "Blue constant", "Hằng số xanh dương"),
            ],
        ),
        (
            "color_pass",
            "Keep colour",
            "Giữ màu",
            &[("similarity", "Similarity", "Độ tương đồng")],
        ),
        (
            "color_replace",
            "Swap colour",
            "Thay màu",
            &[("similarity", "Similarity", "Độ tương đồng")],
        ),
        (
            "extract",
            "Threshold",
            "Ngưỡng",
            &[
                ("black", "Black point", "Điểm đen"),
                ("white", "White point", "Điểm trắng"),
                ("softness", "Softness", "Độ mềm"),
            ],
        ),
        (
            "gamma_correction",
            "Gamma",
            "Gamma",
            &[("gamma", "Gamma", "Gamma")],
        ),
        (
            "invert",
            "Invert",
            "Đảo màu",
            &[("blend", "Blend with original", "Trộn với gốc")],
        ),
        (
            "levels",
            "Levels",
            "Mức màu",
            &[
                ("in_black", "Input black", "Điểm đen gốc"),
                ("in_white", "Input white", "Điểm trắng gốc"),
                ("out_black", "Output black", "Điểm đen sau chỉnh"),
                ("out_white", "Output white", "Điểm trắng sau chỉnh"),
                ("gamma", "Gamma", "Gamma"),
            ],
        ),
        (
            "posterize",
            "Posterise",
            "Giảm bậc màu",
            &[("levels", "Levels", "Mức màu")],
        ),
        (
            "strobe",
            "Strobe",
            "Chớp sáng",
            &[
                ("blend", "Blend with original", "Trộn với gốc"),
                ("duration", "Duration", "Thời lượng"),
                ("period", "Period", "Chu kỳ"),
            ],
        ),
        (
            "tint",
            "Two-tone",
            "Hai tông màu",
            &[("amount", "Amount", "Cường độ")],
        ),
    ];
    for &(kind, en, vi, parameters) in rows {
        let f = fixture();
        let video = f.video;
        // Show the complete twelve-row mixer without substituting a test view.
        let mut h = crate::gpu::builder()
            .with_size(egui::vec2(1727.0, 1800.0))
            .build_ui_state(|ui, editor: &mut Editor| editor.ui(ui), f.editor);
        h.run_steps(3);
        open_effect_tabs(&mut h, video);
        let original = h.state().session.project().clone();
        let history = h.state().session.history_len();
        for (locale, name) in [(Locale::En, en), (Locale::Vi, vi)] {
            let before_locale_history = h.state().session.history_len();
            h.state_mut().preferences.locale = locale;
            h.run_steps(3);
            if locale == Locale::Vi && en != vi {
                assert!(
                    !has_widget(&h, &format!("effects.tile.{kind}")),
                    "stale search {kind}"
                );
            }
            type_value(&mut h, "assets.search", name);
            let tile = widget_rect(&h, &format!("effects.tile.{kind}"));
            assert!(paints_text(&h, name, tile), "tile {kind} {locale:?}");
            for &(other, _, _, _) in rows {
                if other != kind {
                    assert!(
                        !has_widget(&h, &format!("effects.tile.{other}")),
                        "search {kind} included {other}"
                    );
                }
            }
            assert_eq!(h.state().session.project(), &original);
            assert_eq!(h.state().session.history_len(), before_locale_history);
            click(&mut h, &format!("effects.add.{kind}"));
            let added = h.state().session.project().clone();
            let added_history = h.state().session.history_len();
            assert_eq!(added_history.0, history.0 + 1, "one add {kind}");
            assert_eq!(effects(&h, video)[0].kind, kind);
            let toast = h.state().inspect().unwrap().import_toast.unwrap();
            assert!(toast.contains(name), "toast {toast:?} missing {name}");
            assert!(toast.contains(if locale == Locale::En {
                "Effect added"
            } else {
                "Đã thêm hiệu ứng"
            }));
            // Locale switches must preserve every static/animated parameter and history.
            for (display, title) in [(Locale::En, en), (Locale::Vi, vi)] {
                h.state_mut().preferences.locale = display;
                h.run_steps(3);
                let left = widget_rect(&h, &format!("effects.row.{kind}.handle")).left();
                let region =
                    egui::Rect::from_min_max(egui::pos2(left, 0.0), egui::pos2(1727.0, 1800.0));
                assert!(
                    paints_text(&h, title, region),
                    "Properties header {kind} {display:?}"
                );
                let definition = project::effect_definition(kind).unwrap();
                let numeric: Vec<_> = definition
                    .params
                    .iter()
                    .filter(|p| {
                        matches!(
                            p.kind,
                            project::EffectParamKind::Float { .. }
                                | project::EffectParamKind::Angle
                        )
                    })
                    .map(|p| p.id)
                    .collect();
                assert_eq!(
                    numeric,
                    parameters.iter().map(|p| p.0).collect::<Vec<_>>(),
                    "complete numeric census {kind}"
                );
                for &(key, english, vietnamese) in parameters {
                    let expected = if display == Locale::En {
                        english
                    } else {
                        vietnamese
                    };
                    let id = format!("effects.{kind}.param.{key}");
                    h.get(
                        By::new()
                            .label(expected)
                            .predicate(|node| node.author_id() == Some(id.as_str())),
                    );
                    assert!(
                        paints_text(&h, expected, region),
                        "painted numeric {kind}.{key} {display:?}"
                    );
                }
                assert_eq!(h.state().session.project(), &added);
                assert_eq!(h.state().session.history_len(), added_history);
            }
            h.state_mut().activate("edit.undo").unwrap();
            h.run_steps(3);
            assert_eq!(h.state().session.project(), &original, "undo {kind}");
            // A fresh add clears the redo entry; compare the undo count independently.
            assert_eq!(h.state().session.history_len().0, history.0);
        }
    }
}

struct Fixture {
    editor: Editor,
    video: ItemId,
    image: ItemId,
    graphic: ItemId,
    audio: ItemId,
    main: TrackId,
}

fn stream(kind: media::StreamKind, frame_count: Option<u64>) -> SourceStream {
    let mut builder = testkit::source::stream(kind).duration(Tick(10 * SECOND));
    if let Some(frame_count) = frame_count {
        builder = builder.frame_count(frame_count);
    }
    builder.build()
}

fn asset(id: u64, mut streams: Vec<SourceStream>) -> SourceAsset {
    for (index, stream) in streams.iter_mut().enumerate() {
        stream.index = index;
    }
    testkit::source::asset(
        id,
        std::env::temp_dir().join(format!("vibecut-effects-{id}.mp4")),
    )
    .format("mov,mp4")
    .duration(Tick(10 * SECOND))
    .content_hash(ContentHash::from_bytes(&[id as u8; 8]))
    .streams(streams)
    .build()
}

fn add_track(session: &mut Session, name: &str, kind: TrackKind) -> TrackId {
    let outcome = session
        .apply(Command::AddTrack {
            placement: engine::TrackPlacement::Auto,
            name: name.into(),
            kind,
        })
        .unwrap();
    match outcome.created.first().copied() {
        Some(CreatedId::Track(id)) => id,
        _ => panic!("track"),
    }
}

fn insert(
    session: &mut Session,
    track: TrackId,
    start: Tick,
    duration: Tick,
    asset: Option<AssetId>,
    content: ItemParams,
) -> ItemId {
    let outcome = session
        .apply(Command::InsertItem {
            placement: Placement::Track { track },
            item: NewItem {
                retime: Default::default(),
                animations: Default::default(),
                source_asset: asset,
                source_range: SourceRange::new(Tick::ZERO, duration).unwrap(),
                start,
                content,
            },
        })
        .unwrap();
    match outcome.created.first().copied() {
        Some(CreatedId::Item(id)) => id,
        _ => panic!("item"),
    }
}

fn add_effect(session: &mut Session, item: ItemId, id: &str, enabled: bool) {
    session
        .apply(Command::AddEffect {
            item,
            effect: {
                let mut effect = project::effect_definition("blur").unwrap().instance(id);
                effect.enabled = enabled;
                effect
            },
        })
        .unwrap();
}

fn fixture() -> Fixture {
    let mut session =
        Session::new(Project::empty(ProjectId(9), "Effects".into()).unwrap(), 200).unwrap();
    for (id, streams) in [
        (
            1,
            vec![
                stream(media::StreamKind::Video, None),
                stream(media::StreamKind::Audio, None),
            ],
        ),
        (2, vec![stream(media::StreamKind::Audio, None)]),
        (3, vec![stream(media::StreamKind::Video, Some(1))]),
    ] {
        session
            .apply(Command::AddAsset {
                asset: asset(id, streams),
            })
            .unwrap();
    }
    let main = session.project().data().scene().timeline.main.id;
    let audio_track = add_track(&mut session, "Audio", TrackKind::Audio);
    let video = insert(
        &mut session,
        main,
        Tick::ZERO,
        Tick(2 * SECOND),
        Some(AssetId(1)),
        ItemParams::Video(project::VideoParams {
            source_audio_enabled: true,
            ..project::VideoParams::plain()
        }),
    );
    let image = insert(
        &mut session,
        main,
        Tick(2 * SECOND),
        Tick(SECOND),
        Some(AssetId(3)),
        ItemParams::Video(project::VideoParams::plain()),
    );
    let graphic = insert(
        &mut session,
        main,
        Tick(3 * SECOND),
        Tick(SECOND),
        None,
        ItemParams::Overlay(project::OverlayParams {
            color: Color([10, 20, 30, 255]),
        }),
    );
    let audio = insert(
        &mut session,
        audio_track,
        Tick::ZERO,
        Tick(2 * SECOND),
        Some(AssetId(2)),
        ItemParams::Audio(project::AudioParams::default()),
    );
    let mut playback = Playback::new(
        &session,
        ByteBudget::new(64 << 20).unwrap(),
        Arc::new(Files),
    )
    .unwrap();
    playback.set_device_available(true);
    let mut editor = Editor::new(session, playback, Preferences::default());
    editor.set_clock(Arc::new(PreviewClock::default()));
    Fixture {
        editor,
        video,
        image,
        graphic,
        audio,
        main,
    }
}

fn build(editor: Editor) -> Harness<'static, Editor> {
    crate::gpu::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .build_ui_state(|ui, editor: &mut Editor| editor.ui(ui), editor)
}

fn select(harness: &mut Harness<Editor>, item: ItemId) {
    harness
        .state_mut()
        .select([item].into_iter().collect())
        .unwrap();
    harness.run_steps(3);
}

fn click(harness: &mut Harness<Editor>, id: &str) {
    harness
        .get(By::new().predicate(|node| node.author_id() == Some(id)))
        .click();
    harness.run_steps(2);
}

/// The AccessKit node id egui assigned to the widget behind one automation id.
fn node_id(harness: &Harness<Editor>, id: &str) -> String {
    format!(
        "{:?}",
        harness
            .get(By::new().predicate(|node| node.author_id() == Some(id)))
            .accesskit_node()
            .id()
    )
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

fn tile_texture(h: &Harness<Editor>, kind: &str) -> egui::TextureId {
    let center = widget_rect(h, &format!("effects.tile.{kind}")).center();
    let mut pending: Vec<_> = h.output().shapes.iter().map(|s| &s.shape).collect();
    while let Some(shape) = pending.pop() {
        match shape {
            egui::Shape::Vec(children) => pending.extend(children),
            egui::Shape::Mesh(mesh) if mesh.calc_bounds().contains(center) => {
                return mesh.texture_id;
            }
            egui::Shape::Rect(rect) if rect.brush.is_some() && rect.rect.contains(center) => {
                return rect.fill_texture_id();
            }
            _ => {}
        }
    }
    panic!("missing real tile texture {kind}")
}
type Uploads =
    Arc<std::sync::Mutex<std::collections::BTreeMap<egui::TextureId, Arc<egui::ColorImage>>>>;
struct PreviewUploads {
    inner: egui_kittest::LazyRenderer,
    seen: Uploads,
}
impl egui_kittest::TestRenderer for PreviewUploads {
    fn handle_delta(&mut self, delta: &mut egui::TexturesDelta) {
        let mut seen = self.seen.lock().unwrap();
        seen.clear();
        for (id, changes) in &delta.set {
            if let Some(change) = changes.last() {
                let egui::ImageData::Color(image) = &change.image;
                seen.insert(*id, image.clone());
            }
        }
        self.inner.handle_delta(delta);
    }
    fn render(
        &mut self,
        ctx: &egui::Context,
        output: &egui::FullOutput,
    ) -> Result<image::RgbaImage, String> {
        self.inner.render(ctx, output)
    }
}
fn uploaded(seen: &Uploads, id: egui::TextureId) -> Option<Arc<egui::ColorImage>> {
    seen.lock().unwrap().get(&id).cloned()
}

#[test]
fn live_effect_clock_updates_real_textures_pauses_hidden_and_wraps_without_project_edits() {
    let f = fixture();
    let clock = Arc::new(PreviewClock::default());
    let mut editor = f.editor;
    editor.set_clock(clock.clone());
    let adapter = Arc::new(PreviewGate::new(true, None));
    let uploads: Uploads = Default::default();
    let mut h = crate::gpu::builder()
        .renderer(PreviewUploads {
            inner: Default::default(),
            seen: uploads.clone(),
        })
        .with_size(egui::vec2(1440.0, 900.0))
        .build_ui_state(|ui, editor: &mut Editor| editor.ui(ui), editor);
    inject_previews(&mut h, adapter.clone());
    h.run_steps(3);
    click(&mut h, "assets.tab.effects");
    let original = h.state().session.project().clone();
    let history = h.state().session.history_len();
    let playhead = h.state().inspect().unwrap().position;
    let mut initial = None;
    let mut texture = None;
    for ms in [0, 100, 1900, 2000] {
        clock.set(ms);
        h.run_steps(1);
        h.state_mut().settle_previews(SETTLE_BUDGET).unwrap();
        h.run_steps(1);
        assert_eq!(h.state().inspect().unwrap().effect_previews.len(), 43);
        assert!(
            h.state()
                .inspect()
                .unwrap()
                .effect_previews
                .values()
                .all(|s| *s == EffectPreviewStatus::Ready)
        );
        let id = tile_texture(&h, "strobe");
        if let Some(before) = texture {
            assert_eq!(id, before, "update existing texture handle");
        }
        texture = Some(id);
        let image = uploaded(&uploads, id).expect("new phase uploaded");
        if ms == 0 {
            initial = Some(image.clone());
        }
        if ms == 100 || ms == 1900 {
            assert_ne!(image, initial.clone().unwrap());
        }
        if ms == 2000 {
            assert_eq!(
                image,
                initial.clone().unwrap(),
                "loop wrap must present the newer sequence at local zero"
            );
        }
        let calls = adapter.state.lock().unwrap().1;
        h.run_steps(3);
        assert!(
            uploaded(&uploads, id).is_none(),
            "stationary clock must not reload texture"
        );
        assert_eq!(
            adapter.state.lock().unwrap().1,
            calls,
            "stationary time does not resubmit"
        );
    }
    click(&mut h, "assets.tab.media");
    let calls = adapter.state.lock().unwrap().1;
    clock.set(12300);
    h.run_steps(4);
    assert_eq!(
        adapter.state.lock().unwrap().1,
        calls,
        "hidden panel requests no work"
    );
    click(&mut h, "assets.tab.effects");
    h.state_mut().settle_previews(SETTLE_BUDGET).unwrap();
    h.run_steps(1);
    assert_eq!(
        uploaded(&uploads, texture.unwrap()).unwrap(),
        initial.unwrap(),
        "resume excludes 10.3 seconds hidden time"
    );
    adapter.state.lock().unwrap().0 = false;
    clock.set(12400);
    h.run_steps(2);
    let (state, timeout) = adapter
        .changed
        .wait_timeout_while(adapter.state.lock().unwrap(), SETTLE_BUDGET, |s| s.2 == 0)
        .unwrap();
    assert!(!timeout.timed_out());
    drop(state);
    h.state_mut()
        .set_effects(Arc::new(render::effects::CpuEffects))
        .unwrap();
    assert_eq!(
        adapter.cancelled.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "replacing source/cache owner cancels and joins the old generation"
    );
    h.run_steps(1);
    h.state_mut().settle_previews(SETTLE_BUDGET).unwrap();
    h.run_steps(1);
    assert!(
        h.state()
            .inspect()
            .unwrap()
            .effect_previews
            .values()
            .all(|s| *s == EffectPreviewStatus::Ready)
    );
    assert_eq!(h.state().session.project(), &original);
    assert_eq!(h.state().session.history_len(), history);
    assert_eq!(h.state().inspect().unwrap().position, playhead);
}

#[test]
fn live_preview_coalescing_keeps_ready_frames_fair_work_and_generation_custody_bounded() {
    use vibecut::EffectPreviews;
    let budget = ByteBudget::new(16 << 20).unwrap();
    let gate = Arc::new(PreviewGate::new(true, None));
    let cache = engine::effect_thumbnails::EffectThumbnails::new(gate.clone(), budget.clone());
    let mut previews = EffectPreviews::new(cache, budget.clone()).unwrap();
    previews.update(true, std::time::Duration::ZERO);
    for d in project::effect_definitions()
        .iter()
        .filter(|d| d.shelf.in_library())
    {
        previews.ensure(d.id, &d.instance(d.id), &project::Animations::default());
    }
    previews.settle(SETTLE_BUDGET).unwrap();
    let initial_calls = gate.state.lock().unwrap().1;
    gate.state.lock().unwrap().0 = false;
    previews.update(true, std::time::Duration::from_millis(100));
    // Identity kinds emit no pass and never reach the adapter, so pump until the
    // one admitted worker is actually blocked inside it.
    let deadline = std::time::Instant::now() + SETTLE_BUDGET;
    while gate.state.lock().unwrap().2 == 0 {
        previews.tick().unwrap();
        assert!(
            std::time::Instant::now() < deadline,
            "no worker reached the blocked adapter"
        );
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    let mut peak = 0;
    for ms in (101..=2200).step_by(33) {
        previews.update(true, std::time::Duration::from_millis(ms));
        previews.tick().unwrap();
        peak = peak.max(budget.snapshot().unwrap().used);
        assert!(
            previews
                .statuses()
                .values()
                .all(|s| *s == EffectPreviewStatus::Ready)
        );
    }
    assert_eq!(
        gate.state.lock().unwrap().1,
        initial_calls + 1,
        "one blocked worker; time does not cancel it"
    );
    assert_eq!(gate.cancelled.load(std::sync::atomic::Ordering::SeqCst), 0);
    assert!(
        previews
            .settle(std::time::Duration::from_millis(20))
            .is_err(),
        "Ready refresh still belongs to settle target"
    );
    gate.release();
    previews.settle(SETTLE_BUDGET).unwrap();
    assert!(
        previews
            .statuses()
            .values()
            .all(|s| *s == EffectPreviewStatus::Ready)
    );
    let completed_calls = gate.state.lock().unwrap().1;
    // The one blocked worker plus at most one coalesced latest frame per kind;
    // the clock updates above never queue a job each.
    assert!(
        completed_calls <= initial_calls + previews.statuses().len() + 1,
        "coalesced latest frame, not one queued job per clock update"
    );
    eprintln!(
        "live-pressure initial_calls={initial_calls} completed_calls={completed_calls} peak_used={peak} limit={}",
        budget.snapshot().unwrap().limit
    );
    drop(previews);
    assert_eq!(budget.snapshot().unwrap().used, 0);

    // Changed animation input invalidates a running generation without an explicit Retry.
    let gate = Arc::new(PreviewGate::new(false, None));
    let cache = engine::effect_thumbnails::EffectThumbnails::new(gate.clone(), budget.clone());
    let mut previews = EffectPreviews::new(cache, budget.clone()).unwrap();
    let effect = project::effect_definition("blur").unwrap().instance("blur");
    previews.ensure("blur", &effect, &project::Animations::default());
    previews.tick().unwrap();
    gate.wait_started();
    let mut animation = project::Animations::default();
    animation
        .set(
            effect.param_path("intensity").unwrap(),
            project::Param::new(project::ParamValue::Scalar(0.0)).unwrap(),
        )
        .unwrap();
    previews.ensure("blur", &effect, &animation);
    gate.release();
    previews.settle(SETTLE_BUDGET).unwrap();
    assert_eq!(gate.cancelled.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(previews.status("blur"), Some(EffectPreviewStatus::Ready));
    let mut expected = EffectPreviews::cpu().unwrap();
    expected.ensure(
        "blur",
        &effect
            .clone()
            .with("intensity", EffectValue::Scalar(0.0))
            .unwrap(),
        &project::Animations::default(),
    );
    expected.settle(SETTLE_BUDGET).unwrap();
    assert_eq!(
        previews.image("blur").unwrap(),
        expected.image("blur").unwrap(),
        "changed animation cannot display the old blur"
    );
    drop(previews);
    assert_eq!(budget.snapshot().unwrap().used, 0);
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

fn item(harness: &Harness<Editor>, id: ItemId) -> project::Item {
    harness
        .state()
        .session
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| &track.items)
        .find(|item| item.id == id)
        .cloned()
        .unwrap()
}

fn effects(harness: &Harness<Editor>, id: ItemId) -> Vec<project::EffectInstance> {
    item(harness, id).effects
}

fn effect_ids(harness: &Harness<Editor>, id: ItemId) -> Vec<String> {
    effects(harness, id)
        .into_iter()
        .map(|effect| effect.id)
        .collect()
}

fn drag_to(harness: &mut Harness<Editor>, id: &str, delta: egui::Vec2) {
    let start = widget_rect(harness, id).center();
    harness.hover_at(start);
    harness.run_steps(1);
    harness.drag_at(start);
    harness.run_steps(1);
    harness.hover_at(start + delta);
    harness.run_steps(2);
}

fn open_effect_tabs(harness: &mut Harness<Editor>, item: ItemId) {
    select(harness, item);
    click(harness, "assets.tab.effects");
    click(harness, "inspector.tab.effects");
}

fn inject_previews(
    harness: &mut Harness<Editor>,
    adapter: Arc<dyn render::effects::EffectCompositor>,
) {
    harness.state_mut().set_effects(adapter).unwrap();
}

fn wait_preview(
    harness: &mut Harness<Editor>,
    kind: &str,
    wanted: EffectPreviewStatus,
) -> EffectPreviewStatus {
    for _ in 0..400 {
        harness.run_steps(1);
        let Some(status) = harness
            .state()
            .inspect()
            .unwrap()
            .effect_previews
            .get(kind)
            .copied()
        else {
            std::thread::sleep(std::time::Duration::from_millis(2));
            continue;
        };
        if status == wanted {
            return status;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    panic!("preview {kind} did not become {wanted:?}");
}

#[test]
fn effects_tab_is_available_for_visual_clip_kinds_only() {
    let fixture = fixture();
    let mut harness = build(fixture.editor);
    harness.run_steps(3);
    for item in [fixture.video, fixture.image, fixture.graphic] {
        select(&mut harness, item);
        assert!(has_widget(&harness, "inspector.tab.effects"), "{item:?}");
        click(&mut harness, "inspector.tab.effects");
        assert!(has_widget(&harness, "effects.stack.empty"), "{item:?}");
    }
    select(&mut harness, fixture.audio);
    assert!(!has_widget(&harness, "inspector.tab.effects"));
}

#[test]
fn effects_library_lists_only_library_placements() {
    let fixture = fixture();
    let mut harness = build(fixture.editor);
    harness.run_steps(3);
    click(&mut harness, "assets.tab.effects");
    // The library offers library-placement effects only; grade and key live in
    // clip inspector panels, not the browser.
    assert!(has_widget(&harness, "effects.tile.blur"));
    assert!(has_widget(&harness, "effects.add.blur"));
    for kind in ["color_grade", "chroma_key"] {
        assert!(
            !has_widget(&harness, &format!("effects.tile.{kind}")),
            "{kind}"
        );
        assert!(
            !has_widget(&harness, &format!("effects.add.{kind}")),
            "{kind}"
        );
    }
}

#[test]
fn effect_tile_drop_adds_to_a_clip_and_refuses_audio_with_a_reason() {
    let fixture = fixture();
    let video = fixture.video;
    let audio = fixture.audio;
    let mut harness = build(fixture.editor);
    harness.run_steps(3);
    select(&mut harness, video);
    click(&mut harness, "assets.tab.effects");
    let history = harness.state().session.history_len();
    let start = widget_rect(&harness, "effects.tile.blur").center();
    let target = widget_rect(&harness, &format!("timeline.item.{}", video.0));
    drag_to(&mut harness, "effects.tile.blur", target.center() - start);
    harness.drop_at(target.center());
    harness.run_steps(3);
    assert_eq!(effect_ids(&harness, video), vec!["blur".to_owned()]);
    assert_eq!(harness.state().session.history_len().0, history.0 + 1);
    assert_eq!(*harness.state().selection(), [video].into_iter().collect());

    harness.state_mut().activate("edit.undo").unwrap();
    harness.run_steps(2);
    assert!(effects(&harness, video).is_empty());

    let history = harness.state().session.history_len();
    let start = widget_rect(&harness, "effects.tile.blur").center();
    let target = widget_rect(&harness, &format!("timeline.item.{}", audio.0));
    drag_to(&mut harness, "effects.tile.blur", target.center() - start);
    harness.drop_at(target.center());
    harness.run_steps(3);
    assert!(effects(&harness, audio).is_empty());
    assert_eq!(harness.state().session.history_len(), history);
    let toast = harness
        .state()
        .inspect()
        .unwrap()
        .import_toast
        .unwrap_or_default();
    assert!(toast.contains("Effects don't apply to audio"), "{toast}");
}

#[test]
fn effect_drag_escape_and_missed_drop_leave_the_project_untouched() {
    let fixture = fixture();
    let video = fixture.video;
    let mut harness = build(fixture.editor);
    harness.run_steps(3);
    select(&mut harness, video);
    click(&mut harness, "assets.tab.effects");
    let history = harness.state().session.history_len();
    let start = widget_rect(&harness, "effects.tile.blur").center();
    let target = widget_rect(&harness, &format!("timeline.item.{}", video.0));
    drag_to(&mut harness, "effects.tile.blur", target.center() - start);
    harness.key_press(egui::Key::Escape);
    harness.run_steps(2);
    harness.drop_at(target.center());
    harness.run_steps(3);
    assert!(effects(&harness, video).is_empty());
    assert_eq!(harness.state().session.history_len(), history);

    let start = widget_rect(&harness, "effects.tile.blur").center();
    let miss = egui::pos2(700.0, 540.0);
    drag_to(&mut harness, "effects.tile.blur", miss - start);
    harness.drop_at(miss);
    harness.run_steps(3);
    assert!(effects(&harness, video).is_empty());
    assert_eq!(harness.state().session.history_len(), history);
    let toast = harness
        .state()
        .inspect()
        .unwrap()
        .import_toast
        .unwrap_or_default();
    assert!(toast.contains("Drop on a video or image clip"), "{toast}");
}

#[test]
fn effect_plus_adds_to_the_selected_clip_and_needs_a_target() {
    let fixture = fixture();
    let video = fixture.video;
    let image = fixture.image;
    let mut harness = build(fixture.editor);
    harness.run_steps(3);
    click(&mut harness, "assets.tab.effects");
    let history = harness.state().session.history_len();
    click(&mut harness, "effects.add.blur");
    assert!(effects(&harness, video).is_empty());
    assert_eq!(harness.state().session.history_len(), history);
    let toast = harness
        .state()
        .inspect()
        .unwrap()
        .import_toast
        .unwrap_or_default();
    assert!(toast.contains("Select a clip first"), "{toast}");

    harness
        .state_mut()
        .select([video, image].into_iter().collect())
        .unwrap();
    harness.run_steps(3);
    click(&mut harness, "effects.add.blur");
    assert!(effects(&harness, video).is_empty());
    assert!(effects(&harness, image).is_empty());
    assert_eq!(harness.state().session.history_len(), history);

    select(&mut harness, image);
    let history = harness.state().session.history_len();
    click(&mut harness, "effects.add.blur");
    assert_eq!(effect_ids(&harness, image), vec!["blur".to_owned()]);
    assert_eq!(harness.state().session.history_len().0, history.0 + 1);
    assert_eq!(*harness.state().selection(), [image].into_iter().collect());
}

#[test]
fn effect_headers_reorder_toggle_and_remove_through_commands() {
    let fixture = fixture();
    let video = fixture.video;
    let image = fixture.image;
    let mut editor = fixture.editor;
    add_effect(&mut editor.session, video, "blur", true);
    add_effect(&mut editor.session, video, "blur2", true);
    add_effect(&mut editor.session, image, "blur", true);
    add_effect(&mut editor.session, image, "blur2", true);
    editor
        .preferences
        .properties_tabs
        .insert("image".into(), "effects".into());
    let mut harness = build(editor);
    harness.run_steps(3);
    open_effect_tabs(&mut harness, video);
    assert_eq!(effect_ids(&harness, video), vec!["blur", "blur2"]);

    for (source, target, cancel, changed) in [
        ("blur2", "blur", false, true),
        ("blur", "blur2", false, true),
        ("blur", "blur", false, false),
        ("blur2", "blur", true, false),
    ] {
        let history = harness.state().session.history_len();
        let handle = format!("effects.row.{source}.handle");
        let start = widget_rect(&harness, &handle).center();
        let target = widget_rect(&harness, &format!("effects.toggle.{target}")).center();
        drag_to(&mut harness, &handle, target - start);
        if cancel {
            harness.key_press(egui::Key::Escape);
            harness.run_steps(2);
        }
        harness.drop_at(target);
        harness.run_steps(3);
        assert_eq!(
            effect_ids(&harness, video),
            if changed {
                vec!["blur2", "blur"]
            } else {
                vec!["blur", "blur2"]
            },
            "source {source}, cancel {cancel}"
        );
        assert_eq!(
            harness.state().session.history_len().0,
            history.0 + usize::from(changed)
        );
        if changed {
            harness.state_mut().activate("edit.undo").unwrap();
            harness.run_steps(2);
        }
        assert_eq!(effect_ids(&harness, video), vec!["blur", "blur2"]);
    }

    let history = harness.state().session.history_len();
    let start = widget_rect(&harness, "effects.row.blur2.handle").center();
    let target = widget_rect(&harness, "effects.toggle.blur").center();
    drag_to(&mut harness, "effects.row.blur2.handle", target - start);
    select(&mut harness, image);
    let target = widget_rect(&harness, "effects.toggle.blur").center();
    harness.drop_at(target);
    harness.run_steps(3);
    assert_eq!(effect_ids(&harness, video), vec!["blur", "blur2"]);
    assert_eq!(effect_ids(&harness, image), vec!["blur", "blur2"]);
    assert_eq!(harness.state().session.history_len(), history);
    select(&mut harness, video);

    let history = harness.state().session.history_len();
    let toggle = node_id(&harness, "effects.toggle.blur");
    let remove = node_id(&harness, "effects.remove.blur");
    click(&mut harness, "effects.toggle.blur");
    assert!(!effects(&harness, video)[0].enabled);
    assert!(has_widget(&harness, "effects.row.blur.disabled"));
    // The disabled label joins the row above the two controls. A press that
    // straddles that update must still release on the same widget.
    assert_eq!(
        node_id(&harness, "effects.toggle.blur"),
        toggle,
        "the toggle must keep its widget id when the disabled label appears"
    );
    assert_eq!(
        node_id(&harness, "effects.remove.blur"),
        remove,
        "remove must keep its widget id when the disabled label appears"
    );
    assert_eq!(harness.state().session.history_len().0, history.0 + 1);
    click(&mut harness, "effects.toggle.blur");
    assert!(effects(&harness, video)[0].enabled);
    assert_eq!(
        node_id(&harness, "effects.toggle.blur"),
        toggle,
        "the toggle must keep its widget id when the disabled label leaves"
    );

    let history = harness.state().session.history_len();
    click(&mut harness, "effects.remove.blur2");
    assert_eq!(effect_ids(&harness, video), vec!["blur"]);
    assert_eq!(harness.state().session.history_len().0, history.0 + 1);
    harness.state_mut().activate("edit.undo").unwrap();
    harness.run_steps(2);
    assert_eq!(effect_ids(&harness, video), vec!["blur", "blur2"]);
}

#[test]
fn effect_param_scrub_reverses_cancels_and_commits_once() {
    let fixture = fixture();
    let video = fixture.video;
    let mut editor = fixture.editor;
    add_effect(&mut editor.session, video, "blur", true);
    let mut harness = build(editor);
    harness.run_steps(3);
    open_effect_tabs(&mut harness, video);
    let intensity = |harness: &Harness<Editor>| {
        effects(harness, video)[0]
            .value("intensity")
            .and_then(EffectValue::scalar)
            .expect("a blur intensity")
    };

    let history = harness.state().session.history_len();
    let start = widget_rect(&harness, "effects.blur.param.intensity.scrub").center();
    drag_to(
        &mut harness,
        "effects.blur.param.intensity.scrub",
        egui::vec2(40.0, 0.0),
    );
    assert!(harness.state().session.gesture_active());
    assert_eq!(harness.state().session.history_len(), history);
    assert_eq!(intensity(&harness), 35.0);
    harness.hover_at(start);
    harness.run_steps(2);
    harness.drop_at(start);
    harness.run_steps(2);
    assert_eq!(intensity(&harness), 15.0, "reversal leaves the origin");
    assert_eq!(harness.state().session.history_len(), history);

    drag_to(
        &mut harness,
        "effects.blur.param.intensity.scrub",
        egui::vec2(40.0, 0.0),
    );
    assert_eq!(intensity(&harness), 35.0);
    harness.drop_at(start + egui::vec2(40.0, 0.0));
    harness.run_steps(2);
    assert_eq!(harness.state().session.history_len().0, history.0 + 1);
    harness.state_mut().activate("edit.undo").unwrap();
    harness.run_steps(2);
    assert_eq!(intensity(&harness), 15.0);

    let history = harness.state().session.history_len();
    drag_to(
        &mut harness,
        "effects.blur.param.intensity.scrub",
        egui::vec2(40.0, 0.0),
    );
    assert_eq!(intensity(&harness), 35.0);
    harness.key_press(egui::Key::Escape);
    harness.run_steps(2);
    harness.drop_at(start + egui::vec2(40.0, 0.0));
    harness.run_steps(2);
    assert_eq!(intensity(&harness), 15.0);
    assert_eq!(harness.state().session.history_len(), history);

    click(&mut harness, "effects.blur.param.intensity");
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.event(egui::Event::Text("150".into()));
    harness.key_press(egui::Key::Enter);
    harness.run_steps(3);
    assert_eq!(intensity(&harness), 100.0, "typed values clamp");
    let history = harness.state().session.history_len();
    click(&mut harness, "effects.blur.param.intensity");
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.event(egui::Event::Text("NaN".into()));
    harness.key_press(egui::Key::Enter);
    harness.run_steps(3);
    assert_eq!(intensity(&harness), 100.0);
    assert_eq!(harness.state().session.history_len(), history);
}

#[test]
fn a_locked_effect_colour_keyframe_toggle_says_why_on_hover() {
    let f = fixture();
    let video = f.video;
    let main = f.main;
    let mut editor = f.editor;
    editor
        .session
        .apply(Command::AddEffect {
            item: video,
            effect: project::effect_definition("strobe")
                .unwrap()
                .instance("rgb")
                .with("duration", EffectValue::Scalar(30.0))
                .unwrap(),
        })
        .unwrap();
    editor
        .session
        .apply(Command::SetTrackFlags {
            track: main,
            muted: false,
            hidden: false,
            locked: true,
        })
        .unwrap();
    let mut harness = build(editor);
    harness.run_steps(3);
    open_effect_tabs(&mut harness, video);
    assert!(harness.query_by_label("Track is locked").is_none());
    let toggle = widget_rect(&harness, "effects.rgb.param.color.keyframe").center();
    harness.hover_at(toggle);
    harness.run_steps(60);
    assert!(
        harness.query_by_label("Track is locked").is_some(),
        "the blocked toggle shows the lock reason"
    );
}

#[test]
fn effect_add_refuses_locked_tracks_and_the_sixty_fifth_effect() {
    let locked = fixture();
    let video = locked.video;
    let main = locked.main;
    let mut editor = locked.editor;
    editor
        .session
        .apply(Command::SetTrackFlags {
            track: main,
            muted: false,
            hidden: false,
            locked: true,
        })
        .unwrap();
    let mut harness = build(editor);
    harness.run_steps(3);
    select(&mut harness, video);
    click(&mut harness, "assets.tab.effects");
    let history = harness.state().session.history_len();
    click(&mut harness, "effects.add.blur");
    assert!(effects(&harness, video).is_empty());
    assert_eq!(harness.state().session.history_len(), history);
    let toast = harness
        .state()
        .inspect()
        .unwrap()
        .import_toast
        .unwrap_or_default();
    assert!(toast.contains("Track is locked"), "{toast}");

    let limit = fixture();
    let video = limit.video;
    let mut editor = limit.editor;
    for index in 0..64 {
        add_effect(&mut editor.session, video, &format!("fx{index}"), true);
    }
    let mut harness = build(editor);
    harness.run_steps(3);
    select(&mut harness, video);
    click(&mut harness, "assets.tab.effects");
    let before = effects(&harness, video).len();
    let history = harness.state().session.history_len();
    click(&mut harness, "effects.add.blur");
    assert_eq!(effects(&harness, video).len(), before);
    assert_eq!(harness.state().session.history_len(), history);
    let toast = harness
        .state()
        .inspect()
        .unwrap()
        .import_toast
        .unwrap_or_default();
    assert!(toast.contains("Effect limit reached"), "{toast}");
}

struct BlockingPreview;

struct PreviewGate {
    state: std::sync::Mutex<(bool, usize, usize)>, // released, calls, active
    changed: std::sync::Condvar,
    failure: Option<u8>,
    order: std::sync::Mutex<Vec<&'static str>>,
    cancelled: std::sync::atomic::AtomicUsize,
}
impl PreviewGate {
    fn new(released: bool, failure: Option<u8>) -> Self {
        Self {
            state: std::sync::Mutex::new((released, 0, 0)),
            changed: Default::default(),
            failure,
            order: Default::default(),
            cancelled: Default::default(),
        }
    }
    fn wait_started(&self) {
        let (state, timeout) = self
            .changed
            .wait_timeout_while(
                self.state.lock().unwrap(),
                std::time::Duration::from_secs(10),
                |s| s.1 == 0,
            )
            .unwrap();
        assert!(
            !timeout.timed_out() && state.1 > 0,
            "real thumbnail worker never started"
        );
    }
    fn release(&self) {
        self.state.lock().unwrap().0 = true;
        self.changed.notify_all();
    }
}
impl render::effects::EffectCompositor for PreviewGate {
    fn frame<'a>(
        &'a self,
        _: &'a dyn render::ExecutionHooks,
    ) -> render::Result<Option<Box<dyn render::effects::EffectCompositor + 'a>>> {
        Ok(None)
    }
    fn release_idle(&self) -> render::Result<()> {
        Ok(())
    }
    fn supports(&self, _: &str) -> bool {
        true
    }
    fn apply(
        &self,
        current: &render::Image,
        passes: &[render::effects::EffectPass],
        inputs: render::effects::EffectInputs<'_>,
        hooks: &dyn render::ExecutionHooks,
    ) -> render::Result<render::Image> {
        let mut state = self.state.lock().unwrap();
        state.1 += 1;
        state.2 += 1;
        self.order.lock().unwrap().push(passes[0].entry_point());
        self.changed.notify_all();
        while !state.0 && !hooks.cancelled() {
            state = self
                .changed
                .wait_timeout(state, std::time::Duration::from_millis(2))
                .unwrap()
                .0;
        }
        state.2 -= 1;
        drop(state);
        if hooks.cancelled() {
            self.cancelled
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            return Err(render::Error::Cancelled);
        }
        if passes.iter().any(|p| p.entry_point() == "gaussian") {
            match self.failure {
                Some(0) => return Err(render::Error::Invalid("test preview")),
                Some(1) => {
                    return Err(render::Error::Budget {
                        requested: 1,
                        limit: 0,
                    });
                }
                Some(2) => return Err(render::Error::Cancelled),
                _ => {}
            }
        }
        render::effects::CpuEffects.apply(current, passes, inputs, hooks)
    }
}

#[test]
fn effect_preview_queue_defers_then_renders_every_library_kind_without_retry() {
    use vibecut::EffectPreviews;
    let definitions: Vec<_> = project::effect_definitions()
        .iter()
        .filter(|definition| definition.shelf.in_library())
        .collect();
    assert_eq!(definitions.len(), 43);
    // Five is the smallest request set that exceeds the real four-entry queue.
    for count in [5, definitions.len()] {
        let mut previews = EffectPreviews::cpu().unwrap();
        for definition in definitions.iter().take(count) {
            previews.ensure(
                definition.id,
                &definition.instance(definition.id),
                &project::Animations::default(),
            );
        }
        let statuses = previews.statuses();
        assert_eq!(statuses.len(), count);
        assert!(
            statuses
                .values()
                .all(|s| *s == EffectPreviewStatus::Pending),
            "queue pressure must defer requests, not fail them: {statuses:?}"
        );
        previews.settle(SETTLE_BUDGET).unwrap();
        for definition in definitions.iter().take(count) {
            assert_eq!(
                previews.status(definition.id),
                Some(EffectPreviewStatus::Ready),
                "{}: {:?}",
                definition.id,
                previews.failure(definition.id)
            );
            let image = previews.image(definition.id).unwrap();
            assert_eq!(image.size, [320, 180]);
        }
    }
    // Replacing a full, unpumped queue leaves only deferred entries: the old
    // cancelled jobs still occupy the runner but none belongs to a current tile.
    let mut previews = EffectPreviews::cpu().unwrap();
    for definition in definitions.iter().take(4) {
        previews.ensure(
            definition.id,
            &definition.instance(definition.id),
            &project::Animations::default(),
        );
    }
    for definition in definitions.iter().take(4) {
        previews.retry(
            definition.id,
            &definition.instance(definition.id),
            &project::Animations::default(),
        );
    }
    assert!(
        matches!(
            previews.settle(std::time::Duration::ZERO),
            Err(vibecut::Error::PreviewUnavailable(_))
        ),
        "deferred-only work must not falsely settle"
    );
    assert!(
        previews
            .statuses()
            .values()
            .all(|s| *s == EffectPreviewStatus::Pending)
    );
    previews.settle(SETTLE_BUDGET).unwrap();
    assert!(
        previews
            .statuses()
            .values()
            .all(|s| *s == EffectPreviewStatus::Ready)
    );
}

#[test]
fn deferred_previews_timeout_retry_new_generation_and_drop_cancel_owned_jobs() {
    use vibecut::EffectPreviews;
    for retry in [false, true] {
        let budget = ByteBudget::new(16 << 20).unwrap();
        let gate = Arc::new(PreviewGate::new(false, None));
        let cache = engine::effect_thumbnails::EffectThumbnails::new(gate.clone(), budget.clone());
        let mut previews = EffectPreviews::new(cache, budget.clone()).unwrap();
        let blur = project::effect_definition("blur").unwrap().instance("blur");
        for definition in project::effect_definitions()
            .iter()
            .filter(|d| d.shelf.in_library())
        {
            previews.ensure(
                definition.id,
                &definition.instance(definition.id),
                &project::Animations::default(),
            );
        }
        previews.tick().unwrap();
        gate.wait_started();
        assert!(matches!(
            previews.settle(std::time::Duration::from_millis(20)),
            Err(vibecut::Error::PreviewUnavailable(_))
        ));
        assert_eq!(
            previews
                .statuses()
                .values()
                .filter(|s| **s == EffectPreviewStatus::Pending)
                .count(),
            43
        );
        if retry {
            let replacement = blur
                .clone()
                .with("intensity", EffectValue::Scalar(0.0))
                .unwrap();
            // Retry one active and one deferred kind; ordinary ensure cannot duplicate either.
            previews.retry("blur", &replacement, &project::Animations::default());
            let extract = project::effect_definition("extract")
                .unwrap()
                .instance("extract");
            previews.retry("extract", &extract, &project::Animations::default());
            previews.ensure("blur", &replacement, &project::Animations::default());
            gate.release();
            previews.settle(SETTLE_BUDGET).unwrap();
            assert_eq!(previews.statuses().len(), 43);
            assert!(
                previews
                    .statuses()
                    .values()
                    .all(|s| *s == EffectPreviewStatus::Ready)
            );
            let mut expected = EffectPreviews::cpu().unwrap();
            expected.ensure("blur", &replacement, &project::Animations::default());
            expected.settle(SETTLE_BUDGET).unwrap();
            assert_eq!(
                previews.image("blur").unwrap(),
                expected.image("blur").unwrap(),
                "cancelled old blur cannot replace the fresh identity request"
            );
            assert!(gate.state.lock().unwrap().1 > 1, "other kinds progressed");
            let order = gate.order.lock().unwrap();
            assert_eq!(
                order.iter().filter(|kind| **kind == "extract").count(),
                1,
                "a deferred retry admits exactly one fresh job"
            );
            assert!(
                order.iter().position(|kind| *kind == "invert").unwrap()
                    < order.iter().position(|kind| *kind == "extract").unwrap(),
                "retry must join behind already deferred requests: {order:?}"
            );
        }
        drop(previews); // Includes blocked active, queued and deferred requests in the first row.
        assert_eq!(
            gate.cancelled.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "old active job observes cancellation on retry or Drop"
        );
        assert_eq!(gate.state.lock().unwrap().2, 0, "adapter joined on drop");
        assert_eq!(
            budget.snapshot().unwrap().used,
            0,
            "source, jobs and cache leases released"
        );
    }
}

#[test]
fn preview_permanent_failures_do_not_retry_or_starve_healthy_deferred_kinds() {
    use vibecut::EffectPreviews;
    for (failure, retry) in [0, 1, 2]
        .into_iter()
        .flat_map(|failure| [(failure, false), (failure, true)])
    {
        let budget = ByteBudget::new(16 << 20).unwrap();
        let adapter = Arc::new(PreviewGate::new(true, Some(failure)));
        let cache =
            engine::effect_thumbnails::EffectThumbnails::new(adapter.clone(), budget.clone());
        let mut previews = EffectPreviews::new(cache, budget.clone()).unwrap();
        for definition in project::effect_definitions()
            .iter()
            .filter(|d| d.shelf.in_library())
        {
            let mut effect = definition.instance(definition.id);
            if definition.id == "tint" && !retry {
                effect.params.clear();
            } // Validation still refuses behind a full queue.
            previews.ensure(definition.id, &effect, &project::Animations::default());
        }
        let tint = project::effect_definition("tint").unwrap().instance("tint");
        let mut malformed = tint.clone();
        malformed.params.clear();
        let refusal = project::Error::Invalid("effect parameter set").to_string();
        if retry {
            // Four valid jobs still fill the runner; the older valid deferred
            // head must hit QueueFull before the existing tint retry is reached.
            previews.retry("tint", &malformed, &project::Animations::default());
        }
        assert_eq!(
            previews.status("tint"),
            Some(EffectPreviewStatus::Failed),
            "failure {failure}, retry {retry}"
        );
        assert_eq!(previews.failure("tint"), Some(refusal.as_str()));
        previews.settle(SETTLE_BUDGET).unwrap();
        // The gate fails every list that needs the shared gaussian, which is Blur
        // plus each whole-image glow, shadow and torn-edge entry. Ask the real
        // evaluator instead of keeping a second, hand-maintained list of those.
        let blurs = |kind: &str| {
            render::effects::evaluate_effect(
                &project::Animations::default(),
                &project::effect_definition(kind).unwrap().instance(kind),
                Tick::ZERO,
                EffectGeometry::new(vibecut::PREVIEW_SIZE, vibecut::PREVIEW_SCALE).unwrap(),
                false,
                &render::lut::LutResources::default(),
            )
            .unwrap()
            .iter()
            .any(|pass| pass.entry_point() == "gaussian")
        };
        for (kind, status) in previews.statuses() {
            assert_eq!(
                status,
                if kind == "tint" || blurs(&kind) {
                    EffectPreviewStatus::Failed
                } else {
                    EffectPreviewStatus::Ready
                },
                "failure {failure}, kind {kind}"
            );
        }
        let calls = adapter.state.lock().unwrap().1;
        for _ in 0..3 {
            previews.tick().unwrap();
        }
        assert_eq!(
            adapter.state.lock().unwrap().1,
            calls,
            "terminal errors are not transient admission"
        );
        assert!(previews.failure("blur").is_some());
        if retry {
            previews.retry("tint", &tint, &project::Animations::default());
            previews.settle(SETTLE_BUDGET).unwrap();
            let last_good = previews.image("tint").unwrap();
            // Refuse the same malformed retry with a last-good image present,
            // behind four queued valid refreshes and an older deferred head.
            for kind in [
                "black_white",
                "channel_mix",
                "color_pass",
                "color_replace",
                "extract",
            ] {
                previews.retry(
                    kind,
                    &project::effect_definition(kind).unwrap().instance(kind),
                    &project::Animations::default(),
                );
            }
            previews.retry("tint", &malformed, &project::Animations::default());
            assert_eq!(previews.status("tint"), Some(EffectPreviewStatus::Failed));
            assert_eq!(previews.failure("tint"), Some(refusal.as_str()));
            assert!(Arc::ptr_eq(&last_good, &previews.image("tint").unwrap()));
            previews.settle(SETTLE_BUDGET).unwrap();
            // No refused generation retains a desired frame/job after draining.
            previews.settle(std::time::Duration::ZERO).unwrap();
            previews.retry("tint", &tint, &project::Animations::default());
            previews.settle(SETTLE_BUDGET).unwrap();
            assert_eq!(previews.status("tint"), Some(EffectPreviewStatus::Ready));
            assert!(previews.failure("tint").is_none());
            assert_eq!(previews.image("tint").unwrap(), last_good);
        }
        drop(previews);
        assert_eq!(budget.snapshot().unwrap().used, 0);
        eprintln!("permanent admission: failure={failure} existing_retry={retry} passed");
    }
}

#[test]
fn evicted_preview_job_becomes_failed_instead_of_remaining_pending() {
    use vibecut::EffectPreviews;
    let limit = 16 << 20;
    let budget = ByteBudget::new(limit).unwrap();
    let cache = engine::effect_thumbnails::EffectThumbnails::new(
        Arc::new(render::effects::CpuEffects),
        budget.clone(),
    );
    let mut previews = EffectPreviews::new(cache, budget.clone()).unwrap();
    let identity = project::effect_definition("channel_mix")
        .unwrap()
        .instance("channel_mix");
    previews.ensure("channel_mix", &identity, &project::Animations::default());
    let blur = project::effect_definition("blur").unwrap().instance("blur");
    previews.ensure("blur", &blur, &project::Animations::default());
    previews.tick().unwrap();
    // Wait without ticking away the completed result. Once working images are
    // released, only source + runner result ceiling + packed cache image remain.
    let retained = 320 * 180 * 16 + 320 * 320 * 4 + 320 * 180 * 4;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while budget.snapshot().unwrap().used != retained {
        assert!(
            std::time::Instant::now() < deadline,
            "identity job did not release its work images"
        );
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    // The runner must evict that unconsumed result to admit the next real job.
    let pressure = budget
        .reserve(engine::ByteOwner::JobResults, limit - retained - 1)
        .unwrap();
    while previews.status("channel_mix") == Some(EffectPreviewStatus::Pending) {
        assert!(
            std::time::Instant::now() < deadline,
            "evicted ID left Pending forever"
        );
        previews.tick().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert_eq!(
        previews.status("channel_mix"),
        Some(EffectPreviewStatus::Failed)
    );
    assert_eq!(
        previews.failure("channel_mix"),
        Some(
            engine::control::Error::Runtime(engine::RuntimeError::JobNotRetained)
                .to_string()
                .as_str()
        )
    );
    drop(pressure);
    previews.settle(SETTLE_BUDGET).unwrap();
    drop(previews);
    assert_eq!(budget.snapshot().unwrap().used, 0);
}

impl render::effects::EffectCompositor for BlockingPreview {
    fn frame<'a>(
        &'a self,
        _: &'a dyn render::ExecutionHooks,
    ) -> render::Result<Option<Box<dyn render::effects::EffectCompositor + 'a>>> {
        Ok(None)
    }
    fn release_idle(&self) -> render::Result<()> {
        Ok(())
    }
    fn supports(&self, _: &str) -> bool {
        true
    }
    fn apply(
        &self,
        _: &render::Image,
        _: &[render::effects::EffectPass],
        _: render::effects::EffectInputs<'_>,
        hooks: &dyn render::ExecutionHooks,
    ) -> render::Result<render::Image> {
        while !hooks.cancelled() {
            std::thread::yield_now();
        }
        Err(render::Error::Cancelled)
    }
}
struct FailingPreview;
impl render::effects::EffectCompositor for FailingPreview {
    fn frame<'a>(
        &'a self,
        _: &'a dyn render::ExecutionHooks,
    ) -> render::Result<Option<Box<dyn render::effects::EffectCompositor + 'a>>> {
        Ok(None)
    }
    fn release_idle(&self) -> render::Result<()> {
        Ok(())
    }
    fn supports(&self, _: &str) -> bool {
        true
    }
    fn apply(
        &self,
        _: &render::Image,
        _: &[render::effects::EffectPass],
        _: render::effects::EffectInputs<'_>,
        _: &dyn render::ExecutionHooks,
    ) -> render::Result<render::Image> {
        Err(render::Error::Invalid("test preview"))
    }
}

#[test]
fn effect_preview_tiles_render_pending_failed_and_ready_states() {
    let pending = fixture();
    let video = pending.video;
    let mut harness = build(pending.editor);
    inject_previews(&mut harness, Arc::new(BlockingPreview));
    harness.run_steps(3);
    select(&mut harness, video);
    click(&mut harness, "assets.tab.effects");
    assert_eq!(
        harness
            .state()
            .inspect()
            .unwrap()
            .effect_previews
            .get("blur")
            .copied(),
        Some(EffectPreviewStatus::Pending)
    );
    assert!(has_widget(&harness, "effects.tile.blur.pending"));

    let failed = fixture();
    let video = failed.video;
    let mut harness = build(failed.editor);
    inject_previews(&mut harness, Arc::new(FailingPreview));
    harness.run_steps(3);
    select(&mut harness, video);
    click(&mut harness, "assets.tab.effects");
    wait_preview(&mut harness, "blur", EffectPreviewStatus::Failed);
    assert!(has_widget(&harness, "effects.tile.blur.failed"));
    assert!(has_widget(&harness, "effects.preview.retry.blur"));

    let ready = fixture();
    let video = ready.video;
    let mut harness = build(ready.editor);
    harness.run_steps(3);
    select(&mut harness, video);
    click(&mut harness, "assets.tab.effects");
    wait_preview(&mut harness, "blur", EffectPreviewStatus::Ready);
    assert!(has_widget(&harness, "effects.tile.blur.ready"));
    assert_eq!(
        harness
            .state()
            .inspect()
            .unwrap()
            .effect_previews
            .get("blur")
            .copied(),
        Some(EffectPreviewStatus::Ready)
    );
}

#[test]
fn effect_preview_settle_waits_for_terminal_tiles_or_times_out() {
    let settled = fixture();
    let mut harness = build(settled.editor);
    harness.run_steps(3);
    click(&mut harness, "assets.tab.effects");
    harness.state_mut().settle_previews(SETTLE_BUDGET).unwrap();
    let statuses = harness.state().inspect().unwrap().effect_previews;
    assert_eq!(statuses.len(), 43);
    assert!(
        statuses
            .values()
            .all(|status| *status == EffectPreviewStatus::Ready),
        "{statuses:?}"
    );
    harness.run_steps(3);
    for definition in project::effect_definitions()
        .iter()
        .filter(|d| d.shelf.in_library())
    {
        assert!(has_widget(
            &harness,
            &format!("effects.tile.{}.ready", definition.id)
        ));
        assert!(!has_widget(
            &harness,
            &format!("effects.preview.retry.{}", definition.id)
        ));
    }

    let bottom = widget_rect(&harness, "effects.tile.tint").top();
    harness.get_by_label("Two-tone").scroll_to_me();
    harness.run_steps(3);
    assert!(widget_rect(&harness, "effects.tile.tint").top() < bottom);
    let blocking: Arc<dyn render::effects::EffectCompositor> = Arc::new(BlockingPreview);
    let failing: Arc<dyn render::effects::EffectCompositor> = Arc::new(FailingPreview);
    for (source, timeout, expected) in [
        (blocking, 50, None),
        (failing, 10_000, Some(EffectPreviewStatus::Failed)),
    ] {
        let fixture = fixture();
        let video = fixture.video;
        let mut harness = build(fixture.editor);
        inject_previews(&mut harness, source);
        harness.run_steps(3);
        select(&mut harness, video);
        click(&mut harness, "assets.tab.effects");
        let result = harness
            .state_mut()
            .settle_previews(std::time::Duration::from_millis(timeout));
        match expected {
            None => assert!(matches!(
                result.unwrap_err(),
                vibecut::Error::PreviewUnavailable(_)
            )),
            Some(status) => {
                result.unwrap();
                assert_eq!(
                    harness
                        .state()
                        .inspect()
                        .unwrap()
                        .effect_previews
                        .get("blur")
                        .copied(),
                    Some(status)
                );
            }
        }
    }
}

#[test]
fn effect_preferences_round_trip_through_the_app_store() {
    let dir = tempfile::tempdir().unwrap();
    let store = PreferenceStore::new(dir.path().join("preferences.json"));
    let fixture = fixture();
    let mut harness = build(fixture.editor);
    harness.run_steps(3);
    click(&mut harness, "assets.tab.effects");
    store.save(&harness.state().preferences).unwrap();
    let restored = store.load_with_warning().0;
    assert_eq!(restored.assets_tab, vibecut::AssetsTab::Effects);
}

fn grade(harness: &Harness<Editor>, id: ItemId) -> Option<[f64; 4]> {
    effects(harness, id)
        .into_iter()
        .filter(|effect| effect.kind == "color_grade")
        .find_map(|effect| {
            let scalar = |key: &str| effect.value(key)?.scalar();
            Some([
                scalar("brightness")?,
                scalar("contrast")?,
                scalar("saturation")?,
                scalar("hue")?,
            ])
        })
}

/// One wheel family's three stored percentages, neutral when the item has no grade.
fn grade_wheel(harness: &Harness<Editor>, id: ItemId, family: &str) -> [f64; 3] {
    let keys = match family {
        "lift" => ["lift_r", "lift_g", "lift_b"],
        "gamma" => ["gamma_r", "gamma_g", "gamma_b"],
        _ => ["gain_r", "gain_g", "gain_b"],
    };
    let neutral = if family == "lift" { 0.0 } else { 100.0 };
    let effect = effects(harness, id)
        .into_iter()
        .find(|effect| effect.kind == "color_grade");
    keys.map(|key| {
        effect
            .as_ref()
            .and_then(|effect| effect.value(key))
            .and_then(EffectValue::scalar)
            .unwrap_or(neutral)
    })
}

fn grade_curve(harness: &Harness<Editor>, id: ItemId, key: &str) -> project::ToneCurve {
    effects(harness, id)
        .into_iter()
        .find(|effect| effect.kind == "color_grade")
        .and_then(|effect| effect.value(key).and_then(EffectValue::curve).copied())
        .unwrap_or(project::ToneCurve::IDENTITY)
}

/// A plot coordinate as a screen position inside the curve widget.
fn curve_pos(rect: egui::Rect, x: f64, y: f64) -> egui::Pos2 {
    rect.min + egui::vec2(x as f32 * rect.width(), (1.0 - y as f32) * rect.height())
}

fn drag_from(harness: &mut Harness<Editor>, start: egui::Pos2, delta: egui::Vec2) {
    harness.hover_at(start);
    harness.run_steps(1);
    harness.drag_at(start);
    harness.run_steps(1);
    harness.hover_at(start + delta);
    harness.run_steps(2);
}

/// Both clicks arrive in one frame: `harness.event` spends a frame per event,
/// and egui counts a double click only inside its 0.3 s window.
fn double_click_pos(harness: &mut Harness<Editor>, at: egui::Pos2) {
    harness.event(egui::Event::PointerMoved(at));
    harness.run_steps(2);
    for _ in 0..2 {
        for pressed in [true, false] {
            harness.input_mut().events.push(egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::default(),
            });
        }
    }
    harness.run_steps(3);
}

fn type_value(harness: &mut Harness<Editor>, id: &str, text: &str) {
    click(harness, id);
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness.event(egui::Event::Text(text.into()));
    harness.key_press(egui::Key::Enter);
    harness.run_steps(3);
}

#[test]
fn adjust_tab_grades_visual_clips_with_one_undo_step_per_gesture() {
    let fixture = fixture();
    let (video, image, audio) = (fixture.video, fixture.image, fixture.audio);
    let mut harness = crate::gpu::builder()
        .with_size(egui::vec2(1727.0, 2400.0))
        .build_ui_state(|ui, editor: &mut Editor| editor.ui(ui), fixture.editor);
    harness.run_steps(3);
    select(&mut harness, audio);
    assert!(!has_widget(&harness, "inspector.tab.adjust"));
    select(&mut harness, fixture.graphic);
    assert!(!has_widget(&harness, "inspector.tab.adjust"));
    for item in [video, image] {
        select(&mut harness, item);
        click(&mut harness, "inspector.tab.adjust");
        for key in [
            "brightness",
            "contrast",
            "saturation",
            "hue",
            "temperature",
            "highlight",
            "shadow",
            "shine",
            "sharpness",
            "vignette",
            "fade",
            "grain",
        ] {
            assert!(has_widget(
                &harness,
                &format!("effects.color_grade.param.{key}")
            ));
        }
        // Basic, HSL, Curves and Color wheel are separate sub-tabs; only the
        // active tab paints its controls.
        assert!(!has_widget(&harness, "effects.color_grade.wheel.lift"));
        assert!(!has_widget(&harness, "effects.color_grade.curve.plot"));
        click(&mut harness, "effects.color_grade.tab.hsl");
        // The chosen band outlives the item, so each pass starts from red.
        click(&mut harness, "effects.color_grade.hsl.red");
        for band in [
            "red", "orange", "yellow", "green", "cyan", "blue", "purple", "magenta",
        ] {
            assert!(has_widget(
                &harness,
                &format!("effects.color_grade.hsl.{band}")
            ));
        }
        assert!(has_widget(
            &harness,
            "effects.color_grade.param.hsl_red_hue"
        ));
        click(&mut harness, "effects.color_grade.hsl.green");
        assert!(has_widget(
            &harness,
            "effects.color_grade.param.hsl_green_lightness"
        ));
        assert!(!has_widget(
            &harness,
            "effects.color_grade.param.hsl_red_hue"
        ));
        click(&mut harness, "effects.color_grade.tab.wheel");
        for family in ["lift", "gamma", "gain"] {
            assert!(
                has_widget(&harness, &format!("effects.color_grade.wheel.{family}")),
                "wheel {family}"
            );
            assert!(has_widget(
                &harness,
                &format!("effects.color_grade.wheel.{family}.master")
            ));
            assert!(has_widget(
                &harness,
                &format!("effects.color_grade.wheel.{family}.reset")
            ));
        }
        click(&mut harness, "effects.color_grade.tab.curves");
        for channel in ["red", "green", "blue", "luma"] {
            assert!(
                has_widget(&harness, &format!("effects.color_grade.curve.{channel}")),
                "curve channel {channel}"
            );
        }
        assert!(has_widget(&harness, "effects.color_grade.curve.plot"));
        assert!(has_widget(&harness, "effects.color_grade.curve.reset"));
        // The curve editor owns the points; no fake scalar row stands in for them.
        assert!(!has_widget(&harness, "effects.color_grade.param.curve_red"));
        assert!(grade(&harness, item).is_none(), "viewing adds nothing");
        assert_eq!(
            grade_curve(&harness, item, "curve_red"),
            project::ToneCurve::IDENTITY
        );
        click(&mut harness, "effects.color_grade.tab.basic");
    }
    select(&mut harness, video);
    click(&mut harness, "inspector.tab.adjust");
    click(&mut harness, "effects.color_grade.tab.basic");
    // First typed edit adds the effect carrying the value: one undo step.
    let history = harness.state().session.history_len();
    type_value(&mut harness, "effects.color_grade.param.brightness", "20");
    assert_eq!(grade(&harness, video), Some([20.0, 0.0, 0.0, 0.0]));
    assert_eq!(harness.state().session.history_len().0, history.0 + 1);
    // Typed values clamp to the -50..50 range.
    type_value(&mut harness, "effects.color_grade.param.hue", "999");
    assert_eq!(grade(&harness, video), Some([20.0, 0.0, 0.0, 50.0]));
    type_value(&mut harness, "effects.color_grade.param.saturation", "-999");
    assert_eq!(grade(&harness, video), Some([20.0, 0.0, -50.0, 50.0]));
    // The generic stack lists every band's rows, so each names its band; the HSL sub-tab picks
    // a band first and keeps the bare control name.
    let labelled = |harness: &Harness<Editor>, label: &str, id: &str| {
        harness.get(
            By::new()
                .label(label)
                .predicate(|node| node.author_id() == Some(id)),
        );
    };
    click(&mut harness, "inspector.tab.effects");
    for (label, id) in [
        ("Red: Hue", "effects.color_grade.param.hsl_red_hue"),
        (
            "Green: Saturation",
            "effects.color_grade.param.hsl_green_saturation",
        ),
        (
            "Magenta: Lightness",
            "effects.color_grade.param.hsl_magenta_lightness",
        ),
    ] {
        labelled(&harness, label, id);
    }
    click(&mut harness, "inspector.tab.adjust");
    click(&mut harness, "effects.color_grade.tab.hsl");
    labelled(&harness, "Hue", "effects.color_grade.param.hsl_green_hue");
    click(&mut harness, "effects.color_grade.tab.basic");
    let history = harness.state().session.history_len();
    type_value(&mut harness, "effects.color_grade.param.contrast", "NaN");
    assert_eq!(
        harness.state().session.history_len(),
        history,
        "NaN refused"
    );
    // A drag on a control is one gesture and one undo step; reversal is a no-op.
    let start = widget_rect(&harness, "effects.color_grade.param.contrast.scrub").center();
    drag_to(
        &mut harness,
        "effects.color_grade.param.contrast.scrub",
        egui::vec2(40.0, 0.0),
    );
    assert!(harness.state().session.gesture_active());
    assert_eq!(harness.state().session.history_len(), history);
    assert!(grade(&harness, video).unwrap()[1] > 0.0);
    harness.drop_at(start + egui::vec2(40.0, 0.0));
    harness.run_steps(2);
    assert_eq!(harness.state().session.history_len().0, history.0 + 1);
    harness.state_mut().activate("edit.undo").unwrap();
    harness.run_steps(2);
    assert_eq!(grade(&harness, video), Some([20.0, 0.0, -50.0, 50.0]));
    // A wheel drag is one gesture; each move writes all three channels through
    // `effects.update_param`, and the release is one undo step.
    click(&mut harness, "effects.color_grade.tab.wheel");
    let before = grade_wheel(&harness, video, "lift");
    let history = harness.state().session.history_len();
    let start = widget_rect(&harness, "effects.color_grade.wheel.lift").center();
    drag_from(&mut harness, start, egui::vec2(18.0, -10.0));
    assert!(harness.state().session.gesture_active());
    assert_eq!(harness.state().session.history_len(), history);
    let dragged = grade_wheel(&harness, video, "lift");
    assert!(dragged[0] > 0.0, "red lifts");
    assert!(dragged[1] < 0.0 && dragged[2] < 0.0, "green and blue fall");
    harness.drop_at(start + egui::vec2(18.0, -10.0));
    harness.run_steps(2);
    assert_eq!(harness.state().session.history_len().0, history.0 + 1);
    harness.state_mut().activate("edit.undo").unwrap();
    harness.run_steps(2);
    assert_eq!(grade_wheel(&harness, video, "lift"), before);
    // The master slider moves the shared level and keeps the puck's chroma.
    let before = grade_wheel(&harness, video, "gamma");
    let history = harness.state().session.history_len();
    let start = widget_rect(&harness, "effects.color_grade.wheel.gamma.master").center();
    drag_from(&mut harness, start, egui::vec2(0.0, -12.0));
    assert!(harness.state().session.gesture_active());
    assert_eq!(harness.state().session.history_len(), history);
    let lifted = grade_wheel(&harness, video, "gamma");
    assert!(lifted.iter().zip(before).all(|(high, low)| *high > low));
    harness.drop_at(start + egui::vec2(0.0, -12.0));
    harness.run_steps(2);
    assert_eq!(harness.state().session.history_len().0, history.0 + 1);
    harness.state_mut().activate("edit.undo").unwrap();
    harness.run_steps(2);
    assert_eq!(grade_wheel(&harness, video, "gamma"), before);
    // Reset writes the three neutral coefficients in one undo step.
    let start = widget_rect(&harness, "effects.color_grade.wheel.gain.master").center();
    drag_from(&mut harness, start, egui::vec2(0.0, -14.0));
    harness.drop_at(start + egui::vec2(0.0, -14.0));
    harness.run_steps(2);
    let raised = grade_wheel(&harness, video, "gain");
    assert!(raised.iter().all(|value| *value > 100.0));
    let history = harness.state().session.history_len();
    click(&mut harness, "effects.color_grade.wheel.gain.reset");
    assert_eq!(grade_wheel(&harness, video, "gain"), [100.0; 3]);
    assert_eq!(harness.state().session.history_len().0, history.0 + 1);
    harness.state_mut().activate("edit.undo").unwrap();
    harness.run_steps(2);
    assert_eq!(grade_wheel(&harness, video, "gain"), raised);
    harness.state_mut().activate("edit.undo").unwrap();
    harness.run_steps(2);
    assert_eq!(grade_wheel(&harness, video, "gain"), [100.0; 3]);
    // A curve click adds one point through `effects.update_param`.
    click(&mut harness, "effects.color_grade.tab.curves");
    let plot = widget_rect(&harness, "effects.color_grade.curve.plot");
    let history = harness.state().session.history_len();
    click_pos(&mut harness, curve_pos(plot, 0.5, 0.53));
    let added = grade_curve(&harness, video, "curve_red");
    assert_eq!(added.points().len(), 3);
    assert_eq!(harness.state().session.history_len().0, history.0 + 1);
    harness.state_mut().activate("edit.undo").unwrap();
    harness.run_steps(2);
    assert_eq!(grade_curve(&harness, video, "curve_red").points().len(), 2);
    // Dragging that point is one gesture; the release is one undo step.
    click_pos(&mut harness, curve_pos(plot, 0.5, 0.53));
    let before = grade_curve(&harness, video, "curve_red");
    assert_eq!(before.points().len(), 3);
    let point = before.points().get(1).copied().expect("an interior point");
    let start = curve_pos(plot, point[0], point[1]);
    let history = harness.state().session.history_len();
    drag_from(&mut harness, start, egui::vec2(10.0, -8.0));
    assert!(harness.state().session.gesture_active());
    assert_eq!(harness.state().session.history_len(), history);
    let moved = grade_curve(&harness, video, "curve_red");
    assert_ne!(moved, before);
    assert_eq!(moved.points().len(), 3);
    harness.drop_at(start + egui::vec2(10.0, -8.0));
    harness.run_steps(2);
    assert_eq!(harness.state().session.history_len().0, history.0 + 1);
    harness.state_mut().activate("edit.undo").unwrap();
    harness.run_steps(2);
    assert_eq!(grade_curve(&harness, video, "curve_red"), before);
    // Endpoints are fixed: a drag that would move one changes nothing.
    let history = harness.state().session.history_len();
    drag_from(&mut harness, plot.left_bottom(), egui::vec2(12.0, -12.0));
    harness.drop_at(plot.left_bottom() + egui::vec2(12.0, -12.0));
    harness.run_steps(2);
    assert_eq!(grade_curve(&harness, video, "curve_red"), before);
    assert_eq!(harness.state().session.history_len(), history);
    // A double-click removes only an interior point.
    let history = harness.state().session.history_len();
    double_click_pos(&mut harness, start);
    assert_eq!(grade_curve(&harness, video, "curve_red").points().len(), 2);
    assert_eq!(harness.state().session.history_len().0, history.0 + 1);
    harness.state_mut().activate("edit.undo").unwrap();
    harness.run_steps(2);
    assert_eq!(grade_curve(&harness, video, "curve_red"), before);
    // Reset restores the two-point identity in one undo step.
    let history = harness.state().session.history_len();
    click(&mut harness, "effects.color_grade.curve.reset");
    assert!(grade_curve(&harness, video, "curve_red").is_identity());
    assert_eq!(harness.state().session.history_len().0, history.0 + 1);
    harness.state_mut().activate("edit.undo").unwrap();
    harness.run_steps(2);
    assert_eq!(grade_curve(&harness, video, "curve_red"), before);
    // The block's first click added the point as its own edit; drop it too.
    harness.state_mut().activate("edit.undo").unwrap();
    harness.run_steps(2);
    assert_eq!(grade_curve(&harness, video, "curve_red").points().len(), 2);
    // Switching the shown channel changes no project data.
    let snapshot = harness.state().session.project().clone();
    let history = harness.state().session.history_len();
    click(&mut harness, "effects.color_grade.curve.green");
    click(&mut harness, "effects.color_grade.curve.luma");
    assert_eq!(harness.state().session.project(), &snapshot);
    assert_eq!(harness.state().session.history_len(), history);
    // Everything undoes back to an item with no effect at all.
    for _ in 0..3 {
        harness.state_mut().activate("edit.undo").unwrap();
        harness.run_steps(2);
    }
    assert!(grade(&harness, video).is_none());
    // The HSL tab writes only the selected band's three rows, still through
    // `effects.update_param`, and changing bands changes no project data.
    click(&mut harness, "effects.color_grade.tab.basic");
    let history = harness.state().session.history_len();
    type_value(&mut harness, "effects.color_grade.param.brightness", "10");
    assert_eq!(grade(&harness, video), Some([10.0, 0.0, 0.0, 0.0]));
    assert_eq!(harness.state().session.history_len().0, history.0 + 1);
    click(&mut harness, "effects.color_grade.tab.hsl");
    click(&mut harness, "effects.color_grade.hsl.green");
    type_value(
        &mut harness,
        "effects.color_grade.param.hsl_green_hue",
        "25",
    );
    let green = effects(&harness, video)
        .into_iter()
        .find(|effect| effect.kind == "color_grade")
        .and_then(|effect| effect.value("hsl_green_hue").and_then(EffectValue::scalar));
    assert_eq!(green, Some(25.0));
    harness.state_mut().activate("edit.undo").unwrap();
    harness.run_steps(2);
    let green = effects(&harness, video)
        .into_iter()
        .find(|effect| effect.kind == "color_grade")
        .and_then(|effect| effect.value("hsl_green_hue").and_then(EffectValue::scalar));
    assert_eq!(green, Some(0.0));
    // Apply to all copies the grade onto every video and image item in one batch.
    let history = harness.state().session.history_len();
    click(&mut harness, "effects.color_grade.apply_all");
    assert_eq!(grade(&harness, video), Some([10.0, 0.0, 0.0, 0.0]));
    assert_eq!(grade(&harness, image), Some([10.0, 0.0, 0.0, 0.0]));
    assert_eq!(harness.state().session.history_len().0, history.0 + 1);
    harness.state_mut().activate("edit.undo").unwrap();
    harness.run_steps(2);
    assert!(grade(&harness, image).is_none());
    // Save as preset stores the grade with the app, not the project.
    let history = harness.state().session.history_len();
    click(&mut harness, "effects.color_grade.save_preset");
    let presets = harness.state().preferences.adjustment_presets.clone();
    assert_eq!(presets.len(), 1);
    assert_eq!(presets[0].name, "Preset 1");
    assert_eq!(
        presets[0]
            .params
            .get("brightness")
            .and_then(EffectValue::scalar),
        Some(10.0)
    );
    assert_eq!(harness.state().session.history_len(), history);
    // The saved preset is listed under the Adjustment assets tab and applies to
    // the selected clip as one undo step.
    click(&mut harness, "assets.tab.adjustment");
    assert!(has_widget(&harness, "assets.adjustment.0.apply"));
    select(&mut harness, image);
    assert!(grade(&harness, image).is_none());
    let history = harness.state().session.history_len();
    click(&mut harness, "assets.adjustment.0.apply");
    assert_eq!(grade(&harness, image), Some([10.0, 0.0, 0.0, 0.0]));
    assert_eq!(harness.state().session.history_len().0, history.0 + 1);
    harness.state_mut().activate("edit.undo").unwrap();
    harness.run_steps(2);
    assert!(grade(&harness, image).is_none());
    select(&mut harness, video);
    harness.state_mut().activate("edit.undo").unwrap();
    harness.run_steps(2);
    assert!(grade(&harness, video).is_none());
    // A locked track refuses the first edit and stays untouched.
    harness
        .state_mut()
        .session
        .apply(Command::SetTrackFlags {
            track: fixture.main,
            muted: false,
            hidden: false,
            locked: true,
        })
        .unwrap();
    harness.run_steps(2);
    click(&mut harness, "effects.color_grade.tab.basic");
    type_value(&mut harness, "effects.color_grade.param.brightness", "10");
    assert!(grade(&harness, video).is_none());
    // The wheels and curve plot refuse on the same lock without a history entry.
    let history = harness.state().session.history_len();
    click(&mut harness, "effects.color_grade.tab.wheel");
    let start = widget_rect(&harness, "effects.color_grade.wheel.lift").center();
    drag_from(&mut harness, start, egui::vec2(16.0, -8.0));
    harness.drop_at(start + egui::vec2(16.0, -8.0));
    harness.run_steps(2);
    assert!(grade(&harness, video).is_none());
    click(&mut harness, "effects.color_grade.tab.curves");
    let plot = widget_rect(&harness, "effects.color_grade.curve.plot");
    click_pos(&mut harness, curve_pos(plot, 0.5, 0.53));
    assert!(grade(&harness, video).is_none());
    assert_eq!(harness.state().session.history_len(), history);
}

fn chroma(harness: &Harness<Editor>, id: ItemId) -> Option<project::EffectInstance> {
    effects(harness, id)
        .into_iter()
        .find(|effect| effect.kind == "chroma_key")
}

fn chroma_parts(harness: &Harness<Editor>, id: ItemId) -> Option<(project::Color, f64, f64, bool)> {
    chroma(harness, id).and_then(|effect| {
        Some((
            effect.value("key_color")?.color()?,
            effect.value("intensity")?.scalar()?,
            effect.value("shadow")?.scalar()?,
            effect.enabled,
        ))
    })
}

fn channel_value(harness: &Harness<Editor>, id: ItemId, path: &str, at: Tick) -> Option<f64> {
    let path = project::AnimationPath::new(path).unwrap();
    match item(harness, id).animations.value_at(&path, at).ok()? {
        Some(project::ParamValue::Scalar(value)) => Some(value),
        _ => None,
    }
}

fn has_key(harness: &Harness<Editor>, id: ItemId, path: &str, at: Tick) -> bool {
    let path = project::AnimationPath::new(path).unwrap();
    item(harness, id)
        .animations
        .get(&path)
        .and_then(|param| match param {
            project::Param::Value(channel) => Some(channel),
            project::Param::Color(_) => None,
        })
        .is_some_and(|channel| {
            channel
                .keys()
                .binary_search_by_key(&at, |key| key.time)
                .is_ok()
        })
}

fn open_remove_background(harness: &mut Harness<Editor>, item: ItemId) {
    select(harness, item);
    click(harness, "inspector.tab.remove_background");
}

fn click_pos(harness: &mut Harness<Editor>, pos: egui::Pos2) {
    harness.hover_at(pos);
    harness.run_steps(1);
    harness.drag_at(pos);
    harness.run_steps(1);
    harness.drop_at(pos);
    harness.run_steps(2);
}

fn add_chroma_effect(session: &mut Session, item: ItemId) {
    session
        .apply(Command::AddEffect {
            item,
            effect: project::effect_definition("chroma_key")
                .unwrap()
                .instance("chroma_key")
                .with("key_color", EffectValue::Color(Color([0, 255, 0, 255])))
                .unwrap()
                .with(
                    "intensity",
                    EffectValue::Scalar(project::CHROMA_KEY_INTENSITY_DEFAULT),
                )
                .unwrap()
                .with("shadow", EffectValue::Scalar(0.0))
                .unwrap(),
        })
        .unwrap();
}

#[test]
fn remove_background_tab_stages_chroma_key_without_touching_the_project() {
    let fixture = fixture();
    let (video, image) = (fixture.video, fixture.image);
    let mut harness = build(fixture.editor);
    harness.run_steps(3);
    for item in [fixture.graphic, fixture.audio] {
        select(&mut harness, item);
        assert!(
            !has_widget(&harness, "inspector.tab.remove_background"),
            "{item:?}"
        );
    }
    for item in [video, image] {
        select(&mut harness, item);
        assert!(has_widget(&harness, "inspector.tab.remove_background"));
        click(&mut harness, "inspector.tab.remove_background");
        assert!(has_widget(&harness, "inspector.chroma_key.toggle"));
        assert!(
            !has_widget(&harness, "effects.chroma_key.param.intensity"),
            "controls stay hidden until the key is on"
        );
        assert!(chroma(&harness, item).is_none(), "viewing adds nothing");
    }

    // The reference reveals the eyedropper and reset before a colour is picked;
    // switching on stages the panel and never mutates the project.
    open_remove_background(&mut harness, video);
    let history = harness.state().session.history_len();
    click(&mut harness, "inspector.chroma_key.toggle");
    assert!(chroma(&harness, video).is_none(), "on adds no effect");
    assert_eq!(harness.state().session.history_len(), history);
    assert!(has_widget(&harness, "inspector.chroma_key.eyedropper"));
    assert!(has_widget(&harness, "inspector.chroma_key.reset"));
    for id in [
        "inspector.chroma_key.color",
        "effects.chroma_key.param.intensity",
        "effects.chroma_key.param.shadow",
        "effects.chroma_key.param.intensity.keyframe",
    ] {
        assert!(!has_widget(&harness, id), "{id} waits for the first pick");
    }

    // A staged switch turns off without a command; no effect existed to remove.
    click(&mut harness, "inspector.chroma_key.toggle");
    assert!(chroma(&harness, video).is_none());
    assert_eq!(harness.state().session.history_len(), history);
    assert!(!has_widget(&harness, "inspector.chroma_key.eyedropper"));
}

#[test]
fn chroma_key_colour_field_writes_the_packed_key_colour_once() {
    let fixture = fixture();
    let video = fixture.video;
    let mut editor = fixture.editor;
    add_chroma_effect(&mut editor.session, video);
    let mut harness = build(editor);
    harness.run_steps(3);
    open_remove_background(&mut harness, video);
    let history = harness.state().session.history_len();
    click(&mut harness, "inspector.chroma_key.color");
    harness.run_steps(3);
    type_value(&mut harness, "color.hex", "#FF8000");
    assert_eq!(
        chroma_parts(&harness, video).unwrap().0,
        Color([255, 128, 0, 255])
    );
    assert_eq!(harness.state().session.history_len().0, history.0 + 1);
    harness.state_mut().activate("edit.undo").unwrap();
    harness.run_steps(2);
    assert_eq!(
        chroma_parts(&harness, video).unwrap().0,
        Color([0, 255, 0, 255])
    );
}

#[test]
fn chroma_key_controls_clamp_animate_and_undo_once_per_gesture() {
    let fixture = fixture();
    let video = fixture.video;
    let mut editor = fixture.editor;
    add_chroma_effect(&mut editor.session, video);
    let mut harness = build(editor);
    harness.run_steps(3);
    open_remove_background(&mut harness, video);
    let project = harness.state().session.project().clone();
    harness
        .state_mut()
        .playback
        .seek(Tick(SECOND), false, project)
        .unwrap();
    harness.run_steps(2);

    let history = harness.state().session.history_len();
    type_value(&mut harness, "effects.chroma_key.param.intensity", "999");
    assert_eq!(chroma_parts(&harness, video).unwrap().1, 100.0);
    assert_eq!(harness.state().session.history_len().0, history.0 + 1);
    type_value(&mut harness, "effects.chroma_key.param.shadow", "999");
    assert_eq!(chroma_parts(&harness, video).unwrap().2, 100.0);
    let history = harness.state().session.history_len();
    type_value(&mut harness, "effects.chroma_key.param.shadow", "NaN");
    assert_eq!(
        harness.state().session.history_len(),
        history,
        "NaN refused"
    );

    // One diamond adds or removes the key at the playhead: one undo each.
    let path = "effects.chroma_key.params.intensity";
    let history = harness.state().session.history_len();
    click(&mut harness, "effects.chroma_key.param.intensity.keyframe");
    assert!(has_key(&harness, video, path, Tick(SECOND)));
    assert_eq!(
        channel_value(&harness, video, path, Tick(SECOND)),
        Some(100.0)
    );
    assert_eq!(harness.state().session.history_len().0, history.0 + 1);
    click(&mut harness, "effects.chroma_key.param.intensity.keyframe");
    assert!(!has_key(&harness, video, path, Tick(SECOND)));
    assert_eq!(harness.state().session.history_len().0, history.0 + 2);
    click(&mut harness, "effects.chroma_key.param.intensity.keyframe");

    // A keyed channel writes the key at the playhead, not the base value.
    let start = widget_rect(&harness, "effects.chroma_key.param.intensity.scrub").center();
    drag_to(
        &mut harness,
        "effects.chroma_key.param.intensity.scrub",
        egui::vec2(-40.0, 0.0),
    );
    assert!(harness.state().session.gesture_active());
    harness.drop_at(start - egui::vec2(40.0, 0.0));
    harness.run_steps(2);
    assert_eq!(
        channel_value(&harness, video, path, Tick(SECOND)),
        Some(80.0)
    );
    assert_eq!(chroma_parts(&harness, video).unwrap().1, 100.0, "base kept");
    harness.state_mut().activate("edit.undo").unwrap();
    harness.run_steps(2);
    assert_eq!(
        channel_value(&harness, video, path, Tick(SECOND)),
        Some(100.0)
    );

    // A reversal publishes nothing.
    let history = harness.state().session.history_len();
    let start = widget_rect(&harness, "effects.chroma_key.param.intensity.scrub").center();
    drag_to(
        &mut harness,
        "effects.chroma_key.param.intensity.scrub",
        egui::vec2(-40.0, 0.0),
    );
    harness.hover_at(start);
    harness.run_steps(2);
    harness.drop_at(start);
    harness.run_steps(2);
    assert_eq!(
        channel_value(&harness, video, path, Tick(SECOND)),
        Some(100.0)
    );
    assert_eq!(harness.state().session.history_len(), history);

    // Outside the clip the diamonds refuse; the base value still edits.
    let project = harness.state().session.project().clone();
    harness
        .state_mut()
        .playback
        .seek(Tick(3 * SECOND), false, project)
        .unwrap();
    harness.run_steps(2);
    let history = harness.state().session.history_len();
    click(&mut harness, "effects.chroma_key.param.intensity.keyframe");
    assert!(!has_key(&harness, video, path, Tick(3 * SECOND)));
    assert_eq!(harness.state().session.history_len(), history);

    // Off removes the effect in one undo step; undo restores it.
    let history = harness.state().session.history_len();
    click(&mut harness, "inspector.chroma_key.toggle");
    assert!(chroma(&harness, video).is_none());
    assert_eq!(harness.state().session.history_len().0, history.0 + 1);
    assert!(!has_widget(&harness, "inspector.chroma_key.eyedropper"));
    harness.state_mut().activate("edit.undo").unwrap();
    harness.run_steps(2);
    assert!(chroma(&harness, video).is_some());
    assert!(has_widget(&harness, "inspector.chroma_key.eyedropper"));
}

/// A video clip after a solid overlay: the first rendered frame is the overlay,
/// so the eyedropper samples a real preview frame without decoding the clip.
fn chroma_frame_fixture() -> (Editor, ItemId) {
    let mut session =
        Session::new(Project::empty(ProjectId(9), "Chroma".into()).unwrap(), 200).unwrap();
    session
        .apply(Command::AddAsset {
            asset: asset(
                1,
                vec![
                    stream(media::StreamKind::Video, None),
                    stream(media::StreamKind::Audio, None),
                ],
            ),
        })
        .unwrap();
    let main = session.project().data().scene().timeline.main.id;
    insert(
        &mut session,
        main,
        Tick::ZERO,
        Tick(2 * SECOND),
        None,
        ItemParams::Overlay(project::OverlayParams {
            color: Color([10, 20, 30, 255]),
        }),
    );
    let video = insert(
        &mut session,
        main,
        Tick(2 * SECOND),
        Tick(2 * SECOND),
        Some(AssetId(1)),
        ItemParams::Video(project::VideoParams::plain()),
    );
    let mut playback = Playback::new(
        &session,
        ByteBudget::new(64 << 20).unwrap(),
        Arc::new(Files),
    )
    .unwrap();
    playback.set_device_available(true);
    (
        Editor::new(session, playback, Preferences::default()),
        video,
    )
}

#[test]
fn chroma_key_eyedropper_samples_the_rendered_preview_frame() {
    let (editor, video) = chroma_frame_fixture();
    let mut harness = build(editor);
    harness.run_steps(3);
    open_remove_background(&mut harness, video);
    click(&mut harness, "inspector.chroma_key.toggle");
    assert!(chroma(&harness, video).is_none(), "on stages, adds nothing");
    assert!(has_widget(&harness, "inspector.chroma_key.eyedropper"));
    assert!(!has_widget(&harness, "inspector.chroma_key.color"));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while harness.state().playback.frame().is_none() {
        assert!(
            std::time::Instant::now() < deadline,
            "preview frame did not arrive"
        );
        harness.run_steps(1);
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    let stage = widget_rect(&harness, "preview.canvas");
    let canvas = {
        let canvas = &harness.state().session.project().data().canvas;
        [canvas.width, canvas.height]
    };
    let scale = vibecut::services::preview::fit_scale(stage.size(), canvas);
    let displayed = egui::Rect::from_center_size(
        stage.center(),
        egui::vec2(canvas[0] as f32, canvas[1] as f32) * scale,
    );
    let pos = displayed.center();
    let expected = vibecut::services::preview::sample_color(
        &harness.state().playback.frame().unwrap().image,
        displayed,
        pos,
    )
    .expect("the clicked pixel is inside the frame");
    assert_ne!(expected, Color([0, 255, 0, 255]), "not the default key");

    let history = harness.state().session.history_len();
    click(&mut harness, "inspector.chroma_key.eyedropper");
    assert_eq!(
        harness.state().inspect().unwrap().key_color_pick,
        Some(video)
    );
    click_pos(&mut harness, pos);
    assert_eq!(
        chroma_parts(&harness, video),
        Some((expected, project::CHROMA_KEY_INTENSITY_DEFAULT, 0.0, true)),
        "the first pick adds chroma_key with the picked colour and catalog defaults"
    );
    assert_eq!(harness.state().session.history_len().0, history.0 + 1);
    assert_eq!(
        harness.state().inspect().unwrap().key_color_pick,
        None,
        "the pick disarms"
    );
    for id in [
        "inspector.chroma_key.color",
        "effects.chroma_key.param.intensity",
        "effects.chroma_key.param.shadow",
        "effects.chroma_key.param.intensity.keyframe",
        "effects.chroma_key.param.shadow.keyframe",
    ] {
        assert!(has_widget(&harness, id), "{id} appears after the pick");
    }

    // Reset restores the catalog default in one step.
    let history = harness.state().session.history_len();
    click(&mut harness, "inspector.chroma_key.reset");
    assert_eq!(
        chroma_parts(&harness, video).unwrap().0,
        Color([0, 255, 0, 255])
    );
    assert_eq!(harness.state().session.history_len().0, history.0 + 1);

    // Escape leaves the eyedropper without a pick; a click outside the frame
    // does nothing and keeps it armed.
    let history = harness.state().session.history_len();
    click(&mut harness, "inspector.chroma_key.eyedropper");
    assert_eq!(
        harness.state().inspect().unwrap().key_color_pick,
        Some(video)
    );
    harness.key_press(egui::Key::Escape);
    harness.run_steps(2);
    assert_eq!(
        harness.state().inspect().unwrap().key_color_pick,
        None,
        "Escape disarms"
    );
    assert_eq!(
        chroma_parts(&harness, video).unwrap().0,
        Color([0, 255, 0, 255])
    );

    click(&mut harness, "inspector.chroma_key.eyedropper");
    assert_eq!(
        harness.state().inspect().unwrap().key_color_pick,
        Some(video)
    );
    click(&mut harness, "inspector.chroma_key.eyedropper");
    assert_eq!(
        harness.state().inspect().unwrap().key_color_pick,
        None,
        "the button leaves the mode without a pick"
    );
    assert_eq!(
        chroma_parts(&harness, video).unwrap().0,
        Color([0, 255, 0, 255])
    );
    assert_eq!(harness.state().session.history_len(), history);
}

#[test]
fn remove_background_first_pick_refuses_a_locked_track() {
    let (mut editor, video) = chroma_frame_fixture();
    let main = editor.session.project().data().scene().timeline.main.id;
    editor
        .session
        .apply(Command::SetTrackFlags {
            track: main,
            muted: false,
            hidden: false,
            locked: true,
        })
        .unwrap();
    let mut harness = build(editor);
    harness.run_steps(3);
    open_remove_background(&mut harness, video);
    click(&mut harness, "inspector.chroma_key.toggle");
    assert!(has_widget(&harness, "inspector.chroma_key.eyedropper"));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while harness.state().playback.frame().is_none() {
        assert!(
            std::time::Instant::now() < deadline,
            "preview frame did not arrive"
        );
        harness.run_steps(1);
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    let history = harness.state().session.history_len();
    click(&mut harness, "inspector.chroma_key.eyedropper");
    let pos = widget_rect(&harness, "preview.canvas").center();
    click_pos(&mut harness, pos);
    assert!(
        chroma(&harness, video).is_none(),
        "the locked track refuses"
    );
    assert_eq!(harness.state().session.history_len(), history);
    assert_eq!(harness.state().inspect().unwrap().key_color_pick, None);
    assert!(
        has_widget(&harness, "inspector.chroma_key.eyedropper"),
        "the staged switch keeps the eyedropper for a retry"
    );
}

fn removal(harness: &Harness<Editor>, id: ItemId) -> Option<project::BackgroundRemoval> {
    harness
        .state()
        .session
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| &track.items)
        .find(|item| item.id == id)
        .and_then(|item| match &item.content {
            ItemParams::Video(params) => params.background_removal,
            _ => None,
        })
}

/// The fixture editor over a fresh process media cache without a person segmenter, as a
/// workspace whose matte weights are not installed.
fn removal_editor(root: &std::path::Path) -> (Editor, ItemId) {
    let fixture = fixture();
    let mut editor = fixture.editor;
    let media =
        engine::control::MediaAccess::open_in(root, ByteBudget::new(64 << 20).unwrap()).unwrap();
    editor.set_media(Arc::new(std::sync::Mutex::new(media)));
    (editor, fixture.video)
}

#[test]
fn auto_removal_enables_tunes_and_disables_with_one_undo_step_each() {
    let root = tempfile::tempdir().unwrap();
    let (editor, video) = removal_editor(root.path());
    let mut harness = build(editor);
    harness.run_steps(3);
    open_remove_background(&mut harness, video);
    // The reference lists Auto removal above Chroma key; off shows the switch alone.
    assert!(has_widget(&harness, "inspector.auto_removal"));
    assert!(has_widget(&harness, "inspector.chroma_key.toggle"));
    assert!(
        widget_rect(&harness, "inspector.auto_removal").top()
            < widget_rect(&harness, "inspector.chroma_key.toggle").top()
    );
    assert!(!has_widget(
        &harness,
        "inspector.background_removal_feather"
    ));
    assert_eq!(removal(&harness, video), None);

    // Enable: one click switches it on with the model's own edge and starts the analysis.
    let history = harness.state().session.history_len();
    click(&mut harness, "inspector.auto_removal");
    harness.run_steps(3);
    assert_eq!(
        removal(&harness, video),
        Some(project::BackgroundRemoval { feather: 0.0 })
    );
    assert_eq!(harness.state().session.history_len().0, history.0 + 1);
    assert!(has_widget(&harness, "inspector.background_removal_feather"));
    // No weights are installed: the row links to the model manager and keeps a retry; the clip
    // renders unmatted meanwhile.
    assert!(has_widget(&harness, "inspector.auto_removal.models"));
    assert!(has_widget(&harness, "inspector.auto_removal.analyze"));
    assert!(!has_widget(&harness, "inspector.auto_removal.progress"));

    // Tune: a typed feather is one undo step and needs no new analysis.
    type_value(&mut harness, "inspector.background_removal_feather", "40");
    assert_eq!(
        removal(&harness, video),
        Some(project::BackgroundRemoval { feather: 40.0 })
    );
    assert_eq!(harness.state().session.history_len().0, history.0 + 2);
    harness.state_mut().activate("edit.undo").unwrap();
    harness.run_steps(2);
    assert_eq!(
        removal(&harness, video),
        Some(project::BackgroundRemoval { feather: 0.0 })
    );

    // Disable: off removes the switch's rows, one undo step.
    click(&mut harness, "inspector.auto_removal");
    harness.run_steps(3);
    assert_eq!(removal(&harness, video), None);
    assert!(!has_widget(
        &harness,
        "inspector.background_removal_feather"
    ));
    assert!(!has_widget(&harness, "inspector.auto_removal.models"));
    harness.state_mut().activate("edit.undo").unwrap();
    harness.run_steps(3);
    assert_eq!(
        removal(&harness, video),
        Some(project::BackgroundRemoval { feather: 0.0 })
    );
}

/// Offscreen screenshot evidence; wgpu rendering stays out of CI.
#[test]
#[ignore = "renders through wgpu; run with --ignored to refresh target/p2-ai-01-bg-removal.png"]
fn remove_background_panel_renders_the_auto_removal_controls() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let target = root.join("target");
    std::fs::create_dir_all(&target).unwrap();
    let media = tempfile::tempdir().unwrap();
    let (editor, video) = removal_editor(media.path());
    let mut harness = build(editor);
    harness.run_steps(3);
    open_remove_background(&mut harness, video);
    click(&mut harness, "inspector.auto_removal");
    harness.run_steps(3);
    type_value(&mut harness, "inspector.background_removal_feather", "25");
    let image = harness.render().expect("offscreen render");
    image.save(target.join("p2-ai-01-bg-removal.png")).unwrap();
    assert!(image.width() > 0 && image.height() > 0);
}

/// Offscreen screenshot evidence; wgpu rendering stays out of CI.
#[test]
#[ignore = "renders through wgpu; run with --ignored to refresh target/p1-180-chroma.png"]
fn remove_background_panel_renders_the_chroma_key_controls() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let target = root.join("target");
    std::fs::create_dir_all(&target).unwrap();
    let fixture = fixture();
    let video = fixture.video;
    let mut editor = fixture.editor;
    add_chroma_effect(&mut editor.session, video);
    let mut harness = build(editor);
    harness.run_steps(3);
    open_remove_background(&mut harness, video);
    type_value(&mut harness, "effects.chroma_key.param.intensity", "60");
    type_value(&mut harness, "effects.chroma_key.param.shadow", "25");
    let image = harness.render().expect("offscreen render");
    image.save(target.join("p1-180-chroma.png")).unwrap();
    assert!(image.width() > 0 && image.height() > 0);
}

/// Offscreen screenshot evidence; wgpu rendering stays out of CI.
#[test]
#[ignore = "renders through wgpu; run with --ignored to refresh target/p1-68-adjust.png"]
fn adjust_panel_renders_the_grade_controls() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let target = root.join("target");
    std::fs::create_dir_all(&target).unwrap();
    let fixture = fixture();
    let video = fixture.video;
    let mut harness = build(fixture.editor);
    harness.run_steps(3);
    select(&mut harness, video);
    click(&mut harness, "inspector.tab.adjust");
    type_value(&mut harness, "effects.color_grade.param.brightness", "20");
    type_value(&mut harness, "effects.color_grade.param.saturation", "-30");
    let image = harness.render().expect("offscreen render");
    image.save(target.join("p1-68-adjust.png")).unwrap();
    assert!(image.width() > 0 && image.height() > 0);
}

/// Offscreen screenshot evidence; wgpu rendering stays out of CI.
#[test]
#[ignore = "renders through wgpu; run with --ignored to refresh target/p1-87-colour-curves.png"]
fn colour_wheels_and_curves_panel_renders() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let target = root.join("target");
    std::fs::create_dir_all(&target).unwrap();
    let fixture = fixture();
    let video = fixture.video;
    let mut harness = crate::gpu::builder()
        .with_size(egui::vec2(1727.0, 2400.0))
        .build_ui_state(|ui, editor: &mut Editor| editor.ui(ui), fixture.editor);
    harness.run_steps(3);
    select(&mut harness, video);
    click(&mut harness, "inspector.tab.adjust");
    type_value(&mut harness, "effects.color_grade.param.brightness", "15");
    let start = widget_rect(&harness, "effects.color_grade.wheel.lift").center();
    drag_from(&mut harness, start, egui::vec2(20.0, -12.0));
    harness.drop_at(start + egui::vec2(20.0, -12.0));
    harness.run_steps(2);
    let plot = widget_rect(&harness, "effects.color_grade.curve.plot");
    click_pos(&mut harness, curve_pos(plot, 0.5, 0.53));
    harness.run_steps(3);
    let image = harness.render().expect("offscreen render");
    image.save(target.join("p1-87-colour-curves.png")).unwrap();
    assert!(image.width() > 0 && image.height() > 0);
}

/// Offscreen screenshot evidence; wgpu rendering stays out of CI.
#[test]
#[ignore = "renders through wgpu; run with --ignored to refresh target/p1-33-effects.png"]
fn effects_panel_renders_tiles_stack_and_preview() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let target = root.join("target");
    std::fs::create_dir_all(&target).unwrap();
    for (scale, filename) in [(1.0, "p1-33-effects.png"), (2.0, "p1-33-effects-2x.png")] {
        let fixture = fixture();
        let video = fixture.video;
        let mut editor = fixture.editor;
        add_effect(&mut editor.session, video, "blur", true);
        add_effect(&mut editor.session, video, "blur2", false);
        let mut harness = crate::gpu::builder()
            .with_size(egui::vec2(1440.0, 900.0))
            .with_pixels_per_point(scale)
            .build_ui_state(|ui, editor: &mut Editor| editor.ui(ui), editor);
        harness.run_steps(3);
        open_effect_tabs(&mut harness, video);
        wait_preview(&mut harness, "blur", EffectPreviewStatus::Ready);
        let image = harness.render().expect("offscreen render");
        image.save(target.join(filename)).unwrap();
        assert!(image.width() > 0 && image.height() > 0);
    }
}

#[test]
fn discrete_effect_controls_commit_once_and_restore_with_undo_redo() {
    for (kind, key, choices) in [
        ("invert", "channel", true),
        ("color_pass", "reverse", false),
        ("color_replace", "solid", false),
        ("extract", "invert", false),
    ] {
        let f = fixture();
        let video = f.video;
        let mut editor = f.editor;
        if choices {
            editor.preferences.locale = vibecut::i18n::Locale::Vi;
        }
        editor
            .session
            .apply(Command::AddEffect {
                item: video,
                effect: project::effect_definition(kind)
                    .unwrap()
                    .instance("discrete"),
            })
            .unwrap();
        add_effect(&mut editor.session, video, "untouched", false);
        let mut harness = build(editor);
        harness.run_steps(3);
        open_effect_tabs(&mut harness, video);
        let id = format!("effects.discrete.param.{key}");
        assert!(has_widget(&harness, &id), "missing {kind}.{key}");
        assert!(!has_widget(&harness, &format!("{id}.keyframe")));
        assert!(has_widget(
            &harness,
            "effects.untouched.param.intensity.keyframe"
        ));
        let values: Vec<_> = if choices {
            [1, 2, 3, 4, 0, 0]
                .into_iter()
                .map(EffectValue::Choice)
                .collect()
        } else {
            vec![EffectValue::Bool(true), EffectValue::Bool(false)]
        };
        for value in values {
            let before = item(&harness, video);
            let history = harness.state().session.history_len().0;
            if let EffectValue::Choice(index) = value {
                click(&mut harness, &id);
                let option = format!("{id}.{index}");
                let label = ["RGB", "Đỏ", "Xanh lá", "Xanh dương", "Độ trong suốt"][index as usize];
                assert_eq!(
                    harness
                        .get(By::new().predicate(|n| n.author_id() == Some(option.as_str())))
                        .accesskit_node()
                        .label(),
                    Some(label.to_owned())
                );
                click(&mut harness, &option);
            } else {
                click(&mut harness, &id);
            }
            let mut expected = before.clone();
            expected.effects[0].params.insert(key.into(), value);
            assert_eq!(item(&harness, video), expected, "{kind}.{key} {value:?}");
            let changed = expected != before;
            assert_eq!(
                harness.state().session.history_len().0,
                history + usize::from(changed)
            );
            if changed {
                harness.state_mut().activate("edit.undo").unwrap();
                harness.run_steps(2);
                assert_eq!(item(&harness, video), before);
                match before.effects[0].value(key).unwrap() {
                    EffectValue::Bool(on) => assert_eq!(
                        harness
                            .get(By::new().predicate(|n| n.author_id() == Some(id.as_str())))
                            .accesskit_node()
                            .toggled(),
                        Some(if *on {
                            egui::accesskit::Toggled::True
                        } else {
                            egui::accesskit::Toggled::False
                        })
                    ),
                    EffectValue::Choice(index) => {
                        click(&mut harness, &id);
                        assert_eq!(
                            harness
                                .get(By::new().predicate(
                                    |n| n.author_id() == Some(format!("{id}.{index}").as_str())
                                ))
                                .accesskit_node()
                                .toggled(),
                            Some(egui::accesskit::Toggled::True)
                        );
                        harness.key_press(egui::Key::Escape);
                        harness.run_steps(2);
                    }
                    _ => panic!("unknown case"),
                }
                harness.state_mut().activate("edit.redo").unwrap();
                harness.run_steps(2);
                assert_eq!(item(&harness, video), expected);
            }
            if let EffectValue::Bool(on) = value {
                assert_eq!(
                    harness
                        .get(By::new().predicate(|n| n.author_id() == Some(id.as_str())))
                        .accesskit_node()
                        .toggled(),
                    Some(if on {
                        egui::accesskit::Toggled::True
                    } else {
                        egui::accesskit::Toggled::False
                    })
                );
            } else if let EffectValue::Choice(index) = value {
                click(&mut harness, &id);
                assert_eq!(
                    harness
                        .get(
                            By::new().predicate(
                                |n| n.author_id() == Some(format!("{id}.{index}").as_str())
                            )
                        )
                        .accesskit_node()
                        .toggled(),
                    Some(egui::accesskit::Toggled::True)
                );
                harness.key_press(egui::Key::Escape);
                harness.run_steps(2);
            }
            click(&mut harness, "inspector.group.basic");
            click(&mut harness, "inspector.tab.effects");
            assert_eq!(item(&harness, video), expected);
        }
        harness
            .state_mut()
            .session
            .apply(Command::SetTrackLocked {
                track: f.main,
                locked: true,
            })
            .unwrap();
        harness.run_steps(3);
        let before = harness.state().session.project().clone();
        let history = harness.state().session.history_len();
        assert!(
            harness
                .get(By::new().predicate(|n| n.author_id() == Some(id.as_str())))
                .accesskit_node()
                .is_disabled()
        );
        click(&mut harness, &id);
        assert_eq!(harness.state().session.project(), &before);
        assert_eq!(harness.state().session.history_len(), history);
    }
}

#[test]
fn discrete_effect_popups_follow_selection_removal_and_locked_tracks() {
    let f = fixture();
    let (video, image, main) = (f.video, f.image, f.main);
    let mut editor = f.editor;
    for item in [video, image] {
        editor
            .session
            .apply(Command::AddEffect {
                item,
                effect: project::effect_definition("invert")
                    .unwrap()
                    .instance("same"),
            })
            .unwrap();
    }
    editor
        .preferences
        .properties_tabs
        .insert("image".into(), "effects".into());
    let mut harness = build(editor);
    harness.run_steps(3);
    open_effect_tabs(&mut harness, video);
    let id = "effects.same.param.channel";
    click(&mut harness, id);
    assert!(has_widget(&harness, &format!("{id}.4")));
    let before = item(&harness, video);
    select(&mut harness, image);
    assert!(
        !has_widget(&harness, &format!("{id}.4")),
        "old item's popup is absent"
    );
    click(&mut harness, id);
    click(&mut harness, &format!("{id}.4"));
    assert_eq!(item(&harness, video), before);
    assert_eq!(
        effects(&harness, image)[0].value("channel"),
        Some(&EffectValue::Choice(4))
    );
    click(&mut harness, id);
    click(&mut harness, "inspector.group.basic");
    assert!(!has_widget(&harness, &format!("{id}.4")));
    click(&mut harness, "inspector.tab.effects");
    click(&mut harness, id);
    harness
        .state_mut()
        .session
        .apply(Command::RemoveEffect {
            item: image,
            effect: "same".into(),
        })
        .unwrap();
    harness.run_steps(3);
    assert!(!has_widget(&harness, &format!("{id}.4")));
    select(&mut harness, video);
    harness
        .state_mut()
        .session
        .apply(Command::SetTrackLocked {
            track: main,
            locked: true,
        })
        .unwrap();
    harness.run_steps(3);
    let before = harness.state().session.project().clone();
    let history = harness.state().session.history_len();
    assert!(
        harness
            .get(By::new().predicate(|n| n.author_id() == Some(id)))
            .accesskit_node()
            .is_disabled()
    );
    click(&mut harness, id);
    assert_eq!(harness.state().session.project(), &before);
    assert_eq!(harness.state().session.history_len(), history);
}

#[test]
fn effect_rgb_pickers_edit_all_six_authoritative_bases_once() {
    use project::{AnimationPath, ParamValue};
    for (kind, key) in [
        ("color_pass", "color"),
        ("color_replace", "target"),
        ("color_replace", "replace"),
        ("tint", "black"),
        ("tint", "white"),
        ("strobe", "color"),
    ] {
        let f = fixture();
        let video = f.video;
        let mut editor = f.editor;
        if matches!(kind, "tint" | "strobe") {
            editor.preferences.locale = vibecut::i18n::Locale::Vi;
        }
        editor
            .session
            .apply(Command::AddEffect {
                item: video,
                effect: project::effect_definition(kind).unwrap().instance("rgb"),
            })
            .unwrap();
        add_effect(&mut editor.session, video, "untouched", false);
        let mut h = build(editor);
        h.run_steps(3);
        open_effect_tabs(&mut h, video);
        let id = format!("effects.rgb.param.{key}");
        assert!(has_widget(&h, &id), "{kind}.{key}");
        let path = AnimationPath::new(format!("effects.rgb.params.{key}")).unwrap();
        let before = h.state().session.project().clone();
        let history = h.state().session.history_len().0;
        click(&mut h, &id);
        assert!(!has_widget(&h, "color.a"));
        type_value(&mut h, "color.hex", "#8000FF");
        assert_eq!(
            h.state().session.history_len().0,
            history + 1,
            "{kind}.{key}"
        );
        let edited = item(&h, video);
        assert_eq!(
            edited.effects,
            before.data().scene().timeline.main.items[0].effects,
            "static storage stays fallback"
        );
        let Some(ParamValue::Color(rgba)) = edited.animations.value_at(&path, Tick::ZERO).unwrap()
        else {
            panic!("missing authoritative Color");
        };
        for (actual, expected) in rgba.into_iter().zip([0.2158605, 0.0, 1.0, 1.0]) {
            assert!((actual - expected).abs() < 1e-6);
        }
        assert_eq!(
            render::color::at(&edited.animations, &path, Tick::ZERO, Color([0; 4])).unwrap(),
            Color([128, 0, 255, 255])
        );
        let after = h.state().session.project().clone();
        type_value(&mut h, "color.hex", "#8000FF");
        assert_eq!(h.state().session.history_len().0, history + 1, "no-op");
        h.state_mut().activate("edit.undo").unwrap();
        h.run_steps(2);
        assert_eq!(h.state().session.project(), &before);
        h.state_mut().activate("edit.redo").unwrap();
        h.run_steps(2);
        assert_eq!(h.state().session.project(), &after);
    }
}

#[test]
fn effect_rgb_keys_use_real_time_preserve_curves_and_all_alpha() {
    use project::{
        AnimationPath, ColorComponent as C, CurveHandle, Extrapolation, Keyframe, Param,
        ParamValue as V, Segment,
    };
    let f = fixture();
    let video = f.video;
    let mut editor = f.editor;
    editor
        .session
        .apply(Command::AddEffect {
            item: video,
            effect: project::effect_definition("strobe")
                .unwrap()
                .instance("rgb")
                .with("duration", EffectValue::Scalar(30.0))
                .unwrap(),
        })
        .unwrap();
    editor
        .session
        .apply(Command::SetSpeed {
            items: vec![video],
            speed: time::Speed::from_f64(0.5).unwrap(),
            maintain_pitch: false,
        })
        .unwrap();
    let path = AnimationPath::new("effects.rgb.params.color").unwrap();
    let mut param = Param::new(V::Color([0.2, 0.3, 0.4, 0.75])).unwrap();
    for c in [C::R, C::G] {
        for (t, value) in [(0, 0.123456789), (SECOND, 0.5), (2 * SECOND, 0.75)] {
            let mut key = Keyframe::new(Tick(t), V::Scalar(value));
            key.segment = Segment::Bezier;
            key.incoming = Some(CurveHandle {
                dt: Tick(-SECOND / 4),
                dv: -0.02,
            });
            key.outgoing = Some(CurveHandle {
                dt: Tick(SECOND / 4),
                dv: 0.03,
            });
            param.channel_mut(Some(c)).unwrap().put(key).unwrap();
        }
        param
            .channel_mut(Some(c))
            .unwrap()
            .set_extrapolation(Extrapolation::Linear, Extrapolation::Hold)
            .unwrap();
    }
    param
        .channel_mut(Some(C::A))
        .unwrap()
        .put(Keyframe::new(Tick(SECOND), V::Scalar(0.123456789)))
        .unwrap();
    editor
        .session
        .apply(Command::SetAnimatedParam {
            item: video,
            path: path.clone(),
            param: param.clone(),
        })
        .unwrap();
    let mut h = build(editor);
    h.run_steps(3);
    open_effect_tabs(&mut h, video);
    let project = h.state().session.project().clone();
    h.state_mut()
        .playback
        .seek(Tick(SECOND), false, project)
        .unwrap();
    h.run_steps(2);
    assert_eq!(
        item(&h, video).source_time_at(Tick(SECOND)).unwrap(),
        Tick(SECOND / 2)
    );
    click(&mut h, "effects.rgb.param.color");
    assert_eq!(
        h.state().inspect().unwrap().color_preview,
        Some(Color([188, 188, 170, 255]))
    );
    let before = h.state().session.project().clone();
    let history = h.state().session.history_len().0;
    type_value(&mut h, "color.hex", "#FF00FF");
    assert_eq!(h.state().session.history_len().0, history + 1);
    let edited = item(&h, video);
    let updated = edited.animations.get(&path).unwrap();
    assert_eq!(
        updated.channel(Some(C::A)).unwrap(),
        param.channel(Some(C::A)).unwrap()
    );
    for c in [C::R, C::G] {
        let original = param.channel(Some(c)).unwrap();
        let current = updated.channel(Some(c)).unwrap();
        assert_eq!(original.base(), current.base());
        assert_eq!(original.extrapolation(), current.extrapolation());
        for (a, b) in original.keys().iter().zip(current.keys()) {
            if a.time != Tick(SECOND) {
                assert_eq!(a, b);
            } else {
                let mut a = a.clone();
                a.value = b.value.clone();
                assert_eq!(&a, b);
            }
        }
    }
    assert_eq!(
        updated.value_at(Tick(SECOND)).unwrap(),
        V::Color([1.0, 0.0, 1.0, 0.123456789])
    );
    let budget = ByteBudget::new(1 << 20).unwrap();
    let hooks = engine::ByteBudgetHooks::new(&budget, None);
    let image = render::Image::new(1, 1, [0.125, 0.125, 0.125, 0.25], &hooks).unwrap();
    let effect = edited.effects.iter().find(|e| e.id == "rgb").unwrap();
    let passes = render::effects::evaluate_effect(
        &edited.animations,
        effect,
        Tick(SECOND),
        EffectGeometry::new([1, 1], [1.0, 1.0]).unwrap(),
        true,
        &render::lut::LutResources::default(),
    )
    .unwrap();
    use render::effects::EffectCompositor;
    let out = render::effects::CpuEffects
        .apply(
            &image,
            &passes,
            render::effects::EffectInputs {
                input: &image,
                below: &image,
                coverage: None,
            },
            &hooks,
        )
        .unwrap();
    for (a, b) in out.pixels()[0].into_iter().zip([0.25, 0.0, 0.25, 0.25]) {
        assert!((a - b).abs() < 3e-6);
    }
    h.state_mut().activate("edit.undo").unwrap();
    h.run_steps(2);
    assert_eq!(h.state().session.project(), &before);
    h.state_mut().activate("edit.redo").unwrap();
    h.run_steps(2);
    let keyed = h.state().session.project().clone();
    click(&mut h, "effects.rgb.param.color.keyframe");
    let removed = item(&h, video);
    let p = removed.animations.get(&path).unwrap();
    for c in [C::R, C::G, C::B] {
        assert!(
            !p.channel(Some(c))
                .unwrap()
                .keys()
                .iter()
                .any(|k| k.time == Tick(SECOND))
        );
    }
    assert_eq!(
        p.channel(Some(C::A)).unwrap(),
        param.channel(Some(C::A)).unwrap()
    );
    h.state_mut().activate("edit.undo").unwrap();
    h.run_steps(2);
    assert_eq!(h.state().session.project(), &keyed);
    h.state_mut().activate("edit.redo").unwrap();
    h.run_steps(2);
    // A fresh tick adds all RGB components as ONE composite edit, retaining alpha.
    let project = h.state().session.project().clone();
    h.state_mut()
        .playback
        .seek(Tick(3 * SECOND), false, project)
        .unwrap();
    h.run_steps(2);
    let before = h.state().session.project().clone();
    let history = h.state().session.history_len().0;
    click(&mut h, "effects.rgb.param.color.keyframe");
    assert_eq!(h.state().session.history_len().0, history + 1);
    let added = item(&h, video);
    let p = added.animations.get(&path).unwrap();
    for c in [C::R, C::G, C::B] {
        assert!(
            p.channel(Some(c))
                .unwrap()
                .keys()
                .iter()
                .any(|k| k.time == Tick(3 * SECOND))
        );
    }
    assert_eq!(
        p.channel(Some(C::A)).unwrap(),
        param.channel(Some(C::A)).unwrap()
    );
    h.state_mut().activate("edit.undo").unwrap();
    h.run_steps(2);
    assert_eq!(h.state().session.project(), &before);
}

#[test]
fn effect_rgb_zero_key_base_survives_static_changes_and_last_key_removal() {
    use project::{AnimationPath, ColorComponent as C, Param, ParamValue as V};
    let f = fixture();
    let video = f.video;
    let mut editor = f.editor;
    editor
        .session
        .apply(Command::AddEffect {
            item: video,
            effect: project::effect_definition("strobe")
                .unwrap()
                .instance("rgb"),
        })
        .unwrap();
    let path = AnimationPath::new("effects.rgb.params.color").unwrap();
    let mut base = Param::new(V::Color([0.123456789, 0.25, 0.5, 0.314159265])).unwrap();
    base.channel_mut(Some(C::A))
        .unwrap()
        .put(project::Keyframe::new(Tick::ZERO, V::Scalar(0.314159265)))
        .unwrap();
    editor
        .session
        .apply(Command::SetAnimatedParam {
            item: video,
            path: path.clone(),
            param: base.clone(),
        })
        .unwrap();
    editor
        .session
        .apply(Command::UpdateEffectParam {
            item: video,
            effect: "rgb".into(),
            key: "color".into(),
            value: project::EffectParamUpdate::Scalar(16711680.0),
        })
        .unwrap();
    let mut h = build(editor);
    h.run_steps(3);
    open_effect_tabs(&mut h, video);
    click(&mut h, "effects.rgb.param.color");
    assert_eq!(
        h.state().inspect().unwrap().color_preview,
        Some(Color([99, 137, 188, 255]))
    );
    type_value(&mut h, "color.hex", "#00FF00");
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    click(&mut h, "effects.rgb.param.color.keyframe");
    click(&mut h, "effects.rgb.param.color.keyframe");
    let edited = item(&h, video);
    let param = edited.animations.get(&path).unwrap();
    for c in [C::R, C::G, C::B] {
        assert!(param.channel(Some(c)).unwrap().keys().is_empty());
    }
    assert_eq!(
        param.value_at(Tick::ZERO).unwrap(),
        V::Color([0.0, 1.0, 0.0, 0.314159265])
    );
    click(&mut h, "effects.rgb.param.color");
    type_value(&mut h, "color.hex", "#0000FF");
    assert_eq!(
        item(&h, video)
            .animations
            .value_at(&path, Tick::ZERO)
            .unwrap(),
        Some(V::Color([0.0, 0.0, 1.0, 0.314159265]))
    );
}

#[test]
fn effect_rgb_picker_cancels_stale_gestures_and_disables_keyed_edits_outside() {
    for reason in ["escape", "selection", "remove", "lock", "time", "tab"] {
        let f = fixture();
        let video = f.video;
        let image = f.image;
        let main = f.main;
        let mut editor = f.editor;
        editor
            .session
            .apply(Command::AddEffect {
                item: video,
                effect: project::effect_definition("strobe")
                    .unwrap()
                    .instance("rgb"),
            })
            .unwrap();
        let mut h = build(editor);
        h.run_steps(3);
        open_effect_tabs(&mut h, video);
        click(&mut h, "effects.rgb.param.color");
        let slider = widget_rect(&h, "color.v.slider");
        h.drag_at(slider.center());
        h.run_steps(2);
        h.hover_at(slider.left_center());
        h.run_steps(2);
        let before = h.state().session.project().clone();
        let history = h.state().session.history_len();
        match reason {
            "escape" => h.key_press(egui::Key::Escape),
            "selection" => {
                h.state_mut().select([image].into_iter().collect()).unwrap();
            }
            "remove" => {
                h.state_mut()
                    .session
                    .apply(Command::RemoveEffect {
                        item: video,
                        effect: "rgb".into(),
                    })
                    .unwrap();
            }
            "lock" => {
                h.state_mut()
                    .session
                    .apply(Command::SetTrackFlags {
                        track: main,
                        locked: true,
                        muted: false,
                        hidden: false,
                    })
                    .unwrap();
            }
            "time" => {
                let project = h.state().session.project().clone();
                h.state_mut()
                    .playback
                    .seek(Tick(SECOND), false, project)
                    .unwrap();
            }
            "tab" => {
                h.state_mut()
                    .preferences
                    .set("properties_tabs", serde_json::json!({"video":"transform"}))
                    .unwrap();
            }
            _ => panic!("unknown case"),
        }
        let expected = h.state().session.project().clone();
        let expected_history = h.state().session.history_len();
        h.run_steps(3);
        h.drop_at(slider.left_center());
        h.run_steps(2);
        assert_eq!(h.state().session.project(), &expected, "{reason}");
        assert_eq!(
            h.state().session.history_len(),
            expected_history,
            "{reason}"
        );
        assert_eq!(h.state().inspect().unwrap().color_preview, None, "{reason}");
        if !matches!(reason, "lock" | "remove") {
            assert_eq!(&expected, &before);
            assert_eq!(expected_history, history);
        }
    }
    let f = fixture();
    let video = f.video;
    let mut h = build(f.editor);
    h.run_steps(3);
    h.state_mut()
        .session
        .apply(Command::AddEffect {
            item: video,
            effect: project::effect_definition("strobe")
                .unwrap()
                .instance("rgb"),
        })
        .unwrap();
    open_effect_tabs(&mut h, video);
    click(&mut h, "effects.rgb.param.color.keyframe");
    let project = h.state().session.project().clone();
    h.state_mut()
        .playback
        .seek(Tick(3 * SECOND), false, project)
        .unwrap();
    h.run_steps(2);
    let widgets = h.state().inspect().unwrap().widgets;
    for id in [
        "effects.rgb.param.color",
        "effects.rgb.param.color.keyframe",
    ] {
        assert!(!widgets.iter().find(|w| w.id == id).unwrap().enabled);
    }
    let before = h.state().session.project().clone();
    let history = h.state().session.history_len();
    click(&mut h, "effects.rgb.param.color");
    assert_eq!(h.state().session.project(), &before);
    assert_eq!(h.state().session.history_len(), history);
    let project = h.state().session.project().clone();
    h.state_mut()
        .playback
        .seek(Tick::ZERO, false, project)
        .unwrap();
    h.run_steps(2);
    click(&mut h, "effects.rgb.param.color.keyframe");
    let project = h.state().session.project().clone();
    h.state_mut()
        .playback
        .seek(Tick(3 * SECOND), false, project)
        .unwrap();
    h.run_steps(2);
    click(&mut h, "effects.rgb.param.color");
    let history = h.state().session.history_len().0;
    let slider = widget_rect(&h, "color.v.slider");
    h.drag_at(slider.center());
    h.run_steps(2);
    h.hover_at(slider.left_center());
    h.run_steps(2);
    assert_eq!(h.state().session.history_len().0, history);
    h.drop_at(slider.left_center());
    h.run_steps(2);
    assert_eq!(h.state().session.history_len().0, history + 1);
    let path = project::AnimationPath::new("effects.rgb.params.color").unwrap();
    assert_eq!(
        item(&h, video)
            .animations
            .value_at(&path, Tick(3 * SECOND))
            .unwrap(),
        Some(project::ParamValue::Color([0.0, 0.0, 0.0, 1.0]))
    );
}

#[test]
fn streak_hard_entry_soft_drag_keys_units_and_cancel_use_real_commands() {
    let f = fixture();
    let video = f.video;
    let mut h = build(f.editor);
    h.run_steps(3);
    open_effect_tabs(&mut h, video);
    click(&mut h, "effects.add.directional_blur");
    let field = "effects.directional_blur.param.length";
    let direction = "effects.directional_blur.param.direction";
    let value = |h: &Harness<Editor>| {
        effects(h, video)[0]
            .value("length")
            .unwrap()
            .scalar()
            .unwrap()
    };
    assert_eq!(value(&h), 0.0);
    assert_eq!(
        effects(&h, video)[0].value("direction"),
        Some(&EffectValue::Scalar(0.0))
    );
    let before = h.state().session.history_len();
    type_value(&mut h, field, "125");
    assert_eq!(value(&h), 125.0);
    assert_eq!(h.state().session.history_len().0, before.0 + 1);
    assert_eq!(h.get_by_label("Length").value().as_deref(), Some("125"));
    assert!(paints_text(&h, "px", egui::Rect::EVERYTHING));
    assert!(paints_text(&h, "\u{b0}", egui::Rect::EVERYTHING));
    for bad in ["1000.1", "-1", "NaN", "inf"] {
        let before = h.state().session.history_len();
        type_value(&mut h, field, bad);
        assert_eq!(value(&h), 125.0);
        assert_eq!(h.state().session.history_len(), before);
    }
    type_value(&mut h, direction, "-360");
    assert_eq!(
        effects(&h, video)[0].value("direction"),
        Some(&EffectValue::Scalar(-360.0))
    );
    type_value(&mut h, direction, "360");
    assert_eq!(
        effects(&h, video)[0].value("direction"),
        Some(&EffectValue::Scalar(360.0))
    );
    click(&mut h, field);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text("126.5".into()));
    click(&mut h, direction);
    assert_eq!(value(&h), 126.5, "blur commits without soft clipping");
    click(&mut h, field);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text("170".into()));
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert_eq!(value(&h), 126.5);
    let handle = format!("{field}.scrub");
    let start = widget_rect(&h, &handle).center();
    drag_to(&mut h, &handle, egui::vec2(20.0, 0.0));
    assert_eq!(value(&h), 20.0);
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    h.drop_at(start);
    h.run_steps(2);
    assert_eq!(value(&h), 126.5);
    let before = h.state().session.history_len();
    drag_to(&mut h, &handle, egui::vec2(20.0, 0.0));
    h.drop_at(start + egui::vec2(20.0, 0.0));
    h.run_steps(2);
    assert_eq!(value(&h), 20.0);
    assert_eq!(h.state().session.history_len().0, before.0 + 1);
    h.state_mut().activate("edit.undo").unwrap();
    h.run_steps(2);
    assert_eq!(value(&h), 126.5);
    click(&mut h, &format!("{field}.keyframe"));
    type_value(&mut h, field, "140");
    assert_eq!(value(&h), 126.5, "keyed edit preserves static fallback");
    let path = project::AnimationPath::new("effects.directional_blur.params.length").unwrap();
    assert_eq!(
        item(&h, video)
            .animations
            .value_at(&path, Tick::ZERO)
            .unwrap(),
        Some(project::ParamValue::Scalar(140.0))
    );
    click(&mut h, &format!("{field}.keyframe"));
    type_value(&mut h, field, "141.25");
    assert_eq!(value(&h), 126.5);
    assert_eq!(
        item(&h, video)
            .animations
            .value_at(&path, Tick::ZERO)
            .unwrap(),
        Some(project::ParamValue::Scalar(141.25))
    );
    assert_eq!(h.get_by_label("Length").value().as_deref(), Some("141.25"));
    click(&mut h, &format!("{field}.keyframe"));
    click(&mut h, field);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text("190".into()));
    h.run_steps(2);
    let project = h.state().session.project().clone();
    h.state_mut()
        .playback
        .seek(Tick(3 * SECOND), false, project)
        .unwrap();
    h.run_steps(3);
    let animation = item(&h, video).animations;
    assert_eq!(
        animation.value_at(&path, Tick::ZERO).unwrap(),
        Some(project::ParamValue::Scalar(141.25))
    );
    let keys = animation.get(&path).unwrap().channel(None).unwrap().keys();
    assert_eq!(keys.len(), 1);
    let node = h.get(By::new().predicate(|n| n.author_id() == Some(field)));
    assert!(node.accesskit_node().is_disabled());
}

fn effect_of(harness: &Harness<Editor>, item: ItemId, kind: &str) -> project::EffectInstance {
    effects(harness, item)
        .into_iter()
        .find(|effect| effect.kind == kind)
        .unwrap_or_else(|| panic!("missing effect {kind}"))
}

/// Actual production pass execution on a small working image, independent of UI.
fn effect_picture(instance: &project::EffectInstance) -> Vec<[f32; 4]> {
    let budget = ByteBudget::new(64 << 20).unwrap();
    let hooks = engine::ByteBudgetHooks::new(&budget, None);
    let (w, h) = (7u32, 5u32);
    let pixels: Vec<[f32; 4]> = (0..w * h)
        .map(|i| {
            let a = [0.25f32, 0.5, 1.0][(i % 3) as usize];
            let v = (i % w) as f32 / (w - 1) as f32;
            [v * a, (1.0 - v) * a, 0.5 * a, a]
        })
        .collect();
    let source =
        || render::Image::from_working_pixels(w, h, pixels.iter().copied(), &hooks).unwrap();
    let image = source();
    let passes = render::effects::evaluate_effect(
        &project::Animations::default(),
        instance,
        Tick::ZERO,
        EffectGeometry::new([w, h], [1.0, 1.0]).unwrap(),
        true,
        &render::lut::LutResources::default(),
    )
    .unwrap();
    render::effects::apply_effect(
        source(),
        &passes,
        &image,
        &render::effects::CpuEffects,
        &hooks,
    )
    .unwrap()
    .pixels()
    .to_vec()
}

#[test]
fn edge_relief_tile_presets_change_the_actual_demo_picture() {
    use vibecut::EffectPreviews;
    let f = fixture();
    let uploads: Uploads = Default::default();
    let mut h = crate::gpu::builder()
        .renderer(PreviewUploads {
            inner: Default::default(),
            seen: uploads.clone(),
        })
        .with_size(egui::vec2(1440.0, 900.0))
        .build_ui_state(|ui, editor: &mut Editor| editor.ui(ui), f.editor);
    h.run_steps(3);
    click(&mut h, "assets.tab.effects");
    h.state_mut().settle_previews(SETTLE_BUDGET).unwrap();
    let mut captured = std::collections::BTreeMap::new();
    for _ in 0..12 {
        h.run_steps(1);
        for (id, image) in uploads.lock().unwrap().iter() {
            captured.insert(*id, image.clone());
        }
    }
    // A zero-intensity Blur is the exact demo source through the same job path.
    let mut base = EffectPreviews::cpu().unwrap();
    let plain = project::effect_definition("blur")
        .unwrap()
        .instance("blur")
        .with("intensity", EffectValue::Scalar(0.0))
        .unwrap();
    base.ensure("blur", &plain, &project::Animations::default());
    base.settle(SETTLE_BUDGET).unwrap();
    let plain = base.image("blur").unwrap();
    let mut tiles = Vec::new();
    for kind in ["find_edges", "color_emboss"] {
        let image = captured
            .get(&tile_texture(&h, kind))
            .cloned()
            .unwrap_or_else(|| panic!("no upload for {kind}"));
        assert_ne!(image, plain, "{kind} tile must not be the untouched demo");
        tiles.push(image);
    }
    assert_ne!(tiles[0], tiles[1], "the two effects read differently");
}

#[test]
fn edge_relief_categories_units_pictures_keys_and_pending_removal_use_real_commands() {
    let f = fixture();
    let video = f.video;
    let mut h = crate::gpu::builder()
        .with_size(egui::vec2(1440.0, 1800.0))
        .build_ui_state(|ui, editor: &mut Editor| editor.ui(ui), f.editor);
    h.run_steps(3);
    open_effect_tabs(&mut h, video);
    click(&mut h, "effects.category.stylize");
    assert!(has_widget(&h, "effects.tile.find_edges"));
    assert!(has_widget(&h, "effects.tile.color_emboss"));
    click(&mut h, "effects.category.colour");
    assert!(!has_widget(&h, "effects.tile.find_edges"));
    click(&mut h, "effects.category.stylize");

    click(&mut h, "effects.add.find_edges");
    let edge = effect_of(&h, video, "find_edges");
    assert_eq!(edge.value("invert"), Some(&EffectValue::Bool(false)));
    assert_eq!(edge.value("blend"), Some(&EffectValue::Scalar(0.0)));
    let plain = effect_picture(&edge);
    let history = h.state().session.history_len();
    click(&mut h, "effects.find_edges.param.invert");
    let inverted = effect_of(&h, video, "find_edges");
    assert_eq!(inverted.value("invert"), Some(&EffectValue::Bool(true)));
    assert_eq!(h.state().session.history_len().0, history.0 + 1);
    assert_ne!(
        effect_picture(&inverted),
        plain,
        "invert changes the picture"
    );
    h.state_mut().activate("edit.undo").unwrap();
    h.run_steps(2);
    assert_eq!(
        effect_of(&h, video, "find_edges").value("invert"),
        Some(&EffectValue::Bool(false))
    );
    assert_eq!(effect_picture(&effect_of(&h, video, "find_edges")), plain);
    let blend = "effects.find_edges.param.blend";
    type_value(&mut h, blend, "50");
    assert_eq!(
        effect_of(&h, video, "find_edges").value("blend"),
        Some(&EffectValue::Scalar(50.0))
    );
    assert!(paints_text(&h, "%", egui::Rect::EVERYTHING));
    for bad in ["100.1", "-0.1", "NaN", "inf"] {
        type_value(&mut h, blend, bad);
        assert_eq!(
            effect_of(&h, video, "find_edges").value("blend"),
            Some(&EffectValue::Scalar(50.0)),
            "{bad}"
        );
    }

    click(&mut h, "effects.add.color_emboss");
    let relief = "effects.color_emboss.param.relief";
    let direction = "effects.color_emboss.param.direction";
    let contrast = "effects.color_emboss.param.contrast";
    let effect_blend = "effects.color_emboss.param.blend";
    let value =
        |h: &Harness<Editor>, id: &str| effect_of(h, video, "color_emboss").value(id).cloned();
    assert_eq!(value(&h, "direction"), Some(EffectValue::Scalar(45.0)));
    assert_eq!(value(&h, "relief"), Some(EffectValue::Scalar(1.0)));
    assert_eq!(value(&h, "contrast"), Some(EffectValue::Scalar(100.0)));
    assert_eq!(value(&h, "blend"), Some(EffectValue::Scalar(0.0)));
    type_value(&mut h, direction, "-360");
    assert_eq!(value(&h, "direction"), Some(EffectValue::Scalar(-360.0)));
    type_value(&mut h, direction, "360.1");
    assert_eq!(value(&h, "direction"), Some(EffectValue::Scalar(-360.0)));
    type_value(&mut h, relief, "2.5");
    assert_eq!(value(&h, "relief"), Some(EffectValue::Scalar(2.5)));
    assert!(paints_text(&h, "px", egui::Rect::EVERYTHING));
    for bad in ["10.1", "-0.1", "NaN", "inf"] {
        type_value(&mut h, relief, bad);
        assert_eq!(
            value(&h, "relief"),
            Some(EffectValue::Scalar(2.5)),
            "Relief {bad}"
        );
    }
    type_value(&mut h, contrast, "499.5");
    assert_eq!(value(&h, "contrast"), Some(EffectValue::Scalar(499.5)));
    assert!(paints_text(&h, "%", egui::Rect::EVERYTHING));
    type_value(&mut h, effect_blend, "37.5");
    assert_eq!(value(&h, "blend"), Some(EffectValue::Scalar(37.5)));
    assert!(paints_text(&h, "\u{b0}", egui::Rect::EVERYTHING));

    let path = project::AnimationPath::new("effects.color_emboss.params.relief").unwrap();
    h.state_mut()
        .session
        .apply(Command::SetAnimatedParam {
            item: video,
            path: path.clone(),
            param: project::Param::new(project::ParamValue::Scalar(3.0)).unwrap(),
        })
        .unwrap();
    h.run_steps(3);
    type_value(&mut h, relief, "4.25");
    assert_eq!(
        item(&h, video)
            .animations
            .value_at(&path, Tick::ZERO)
            .unwrap(),
        Some(project::ParamValue::Scalar(4.25))
    );
    assert_eq!(value(&h, "relief"), Some(EffectValue::Scalar(2.5)));
    click(&mut h, &format!("{relief}.keyframe"));
    type_value(&mut h, relief, "6.5");
    assert_eq!(
        item(&h, video)
            .animations
            .value_at(&path, Tick::ZERO)
            .unwrap(),
        Some(project::ParamValue::Scalar(6.5))
    );
    assert_eq!(value(&h, "relief"), Some(EffectValue::Scalar(2.5)));
    click(&mut h, &format!("{relief}.keyframe"));
    type_value(&mut h, relief, "7.25");
    assert_eq!(
        item(&h, video)
            .animations
            .value_at(&path, Tick::ZERO)
            .unwrap(),
        Some(project::ParamValue::Scalar(7.25))
    );
    assert!(
        item(&h, video)
            .animations
            .get(&path)
            .unwrap()
            .channel(None)
            .unwrap()
            .keys()
            .is_empty(),
        "last key removal keeps the authoritative base"
    );

    let before_remove = h.state().session.project().clone();
    let history = h.state().session.history_len();
    click(&mut h, relief);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text("9.5".into()));
    h.run_steps(2);
    click(&mut h, "effects.remove.color_emboss");
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    assert!(effects(&h, video).iter().all(|e| e.kind != "color_emboss"));
    assert_eq!(h.state().session.history_len().0, history.0 + 1);
    h.state_mut().activate("edit.undo").unwrap();
    h.run_steps(2);
    assert_eq!(h.state().session.project(), &before_remove);
    assert_eq!(value(&h, "relief"), Some(EffectValue::Scalar(2.5)));
}

#[test]
fn streak_pending_edit_invalidates_on_selection_panel_effect_and_lock() {
    for reason in ["selection", "panel", "effect", "lock", "remove"] {
        let f = fixture();
        let video = f.video;
        let image = f.image;
        let main = f.main;
        let mut h = build(f.editor);
        h.run_steps(3);
        open_effect_tabs(&mut h, video);
        click(&mut h, "effects.add.directional_blur");
        type_value(&mut h, "effects.directional_blur.param.length", "8");
        let before = h.state().session.project().clone();
        click(&mut h, "effects.directional_blur.param.length");
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
        h.event(egui::Event::Text("125".into()));
        h.run_steps(2);
        let history = h.state().session.history_len();
        match reason {
            "selection" => select(&mut h, image),
            "panel" => click(&mut h, "inspector.group.basic"),
            "remove" => click(&mut h, "effects.remove.directional_blur"),
            "effect" => {
                h.state_mut()
                    .session
                    .apply(Command::RemoveEffect {
                        item: video,
                        effect: "directional_blur".into(),
                    })
                    .unwrap();
            }
            _ => {
                h.state_mut()
                    .session
                    .apply(Command::SetTrackFlags {
                        track: main,
                        muted: false,
                        hidden: false,
                        locked: true,
                    })
                    .unwrap();
            }
        }
        h.run_steps(3);
        h.key_press(egui::Key::Enter);
        h.run_steps(2);
        if !matches!(reason, "effect" | "remove") {
            assert_eq!(
                effects(&h, video)[0].value("length"),
                Some(&EffectValue::Scalar(8.0)),
                "{reason}"
            );
        } else {
            assert!(effects(&h, video).is_empty());
        }
        if matches!(reason, "selection" | "panel") {
            assert!(Arc::ptr_eq(h.state().session.project(), &before));
        }
        if reason == "remove" {
            assert_eq!(h.state().session.history_len().0, history.0 + 1);
            h.state_mut().activate("edit.undo").unwrap();
            h.run_steps(2);
            assert_eq!(h.state().session.project(), &before);
            assert_eq!(
                effects(&h, video)[0].value("length"),
                Some(&EffectValue::Scalar(8.0))
            );
            h.state_mut().activate("edit.redo").unwrap();
            h.run_steps(2);
            assert!(effects(&h, video).is_empty());
        }
    }
}

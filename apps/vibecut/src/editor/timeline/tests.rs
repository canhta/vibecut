//! Verifies the timeline interface: the actions `show` returns for each gesture.
#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]
use super::*;
use crate::editor::context::visuals::{
    StripState, StripTile, VisualQuery, VisualRequest, WaveBar, WaveState,
};
use egui_kittest::Harness;
use engine::{NewItem, Session};
use project::{Project, ProjectId};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use time::{SourceRange, TICKS_PER_SECOND};

const SECOND: i64 = TICKS_PER_SECOND;

/// A cache-only display stand-in that records what the panel asked for.
#[derive(Default)]
struct FakeVisuals {
    strips: RefCell<BTreeMap<AssetId, FakeStrip>>,
    waves: RefCell<BTreeMap<AssetId, FakeWave>>,
    requests: RefCell<Vec<VisualRequest>>,
    calls: RefCell<Vec<VisualCall>>,
    textures: RefCell<BTreeMap<(AssetId, Tick), egui::TextureHandle>>,
}

#[derive(Clone, Debug, PartialEq)]
enum FakeStrip {
    Ready(Vec<time::TimeRange>),
    Pending,
    Failed,
    Unsupported,
}

#[derive(Clone, Debug, PartialEq)]
enum FakeWave {
    Ready(Vec<WaveBar>),
    Pending,
    Failed,
    Unsupported,
}

#[derive(Clone, Debug, PartialEq)]
struct VisualCall {
    asset: AssetId,
    item: ItemId,
    start: Tick,
    end: Tick,
    source: SourceRange,
    visible: time::TimeRange,
    pixels_per_second: f64,
    columns: usize,
}

impl FakeVisuals {
    fn texture(&self, ctx: &egui::Context, asset: AssetId, at: Tick) -> egui::TextureHandle {
        let mut textures = self.textures.borrow_mut();
        if let Some(texture) = textures.get(&(asset, at)) {
            return texture.clone();
        }
        let shade = ((at.0 % 255) + 1) as u8;
        let texture = ctx.load_texture(
            format!("fake.strip.{}.{}", asset.0, at.0),
            egui::ColorImage::from_rgba_unmultiplied(
                [2, 1],
                &[shade, shade, shade, 255, 32, 32, 32, 255],
            ),
            egui::TextureOptions::LINEAR,
        );
        textures.insert((asset, at), texture.clone());
        texture
    }
    fn strip_ready(&mut self, asset: AssetId, ranges: Vec<time::TimeRange>) {
        self.strips
            .borrow_mut()
            .insert(asset, FakeStrip::Ready(ranges));
    }
    fn strip_state(&mut self, asset: AssetId, state: FakeStrip) {
        self.strips.borrow_mut().insert(asset, state);
    }
    fn wave_ready(&mut self, asset: AssetId, bars: Vec<(f32, f32, bool)>) {
        let bars = bars
            .into_iter()
            .map(|(min, max, over)| WaveBar { min, max, over })
            .collect();
        self.waves.borrow_mut().insert(asset, FakeWave::Ready(bars));
    }
    fn wave_state(&mut self, asset: AssetId, state: FakeWave) {
        self.waves.borrow_mut().insert(asset, state);
    }
    fn calls(&self) -> Vec<VisualCall> {
        self.calls.borrow().clone()
    }
}

impl VisualQuery for FakeVisuals {
    fn strip(
        &self,
        ctx: &egui::Context,
        asset: &media::SourceAsset,
        item: &project::Item,
        visible: time::TimeRange,
        pixels_per_second: f64,
    ) -> StripState {
        self.calls.borrow_mut().push(VisualCall {
            asset: asset.id(),
            item: item.id,
            start: item.start,
            end: item.end().unwrap_or(Tick::ZERO),
            source: item.source_range,
            visible,
            pixels_per_second,
            columns: 0,
        });
        match self.strips.borrow().get(&asset.id()).cloned() {
            Some(FakeStrip::Ready(ranges)) => {
                let tiles = ranges
                    .into_iter()
                    .map(|output| StripTile {
                        output,
                        texture: self.texture(ctx, asset.id(), output.start()),
                    })
                    .collect();
                StripState::Ready(tiles)
            }
            Some(FakeStrip::Failed) => StripState::Failed,
            Some(FakeStrip::Unsupported) => StripState::Unsupported,
            Some(FakeStrip::Pending) | None => {
                self.requests.borrow_mut().push(VisualRequest::Thumbnails {
                    item: item.id,
                    visible,
                    pixels_per_second,
                });
                StripState::Pending
            }
        }
    }

    fn waveform(
        &self,
        asset: &media::SourceAsset,
        item: &project::Item,
        visible: time::TimeRange,
        columns: usize,
    ) -> WaveState {
        self.calls.borrow_mut().push(VisualCall {
            asset: asset.id(),
            item: item.id,
            start: item.start,
            end: item.end().unwrap_or(Tick::ZERO),
            source: item.source_range,
            visible,
            pixels_per_second: 0.0,
            columns,
        });
        match self.waves.borrow().get(&asset.id()).cloned() {
            Some(FakeWave::Ready(bars)) => WaveState::Ready(bars),
            Some(FakeWave::Failed) => WaveState::Failed,
            Some(FakeWave::Unsupported) => WaveState::Unsupported,
            Some(FakeWave::Pending) | None => {
                self.requests
                    .borrow_mut()
                    .push(VisualRequest::Waveform { asset: asset.id() });
                WaveState::Pending
            }
        }
    }

    fn take_requests(&self) -> Vec<VisualRequest> {
        std::mem::take(&mut *self.requests.borrow_mut())
    }
}

/// Everything `show` reads, owned by the test so the panel can be driven alone.
struct Rig {
    session: Session,
    playback: crate::services::playback::Playback,
    selection: BTreeSet<ItemId>,
    disabled: BTreeSet<&'static str>,
    preferences: crate::preferences::Preferences,
    visuals: FakeVisuals,
    state: State,
    /// Actions returned since the test last took them.
    returned: Vec<Action>,
    /// Controls painted in the latest frame.
    widgets: Vec<state::Remembered>,
    /// Paint the scene sheet instead of the panel.
    sheet: bool,
    configured: bool,
    first: ItemId,
    second: ItemId,
    track: TrackId,
}

impl Rig {
    fn new() -> Self {
        let mut session =
            Session::new(Project::empty(ProjectId(1), "Timeline".into()).unwrap(), 50).unwrap();
        let track = match session
            .apply(Command::AddTrack {
                placement: engine::TrackPlacement::Auto,
                name: "Text".into(),
                kind: TrackKind::Text,
            })
            .unwrap()
            .created
            .first()
        {
            Some(engine::CreatedId::Track(id)) => *id,
            _ => panic!("track"),
        };
        let mut insert = |start: i64, end: i64| match session
            .apply(Command::InsertItem {
                placement: engine::Placement::Track { track },
                item: NewItem {
                    retime: Default::default(),
                    animations: Default::default(),
                    source_asset: None,
                    source_range: SourceRange::new(Tick(start), Tick(end)).unwrap(),
                    start: Tick(start),
                    content: ItemParams::Text(project::TextParams {
                        content: "Hello".into(),
                        font_size: 48.0,
                        ..Default::default()
                    }),
                },
            })
            .unwrap()
            .created
            .first()
        {
            Some(engine::CreatedId::Item(id)) => *id,
            _ => panic!("item"),
        };
        let first = insert(SECOND, 3 * SECOND);
        let second = insert(5 * SECOND, 7 * SECOND);
        let playback = crate::services::playback::Playback::new(
            &session,
            engine::ByteBudget::new(64 << 20).unwrap(),
            std::sync::Arc::new(crate::services::playback::Files),
        )
        .unwrap();
        Self {
            session,
            playback,
            selection: BTreeSet::new(),
            disabled: BTreeSet::new(),
            preferences: crate::preferences::Preferences::default(),
            visuals: FakeVisuals::default(),
            state: State::default(),
            returned: Vec::new(),
            widgets: Vec::new(),
            sheet: false,
            configured: false,
            first,
            second,
            track,
        }
    }
    fn visual(kind: TrackKind) -> Self {
        let mut session =
            Session::new(Project::empty(ProjectId(1), "Visuals".into()).unwrap(), 50).unwrap();
        let stream = match kind {
            TrackKind::Audio => testkit::source::audio(),
            _ => testkit::source::video(),
        };
        let asset = testkit::source::asset(1, std::env::temp_dir().join("fixture.mp4"))
            .format("mov,mp4")
            .duration(Tick(5 * SECOND))
            .hash([7; 32])
            .stream(stream.duration(Tick(5 * SECOND)).build())
            .build();
        session.apply(Command::AddAsset { asset }).unwrap();
        let track = match session
            .apply(Command::AddTrack {
                placement: engine::TrackPlacement::Auto,
                name: "Media".into(),
                kind,
            })
            .unwrap()
            .created
            .first()
        {
            Some(engine::CreatedId::Track(id)) => *id,
            _ => panic!("track"),
        };
        let content = if kind == TrackKind::Audio {
            ItemParams::Audio(project::AudioParams {
                volume: 0.0,
                muted: false,
                source: Default::default(),
                fades: Default::default(),
            })
        } else {
            ItemParams::Video(project::VideoParams {
                source_audio_enabled: true,
                volume: 0.0,
                muted: false,
                crop: None,
                fades: Default::default(),
            })
        };
        let first = match session
            .apply(Command::InsertItem {
                placement: engine::Placement::Track { track },
                item: NewItem {
                    retime: Default::default(),
                    animations: Default::default(),
                    source_asset: Some(AssetId(1)),
                    source_range: SourceRange::new(Tick::ZERO, Tick(4 * SECOND)).unwrap(),
                    start: Tick::ZERO,
                    content,
                },
            })
            .unwrap()
            .created
            .first()
        {
            Some(engine::CreatedId::Item(id)) => *id,
            _ => panic!("item"),
        };
        let playback = crate::services::playback::Playback::new(
            &session,
            engine::ByteBudget::new(64 << 20).unwrap(),
            std::sync::Arc::new(crate::services::playback::Files),
        )
        .unwrap();
        Self {
            session,
            playback,
            selection: BTreeSet::new(),
            disabled: BTreeSet::new(),
            preferences: crate::preferences::Preferences::default(),
            visuals: FakeVisuals::default(),
            state: State::default(),
            returned: Vec::new(),
            widgets: Vec::new(),
            sheet: false,
            configured: false,
            first,
            second: first,
            track,
        }
    }
    fn paint(&mut self, ui: &mut egui::Ui) {
        if !self.configured {
            crate::kit::install_fonts(ui.ctx());
            crate::kit::install(ui.ctx());
            self.configured = true;
        }
        crate::kit::install_theme(ui.ctx(), true);
        crate::kit::install_editor_density(ui.ctx());
        let keyframes = crate::selection::SelectionScopes::default().keyframes;
        let cx = Context {
            session: &self.session,
            selection: &self.selection,
            locale: Locale::En,
            dark: true,
            save_status: Text::AutoSaved,
            playhead: Tick::ZERO,
            ripple: false,
            preferences: &self.preferences,
            playback: &self.playback,
            preview: None,
            keybindings: &self.preferences.keybindings,
            scope: crate::selection::SelectionScope::None,
            keyframes: &keyframes,
            visuals: &self.visuals,
            disabled: self.disabled.clone(),
            effect_targets: None,
            key_color_pick: None,
        };
        if self.sheet {
            self.returned
                .extend(show_scenes(&mut self.state, &cx, ui.ctx()));
        } else {
            self.returned.extend(show(&mut self.state, &cx, ui));
        }
        self.widgets = self.state.take_widgets();
    }
    fn rect(&self, id: &str) -> egui::Rect {
        self.widgets
            .iter()
            .find(|widget| widget.id == id)
            .unwrap_or_else(|| panic!("missing control {id}"))
            .response
            .rect
    }
    fn take(&mut self) -> Vec<Action> {
        std::mem::take(&mut self.returned)
    }
    fn tooltip(&self, id: &str) -> Option<String> {
        self.widgets
            .iter()
            .find(|widget| widget.id == id)
            .and_then(|widget| widget.tooltip.clone())
    }
    fn calls(&self) -> Vec<VisualCall> {
        self.visuals.calls()
    }
}

fn harness(rig: Rig) -> Harness<'static, Rig> {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1000.0, 260.0))
        .build_ui_state(|ui, rig: &mut Rig| rig.paint(ui), rig);
    harness.run_steps(3);
    harness
}

fn click(harness: &mut Harness<'_, Rig>, at: egui::Pos2) {
    harness.drag_at(at);
    harness.run_steps(1);
    harness.drop_at(at);
    harness.run_steps(2);
}

fn drag(harness: &mut Harness<'_, Rig>, from: egui::Pos2, to: egui::Pos2) {
    harness.drag_at(from);
    harness.run_steps(1);
    harness.hover_at(to);
    harness.run_steps(2);
    harness.drop_at(to);
    harness.run_steps(2);
}

/// The source assets the visual tests insert: one per content hash/kind.
fn source(id: u64, hash: u8, kind: TrackKind) -> media::SourceAsset {
    let stream = match kind {
        TrackKind::Audio => testkit::source::audio(),
        _ => testkit::source::video(),
    };
    testkit::source::asset(id, std::env::temp_dir().join("fixture.mp4"))
        .format("mov,mp4")
        .duration(Tick(5 * SECOND))
        .hash([hash; 32])
        .stream(stream.duration(Tick(5 * SECOND)).build())
        .build()
}

fn new_media_item(asset: AssetId, kind: TrackKind) -> NewItem {
    let content = if kind == TrackKind::Audio {
        ItemParams::Audio(project::AudioParams {
            volume: 0.0,
            muted: false,
            source: Default::default(),
            fades: Default::default(),
        })
    } else {
        ItemParams::Video(project::VideoParams {
            source_audio_enabled: true,
            volume: 0.0,
            muted: false,
            crop: None,
            fades: Default::default(),
        })
    };
    NewItem {
        retime: Default::default(),
        animations: Default::default(),
        source_asset: Some(asset),
        source_range: SourceRange::new(Tick::ZERO, Tick(4 * SECOND)).unwrap(),
        start: Tick::ZERO,
        content,
    }
}

fn ranges(count: i64) -> Vec<time::TimeRange> {
    (0..count)
        .map(|index| {
            time::TimeRange::new(Tick(index * SECOND), Tick((index + 1) * SECOND)).unwrap()
        })
        .collect()
}

#[test]
fn desktop_media_access_serves_mcp_generation_and_the_timeline_display() {
    if !testkit::oracle::require("desktop shared media") {
        return;
    }
    let budget = engine::ByteBudget::new(128 << 20).unwrap();
    let cache = tempfile::tempdir().unwrap();
    let shared = Arc::new(Mutex::new(
        engine::control::MediaAccess::open_in(cache.path(), budget.clone()).unwrap(),
    ));
    let dir = testkit::fixture::dir("p1-93-shared-media");
    let wav = testkit::oracle::generate(
        &dir,
        "desktop shared media",
        "tone.wav",
        &[
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=1",
            "-ac",
            "1",
            "-ar",
            "8000",
        ],
    )
    .unwrap();
    let asset = media::SourceAsset::import(AssetId(1), wav).unwrap();
    let mut session =
        Session::new(Project::empty(ProjectId(7), "Shared".into()).unwrap(), 50).unwrap();
    session
        .apply(Command::AddAsset {
            asset: asset.clone(),
        })
        .unwrap();
    let track = match session
        .apply(Command::AddTrack {
            placement: engine::TrackPlacement::Auto,
            name: "Audio".into(),
            kind: TrackKind::Audio,
        })
        .unwrap()
        .created
        .first()
    {
        Some(engine::CreatedId::Track(id)) => *id,
        _ => panic!("track"),
    };
    let item = match session
        .apply(Command::InsertItem {
            placement: engine::Placement::Track { track },
            item: NewItem {
                retime: Default::default(),
                animations: Default::default(),
                source_asset: Some(AssetId(1)),
                source_range: SourceRange::new(Tick::ZERO, Tick(TICKS_PER_SECOND)).unwrap(),
                start: Tick::ZERO,
                content: ItemParams::Audio(project::AudioParams {
                    volume: 0.0,
                    muted: false,
                    source: Default::default(),
                    fades: Default::default(),
                }),
            },
        })
        .unwrap()
        .created
        .first()
    {
        Some(engine::CreatedId::Item(id)) => *id,
        _ => panic!("item"),
    };
    let before = session.project().clone();
    let endpoint = std::path::PathBuf::from(format!(
        "vibecut-shared-media-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let mut server = mcp::Server::bind_with_media(
        &endpoint,
        mcp::Grants::default(),
        budget.clone(),
        Some(shared.clone()),
    )
    .unwrap();
    let accepted_rev = session.revision();
    let accepted = server
        .inbox
        .execute_project(
            &mut session,
            serde_json::json!({
                "request_id": "shared-waveform",
                "idempotency_key": "shared-waveform",
                "base_rev": accepted_rev,
                "command": "media.generate_waveform",
                "params": {"asset": 1},
            }),
            |_, _| Ok(()),
        )
        .unwrap();
    let job = accepted
        .pointer("/value/value/job")
        .expect("waveform job")
        .clone();
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut status_id = 0u64;
    loop {
        server
            .inbox
            .poll_projects(|_, _, _| Ok(serde_json::Value::Null))
            .unwrap();
        status_id = status_id.saturating_add(1);
        let key = format!("shared-status-{status_id}");
        let status_rev = session.revision();
        let status = server
            .inbox
            .execute_project(
                &mut session,
                serde_json::json!({
                    "request_id": key,
                    "idempotency_key": key,
                    "base_rev": status_rev,
                    "command": "job_status",
                    "params": {"job": job},
                }),
                |_, _| Ok(()),
            )
            .unwrap();
        match status
            .pointer("/value/value/snapshot/state")
            .and_then(serde_json::Value::as_str)
        {
            Some("completed") => break,
            Some("failed" | "cancelled") => panic!("waveform job ended early: {status}"),
            _ => {}
        }
        assert!(
            Instant::now() < deadline,
            "waveform job timed out: {status}"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(
        std::sync::Arc::ptr_eq(&before, session.project()),
        "media generation must not mutate the project"
    );

    // The timeline reads the waveform through the same desktop owner; the MCP
    // job published that entry, so the display is immediately ready.
    let item = session
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| track.items.iter())
        .find(|candidate| candidate.id == item)
        .cloned()
        .unwrap();
    let visible = time::TimeRange::new(Tick::ZERO, Tick(TICKS_PER_SECOND)).unwrap();
    let mut media = Media::new(budget.clone());
    media.set_access(shared.clone());
    let state = media.waveform(&asset, &item, visible, 16);
    assert!(
        matches!(&state, WaveState::Ready(bars) if !bars.is_empty()),
        "timeline display is not ready: {state:?}"
    );
    // A second owner would have to refuse: the desktop keeps the only open.
    assert!(matches!(
        engine::control::MediaAccess::open_in(cache.path(), budget),
        Err(engine::EditError::CacheBusy)
    ));
}

#[test]
fn thumbnail_strip_and_waveform_render_and_trim_updates_them() {
    let mut rig = Rig::visual(TrackKind::Video);
    rig.visuals.strip_ready(AssetId(1), ranges(4));
    let bars = (0..64)
        .map(|index| {
            let amplitude = ((index as f32) * 0.7).sin().abs() * 0.85 + 0.12;
            (-amplitude * 0.55, amplitude, false)
        })
        .collect();
    rig.visuals.wave_ready(AssetId(1), bars);
    let first = rig.first;
    let mut harness = harness(rig);
    harness.state_mut().take();
    let strip = format!("timeline.item.{}.filmstrip", first.0);
    let wave = format!("timeline.item.{}.waveform", first.0);
    assert_eq!(
        harness.state().tooltip(&strip).as_deref(),
        Some("Thumbnails")
    );
    assert_eq!(harness.state().tooltip(&wave).as_deref(), Some("Waveform"));
    assert!(
        harness.state_mut().take().is_empty(),
        "ready asks no command"
    );
    let calls = harness.state().calls();
    let strip_call = calls
        .iter()
        .rev()
        .find(|call| call.pixels_per_second > 0.0)
        .expect("strip display queried");
    assert_eq!(strip_call.asset, AssetId(1));
    assert_eq!(strip_call.end, Tick(4 * SECOND));
    assert!(
        (strip_call.pixels_per_second - f64::from(harness.state().state.zoom)).abs() < 1e-9,
        "the timeline zoom is passed in pixels per second"
    );

    // A trim command changes the source window the display query receives.
    let at = SECOND;
    harness
        .state_mut()
        .session
        .apply(Command::TrimStart {
            item: first,
            at: Tick(at),
        })
        .unwrap();
    harness.run_steps(2);
    let calls = harness.state().calls();
    let trimmed = calls.last().expect("display queried after trim");
    assert_eq!(trimmed.start, Tick(at));
    assert_eq!(trimmed.source.start(), Tick(at));
    // Retime keeps the source window and shortens the output range the query maps.
    harness
        .state_mut()
        .session
        .apply(Command::SetSpeed {
            items: vec![first],
            speed: time::Speed::from_thousandths(2000).unwrap(),
            maintain_pitch: true,
        })
        .unwrap();
    harness.run_steps(2);
    let calls = harness.state().calls();
    let retimed = calls.last().expect("display queried after retime");
    assert_eq!(retimed.source.start(), Tick(at));
    assert_eq!(retimed.end, Tick(5 * SECOND / 2));
    assert!(
        harness.state_mut().take().is_empty(),
        "the panel itself never mutates the project"
    );

    let screenshot = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/p1-18-clip-visuals.png");
    std::fs::create_dir_all(screenshot.parent().unwrap()).unwrap();
    harness.render().unwrap().save(&screenshot).unwrap();
}

#[test]
fn pending_displays_place_holders_and_request_the_generation_commands() {
    let mut rig = Rig::visual(TrackKind::Video);
    rig.visuals.strip_state(AssetId(1), FakeStrip::Pending);
    rig.visuals.wave_state(AssetId(1), FakeWave::Pending);
    let first = rig.first;
    let mut harness = harness(rig);
    let before = harness.state().session.project().clone();
    let returned = harness.state_mut().take();
    let generated: Vec<_> = returned
        .iter()
        .filter_map(|action| match action {
            Action::Generate(request) => Some(request.clone()),
            _ => None,
        })
        .collect();
    assert!(
        generated.iter().any(|request| matches!(
            request,
            VisualRequest::Waveform { asset } if *asset == AssetId(1)
        )),
        "{generated:?}"
    );
    assert!(
        generated.iter().any(|request| matches!(
            request,
            VisualRequest::Thumbnails { item, .. } if *item == first
        )),
        "{generated:?}"
    );
    // Every request names its registered command and paints a placeholder.
    for request in &generated {
        let (command, params) = request.command();
        match request {
            VisualRequest::Waveform { asset } => {
                assert_eq!(command, "media.generate_waveform");
                assert_eq!(params["asset"], serde_json::json!(asset));
            }
            VisualRequest::Thumbnails {
                item,
                pixels_per_second,
                ..
            } => {
                assert_eq!(command, "media.generate_clip_thumbnails");
                assert_eq!(params["item"], serde_json::json!(item));
                assert_eq!(
                    params["pixels_per_second"],
                    serde_json::json!(pixels_per_second)
                );
            }
        }
    }
    assert!(
        std::sync::Arc::ptr_eq(&before, harness.state().session.project()),
        "requesting visuals never mutates the project"
    );
    let strip = format!("timeline.item.{}.filmstrip", first.0);
    let wave = format!("timeline.item.{}.waveform", first.0);
    let loading = crate::i18n::text(Locale::En, Text::Loading);
    assert_eq!(harness.state().tooltip(&strip).as_deref(), Some(loading));
    assert_eq!(harness.state().tooltip(&wave).as_deref(), Some(loading));
}

#[test]
fn failed_and_unsupported_bands_show_typed_markers_without_requests() {
    let mut rig = Rig::visual(TrackKind::Video);
    rig.visuals.strip_state(AssetId(1), FakeStrip::Failed);
    rig.visuals.wave_state(AssetId(1), FakeWave::Unsupported);
    let first = rig.first;
    let mut video = harness(rig);
    let returned = video.state_mut().take();
    assert!(
        !returned
            .iter()
            .any(|action| matches!(action, Action::Generate(_))),
        "terminal displays never retry: {returned:?}"
    );
    let strip = format!("timeline.item.{}.filmstrip", first.0);
    let wave = format!("timeline.item.{}.waveform", first.0);
    assert_eq!(
        video.state().tooltip(&strip).as_deref(),
        Some("Unavailable")
    );
    assert_eq!(
        video.state().tooltip(&wave).as_deref(),
        Some("Not supported")
    );
    // An audio clip has no filmstrip band; only its waveform is queried, and a
    // failed waveform is its own typed marker.
    let mut rig = Rig::visual(TrackKind::Audio);
    rig.visuals.strip_state(AssetId(1), FakeStrip::Unsupported);
    rig.visuals.wave_state(AssetId(1), FakeWave::Failed);
    let mut audio = harness(rig);
    audio.state_mut().take();
    let first = audio.state().first;
    let wave = format!("timeline.item.{}.waveform", first.0);
    assert_eq!(audio.state().tooltip(&wave).as_deref(), Some("Unavailable"));
    assert!(
        audio
            .state()
            .widgets
            .iter()
            .all(|widget| !widget.id.ends_with(".filmstrip")),
        "audio clips draw only a waveform"
    );
}

#[test]
fn clipping_bars_raise_the_zero_dbfs_tip_and_ready_cache_keeps_rendering_on_cancel() {
    let mut rig = Rig::visual(TrackKind::Audio);
    rig.visuals
        .wave_ready(AssetId(1), vec![(0.1, 0.3, false), (-1.4, 1.2, true)]);
    let first = rig.first;
    let mut harness = harness(rig);
    let wave = format!("timeline.item.{}.waveform", first.0);
    let label = harness.state().tooltip(&wave).expect("waveform status");
    assert!(label.contains("0 dBFS"), "{label}");
    // A cancelled regeneration publishes nothing, so the previous ready cache
    // stays on screen and the panel asks for nothing new.
    let returned = harness.state_mut().take();
    assert!(
        !returned
            .iter()
            .any(|action| matches!(action, Action::Generate(_))),
        "{returned:?}"
    );
    assert!(harness.state().tooltip(&wave).is_some());
}

#[test]
fn a_changed_source_asset_returns_to_pending_and_requests_the_new_source() {
    let mut rig = Rig::visual(TrackKind::Video);
    rig.visuals.strip_ready(AssetId(1), ranges(2));
    rig.visuals.wave_ready(AssetId(1), vec![(0.1, 0.2, false)]);
    let first = rig.first;
    let track = rig.track;
    let second_asset = source(2, 9, TrackKind::Video);
    let session = &mut rig.session;
    session
        .apply(Command::AddAsset {
            asset: second_asset,
        })
        .unwrap();
    session
        .apply(Command::Delete { items: vec![first] })
        .unwrap();
    let replacement = match session
        .apply(Command::InsertItem {
            placement: engine::Placement::Track { track },
            item: new_media_item(AssetId(2), TrackKind::Video),
        })
        .unwrap()
        .created
        .first()
    {
        Some(engine::CreatedId::Item(id)) => *id,
        _ => panic!("replacement item"),
    };
    let mut harness = harness(rig);
    let returned = harness.state_mut().take();
    let generated: Vec<_> = returned
        .iter()
        .filter_map(|action| match action {
            Action::Generate(request) => Some(request.clone()),
            _ => None,
        })
        .collect();
    assert!(
        generated.iter().any(|request| matches!(
            request,
            VisualRequest::Waveform { asset } if *asset == AssetId(2)
        )),
        "{generated:?}"
    );
    assert!(
        !generated.iter().any(|request| matches!(
            request,
            VisualRequest::Waveform { asset } if *asset == AssetId(1)
        )),
        "{generated:?}"
    );
    let calls = harness.state().calls();
    let call = calls.last().expect("display queried");
    assert_eq!(call.asset, AssetId(2));
    assert_eq!(call.item, replacement);
}

#[test]
fn toolbar_buttons_return_their_registered_action_and_respect_availability() {
    let mut harness = harness(Rig::new());
    harness.state_mut().take();
    for id in [
        "timeline.add_track",
        "edit.undo",
        "edit.redo",
        "timeline.split",
        "timeline.delete",
        "timeline.snap",
        "timeline.ripple",
        "timeline.zoom_in",
        "timeline.zoom_out",
        "scene.open",
    ] {
        let at = harness.state().rect(id).center();
        click(&mut harness, at);
        let returned = harness.state_mut().take();
        assert!(
            matches!(returned.as_slice(), [Action::Activate(name)] if name == id),
            "{id} returned {returned:?}"
        );
    }
    // Magnet edits the project, so it asks for the command itself.
    let at = harness.state().rect("timeline.magnet").center();
    click(&mut harness, at);
    let returned = harness.state_mut().take();
    assert!(
        matches!(
            returned.as_slice(),
            [Action::Command(command)]
                if matches!(
                    command.as_ref(),
                    engine::Command::SetTrackMagnet { enabled: false }
                )
        ),
        "{returned:?}"
    );
    // An action the editor refuses cannot be requested.
    harness.state_mut().disabled.insert("edit.undo");
    harness.run_steps(2);
    let at = harness.state().rect("edit.undo").center();
    click(&mut harness, at);
    assert!(harness.state_mut().take().is_empty());
}

#[test]
fn clip_drag_returns_one_preview_gesture_and_never_edits_the_session() {
    for (selected, moves) in [(vec![], 1), (vec![0], 1), (vec![0, 1], 2)] {
        let mut rig = Rig::new();
        let ids = [rig.first, rig.second];
        rig.selection = selected.iter().map(|index| ids[*index]).collect();
        let project = rig.session.project().clone();
        let grabbed_selected = rig.selection.contains(&rig.first);
        let mut harness = harness(rig);
        harness.state_mut().take();
        let body = harness.state().rect(&format!("timeline.item.{}", ids[0].0));
        drag(
            &mut harness,
            body.center(),
            body.center() + egui::vec2(60.0, 0.0),
        );
        let returned = harness.state_mut().take();
        let rig = harness.state();
        assert!(
            std::sync::Arc::ptr_eq(&project, rig.session.project()),
            "show must not edit the project"
        );
        assert!(!rig.session.gesture_active());
        let Some(Action::BeginPreview { select }) = returned.first() else {
            panic!("expected the preview gesture first: {returned:?}");
        };
        assert_eq!(
            *select,
            (!grabbed_selected).then_some(ids[0]),
            "an unselected clip is selected alone first"
        );
        assert!(matches!(returned.last(), Some(Action::EndPreview)));
        assert_eq!(
            returned
                .iter()
                .filter(|action| matches!(action, Action::BeginPreview { .. }))
                .count(),
            1
        );
        assert_eq!(
            returned
                .iter()
                .filter(|action| matches!(action, Action::EndPreview))
                .count(),
            1
        );
        let mut previews = 0;
        for action in &returned[1..returned.len() - 1] {
            let Action::PreviewGesture(gesture) = action else {
                panic!("only previews between begin and end: {action:?}");
            };
            let Gesture::Move {
                items, placement, ..
            } = gesture.as_ref()
            else {
                panic!("a move drag previews moves: {gesture:?}");
            };
            assert!(placement.is_none(), "a horizontal drag keeps its tracks");
            assert_eq!(items.len(), moves);
            previews += 1;
        }
        assert!(previews > 0, "{returned:?}");
        assert!(harness.state().state.drag.is_none());
    }
}

#[test]
fn clicks_and_ruler_return_selection_and_frame_aligned_seeks() {
    let mut harness = harness(Rig::new());
    harness.state_mut().take();
    let first = harness.state().first;
    let body = harness.state().rect(&format!("timeline.item.{}", first.0));
    click(&mut harness, body.center());
    let returned = harness.state_mut().take();
    assert!(
        matches!(returned.as_slice(), [Action::Select { item, .. }] if *item == first),
        "{returned:?}"
    );
    let ruler = harness.state().rect("timeline.ruler");
    click(&mut harness, ruler.center() + egui::vec2(37.0, 0.0));
    let returned = harness.state_mut().take();
    let [Action::Seek(at)] = returned.as_slice() else {
        panic!("{returned:?}");
    };
    let frame = harness.state().session.project().data().canvas.fps;
    assert_eq!(
        frame.snap_nearest(*at).unwrap(),
        *at,
        "seeks land on frames"
    );
    assert!(*at > Tick::ZERO);
}

#[test]
fn track_header_controls_return_track_commands() {
    let mut harness = harness(Rig::new());
    harness.state_mut().take();
    let track = harness.state().track;
    let lock = harness.state().rect(&format!("track.{}.lock", track.0));
    click(&mut harness, lock.center());
    let returned = harness.state_mut().take();
    assert!(
        matches!(
            returned.as_slice(),
            [Action::Command(command)]
                if matches!(
                    command.as_ref(),
                    engine::Command::SetTrackLocked {
                        track: clicked,
                        locked: true,
                    } if *clicked == track
                )
        ),
        "{returned:?}"
    );
}

#[test]
fn marquee_streams_the_selection_then_settles_and_background_clears_it() {
    let mut harness = harness(Rig::new());
    harness.state_mut().take();
    let (first, second) = (harness.state().first, harness.state().second);
    let body = harness.state().rect("timeline.marquee");
    let near = harness.state().rect(&format!("timeline.item.{}", first.0));
    let far = harness.state().rect(&format!("timeline.item.{}", second.0));
    let start = egui::pos2(body.left() + 4.0, body.top() + 4.0);
    drag(&mut harness, start, far.center());
    let returned = harness.state_mut().take();
    let swept: Vec<_> = returned
        .iter()
        .filter_map(|action| match action {
            Action::Marquee(items) => Some(items.clone()),
            _ => None,
        })
        .collect();
    assert!(!swept.is_empty(), "{returned:?}");
    let last = swept.last().unwrap();
    assert!(last.contains(&first) && last.contains(&second) && near.is_positive());
    assert!(matches!(returned.last(), Some(Action::SettleSelection)));
    assert!(!harness.state().state.marquee_active());
    // A plain click on an empty lane deselects.
    click(
        &mut harness,
        egui::pos2(body.left() + 4.0, body.top() + 4.0),
    );
    assert!(matches!(
        harness.state_mut().take().as_slice(),
        [Action::ClearSelection]
    ));
}

#[test]
fn scene_sheet_returns_actions_and_a_refused_rename_keeps_its_draft() {
    let mut rig = Rig::new();
    rig.sheet = true;
    rig.state.scenes.open();
    let scene = rig.session.project().data().current_scene_id;
    let mut harness = harness(rig);
    for id in ["scene.create", "scene.select", "scene.close"] {
        harness.state().rect(id);
    }
    harness.state_mut().take();
    let at = harness.state().rect("scene.create").center();
    click(&mut harness, at);
    let returned = harness.state_mut().take();
    assert!(
        matches!(returned.as_slice(), [Action::Activate(id)] if id == "scene.create"),
        "{returned:?}"
    );
    let row = format!("scene.row.{}", scene.0);
    let at = harness.state().rect(&row).center();
    click(&mut harness, at);
    let returned = harness.state_mut().take();
    assert!(
        matches!(returned.as_slice(), [Action::Activate(id)] if *id == row),
        "{returned:?}"
    );
    // Committing a rename hands the text to the frame and closes the field.
    harness
        .state_mut()
        .state
        .scenes
        .begin_rename(scene, "Renamed".into());
    harness.run_steps(3);
    harness.key_press(egui::Key::Enter);
    harness.run_steps(2);
    let returned = harness.state_mut().take();
    assert!(
        matches!(
            returned.as_slice(),
            [Action::RenameScene { scene: renamed, name }] if *renamed == scene && name == "Renamed"
        ),
        "{returned:?}"
    );
    assert!(!harness.state().state.scenes.is_renaming());
    // A refusal from the frame puts the typed text back for another try.
    harness
        .state_mut()
        .state
        .keep_scene_rename(scene, "Renamed".into());
    assert!(harness.state().state.scenes.is_renaming());
    // Closing the sheet stops it asking for anything.
    harness.state_mut().state.scenes.close();
    harness.run_steps(2);
    assert!(harness.state_mut().take().is_empty());
}

#[test]
fn escape_hooks_cancel_only_what_is_in_progress() {
    let mut state = State::default();
    assert!(!state.cancel_drag() && !state.cancel_bookmark_drag() && !state.cancel_track_edit());
    assert!(state.cancel_marquee().is_none());
    state.snap_indicator = Some(Tick(5));
    state.track_rename = Some((TrackId(1), "x".into()));
    state.track_rename_focus = true;
    assert!(state.renaming_track());
    assert!(state.cancel_track_edit());
    assert!(!state.renaming_track() && !state.track_rename_focus);
    state.marquee = Some((egui::Pos2::ZERO, BTreeSet::from([ItemId(3)]), false));
    assert!(state.marquee_active());
    assert_eq!(state.cancel_marquee(), Some(BTreeSet::from([ItemId(3)])));
    state.abandon_gesture();
    assert_eq!(state.snap_indicator(), None);

    // Escape closes the colour window first, then the marker editor.
    state.open_bookmark_popover(BookmarkId(7), egui::Pos2::ZERO, String::new(), Tick::ZERO);
    state.bookmark_popover.as_mut().unwrap().picker = Some(
        crate::services::color_picker::Picker::new(project::Color([1, 2, 3, 255])),
    );
    assert!(state.escape_bookmark());
    assert_eq!(state.bookmark_popover(), Some(BookmarkId(7)));
    assert!(state.escape_bookmark());
    assert_eq!(state.bookmark_popover(), None);
    assert!(!state.escape_bookmark());
}

#[test]
fn zoom_steps_and_a_restored_view_replace_the_viewport() {
    let mut state = State::default();
    state.restore_view(10.0, 5.0);
    assert_eq!((state.zoom(), state.scroll()), (10.0, 5.0));
    state.zoom_by(2.0, Tick(60 * SECOND), Tick(30 * SECOND));
    assert!(state.zoom() > 10.0, "zooming in raises the scale");
    state.restore_view(40.0, 0.0);
    assert_eq!((state.zoom(), state.scroll()), (40.0, 0.0));
}

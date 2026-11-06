//! Verifies the timeline interface: the actions `show` returns for each gesture.
#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]
use super::*;
use crate::editor::frame::Thumbnails;
use egui_kittest::Harness;
use engine::{NewItem, Session};
use project::{Project, ProjectId};
use std::cell::RefCell;
use std::collections::BTreeMap;
use time::{SourceRange, TICKS_PER_SECOND};

const SECOND: i64 = TICKS_PER_SECOND;

/// Everything `show` reads, owned by the test so the panel can be driven alone.
struct Rig {
    session: Session,
    playback: crate::playback::Playback,
    selection: BTreeSet<ItemId>,
    disabled: BTreeSet<&'static str>,
    preferences: crate::catalog::Preferences,
    images: BTreeMap<AssetId, engine::thumbnails::RetainedThumbnail>,
    textures: RefCell<BTreeMap<AssetId, egui::TextureHandle>>,
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
        let playback = crate::playback::Playback::new(
            &session,
            engine::ByteBudget::new(64 << 20).unwrap(),
            std::sync::Arc::new(crate::playback::Files),
        )
        .unwrap();
        Self {
            session,
            playback,
            selection: BTreeSet::new(),
            disabled: BTreeSet::new(),
            preferences: crate::catalog::Preferences::default(),
            images: BTreeMap::new(),
            textures: RefCell::default(),
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
            keybindings: &self.preferences.keybindings,
            scope: crate::selection::SelectionScope::None,
            keyframes: &keyframes,
            thumbnails: Thumbnails {
                images: &self.images,
                textures: &self.textures,
            },
            disabled: self.disabled.clone(),
            effect_targets: None,
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
            [Action::Command { id: "track.set_magnet", params }] if params["enabled"] == false
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
            [Action::Command { id: "track.set_locked", params }]
                if params["track"] == serde_json::json!(track) && params["locked"] == true
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

//! Transition catalog interaction through the real assets widgets.
#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]
use egui_kittest::{
    Harness,
    kittest::{By, Queryable},
};
use engine::{ByteBudget, Command, Session};
use project::{
    Color, Item, ItemId, ItemParams, OverlayParams, Project, ProjectId, TransitionAlignment,
};
use std::sync::Arc;
use time::{SourceRange, TICKS_PER_SECOND, Tick};
use vibecut::{
    Editor, egui,
    i18n::Locale,
    panels::AssetsTab,
    preferences::Preferences,
    services::playback::{Files, Playback},
};

const SECOND: i64 = TICKS_PER_SECOND;

fn editor(duration: i64) -> Editor {
    let mut data = Project::empty(ProjectId(1), "Transitions".into())
        .unwrap()
        .into_data();
    data.scene_mut().timeline.main.items = (0..3)
        .map(|n| Item {
            hidden: false,
            id: ItemId(n + 1),
            source_asset: None,
            source_range: SourceRange::new(Tick(0), Tick(duration)).unwrap(),
            start: Tick(n as i64 * duration),
            content: ItemParams::Overlay(OverlayParams {
                color: Color([100, 150, 200, 255]),
            }),
            retime: Default::default(),
            animations: Default::default(),
            effects: vec![],
            masks: vec![],
            blend_mode: Default::default(),
        })
        .collect();
    let session = Session::new(Project::new(data).unwrap(), 100).unwrap();
    let playback = Playback::new(
        &session,
        ByteBudget::new(64 << 20).unwrap(),
        Arc::new(Files),
    )
    .unwrap();
    Editor::new(session, playback, Preferences::default())
}
fn build(editor: Editor) -> Harness<'static, Editor> {
    crate::gpu::builder()
        .with_size(egui::vec2(1727.0, 1029.0))
        .build_ui_state(|ui, editor: &mut Editor| editor.ui(ui), editor)
}
fn click(h: &mut Harness<Editor>, id: &str) {
    h.get(By::new().predicate(|node| node.author_id() == Some(id)))
        .click();
    h.run_steps(3);
}
/// Activate a control by its automation id even when the scrolling grid clips it.
fn activate(h: &mut Harness<Editor>, id: &str) {
    h.get(By::new().predicate(|node| node.author_id() == Some(id)))
        .click_accesskit();
    h.run_steps(3);
}
fn widget(h: &Harness<Editor>, id: &str) -> vibecut::WidgetState {
    h.state()
        .inspect()
        .unwrap()
        .widgets
        .into_iter()
        .find(|widget| widget.id == id)
        .unwrap_or_else(|| panic!("missing {id}"))
}
fn open(h: &mut Harness<Editor>) {
    click(h, "assets.tab.transitions");
}
fn select_pair(h: &mut Harness<Editor>) {
    h.state_mut().selection = [ItemId(1), ItemId(2)].into_iter().collect();
    h.run_steps(3);
}
fn transition(h: &Harness<Editor>) -> project::Transition {
    h.state()
        .session
        .project()
        .data()
        .scene()
        .timeline
        .transitions[0]
        .clone()
}
fn begin_drag(h: &mut Harness<Editor>, kind: &str) -> egui::Pos2 {
    let start = widget(h, &format!("transitions.tile.{kind}"))
        .rect
        .center_top()
        + egui::vec2(0.0, 24.0);
    h.hover_at(start);
    h.run_steps(1);
    h.drag_at(start);
    h.run_steps(1);
    h.hover_at(start + egui::vec2(20.0, 0.0));
    h.run_steps(2);
    start
}
fn drop_at(h: &mut Harness<Editor>, point: egui::Pos2) {
    h.hover_at(point);
    h.run_steps(2);
    h.drop_at(point);
    h.run_steps(3);
}
fn cut(h: &Harness<Editor>) -> egui::Pos2 {
    let a = widget(h, "timeline.item.1").rect;
    let b = widget(h, "timeline.item.2").rect;
    egui::pos2((a.right() + b.left()) / 2.0, a.center().y)
}

#[test]
fn every_definition_applies_once_and_round_trips_with_history() {
    let mut h = build(editor(2 * SECOND));
    open(&mut h);
    select_pair(&mut h);
    for definition in project::TRANSITION_DEFINITIONS {
        let before = h.state().session.project().clone();
        let history = h.state().session.history_len().0;
        activate(&mut h, &format!("transitions.add.{}", definition.kind));
        assert_eq!(
            h.state()
                .session
                .project()
                .data()
                .scene()
                .timeline
                .transitions
                .len(),
            1,
            "{} did not apply",
            definition.kind
        );
        let tr = transition(&h);
        assert_eq!(tr.kind, definition.kind);
        assert_eq!(
            (tr.start, tr.duration, tr.alignment),
            (
                Tick(2 * SECOND - SECOND / 4),
                Tick(SECOND / 2),
                TransitionAlignment::Center
            )
        );
        assert_eq!(h.state().session.history_len().0, history + 1);
        assert!(!widget(&h, &format!("transitions.add.{}", definition.kind)).enabled);
        let applied = h.state().session.project().clone();
        let decoded = project::decode(&project::encode(&applied, false).unwrap())
            .unwrap()
            .project;
        assert_eq!(&decoded, applied.as_ref());
        h.state_mut().activate("edit.undo").unwrap();
        h.run_steps(2);
        assert_eq!(h.state().session.project(), &before);
        h.state_mut().activate("edit.redo").unwrap();
        h.run_steps(2);
        assert_eq!(h.state().session.project(), &applied);
        h.state_mut().activate("edit.undo").unwrap();
        h.run_steps(2);
    }
    let target = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target");
    std::fs::create_dir_all(&target).unwrap();
    h.hover_at(egui::pos2(500.0, 400.0));
    h.run_steps(3);
    h.render()
        .unwrap()
        .save(target.join("p1-63-transitions.png"))
        .unwrap();
}

#[test]
fn click_at_playhead_cut_clamps_default_to_short_handles() {
    let mut h = build(editor(SECOND / 10));
    open(&mut h);
    assert!(!widget(&h, "transitions.add.cross_dissolve").enabled);
    let project = h.state().session.project().clone();
    h.state_mut()
        .playback
        .seek(Tick(SECOND / 10), false, project)
        .unwrap();
    h.run_steps(3);
    click(&mut h, "transitions.tile.cross_dissolve");
    assert_eq!(
        (transition(&h).start, transition(&h).duration),
        (Tick::ZERO, Tick(SECOND / 5))
    );
}

#[test]
fn invalid_targets_are_disabled_with_reasons_without_history() {
    for state in [
        "empty",
        "gap",
        "locked",
        "unrelated",
        "handles",
        "overlap",
        "busy",
    ] {
        let mut editor = editor(2 * SECOND);
        let mut data = editor.session.project().as_ref().clone().into_data();
        match state {
            "empty" => data.scene_mut().timeline.main.items.clear(),
            "gap" => {
                data.scene_mut().timeline.main.items[1].start = Tick(2 * SECOND + 1);
                data.scene_mut().timeline.main.items[2].start = Tick(4 * SECOND + 1);
            }
            "locked" => data.scene_mut().timeline.main.locked = true,
            "handles" => {
                let asset =
                    testkit::source::asset(1, std::env::temp_dir().join("transition-handles.mp4"))
                        .format("synthetic")
                        .duration(Tick(2 * SECOND))
                        .stream(
                            testkit::source::video()
                                .codec("rgba")
                                .time_base(media::TimeBase::new(1, 30).unwrap())
                                .duration(Tick(2 * SECOND))
                                .size(4, 4)
                                .build(),
                        )
                        .build();
                data.assets.push(asset);
                data.scene_mut().timeline.main.items[0].source_asset = Some(media::AssetId(1));
                data.scene_mut().timeline.main.items[0].content =
                    ItemParams::Video(project::VideoParams {
                        source_audio_enabled: false,
                        volume: 0.0,
                        muted: false,
                        fades: Default::default(),
                        crop: None,
                    });
            }
            "overlap" => {
                let track = data.scene().timeline.main.id;
                data.scene_mut()
                    .timeline
                    .transitions
                    .push(project::Transition {
                        id: project::TransitionId(9),
                        track,
                        from: ItemId(2),
                        to: ItemId(3),
                        from_pieces: vec![],
                        to_pieces: vec![],
                        kind: "cross_dissolve".into(),
                        start: Tick(2 * SECOND),
                        duration: Tick(4 * SECOND),
                        alignment: TransitionAlignment::Center,
                        params: Default::default(),
                    });
            }
            _ => {}
        }
        *editor.session = Session::new(Project::new(data).unwrap(), 100).unwrap();
        let mut h = build(editor);
        open(&mut h);
        select_pair(&mut h);
        if state == "unrelated" {
            h.state_mut().selection = [ItemId(1), ItemId(3)].into_iter().collect();
            h.run_steps(2);
        }
        if state == "busy" {
            h.state_mut().session.begin_gesture().unwrap();
            h.run_steps(2);
        }
        let button = widget(&h, "transitions.add.cross_dissolve");
        assert!(!button.enabled, "{state}");
        assert!(button.tooltip.is_some(), "{state} gives a reason");
        let before = h.state().session.project().clone();
        let history = h.state().session.history_len();
        h.hover_at(button.rect.center());
        h.run_steps(2);
        h.event(egui::Event::PointerButton {
            pos: button.rect.center(),
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: Default::default(),
        });
        h.run_steps(1);
        h.event(egui::Event::PointerButton {
            pos: button.rect.center(),
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: Default::default(),
        });
        h.run_steps(2);
        assert_eq!(h.state().session.project(), &before);
        assert_eq!(h.state().session.history_len(), history);
    }
}

#[test]
fn coincident_cuts_require_selection_and_exact_playhead_boundaries() {
    let mut editor = editor(2 * SECOND);
    let mut data = editor.session.project().as_ref().clone().into_data();
    let mut upper = data.scene().timeline.main.clone();
    upper.id = project::TrackId(10);
    for item in &mut upper.items {
        item.id.0 += 10;
    }
    data.scene_mut().timeline.insert_track(upper);
    *editor.session = Session::new(Project::new(data).unwrap(), 100).unwrap();
    let mut h = build(editor);
    open(&mut h);
    for (at, selected, enabled) in [
        (2 * SECOND - 1, vec![ItemId(11)], false),
        (2 * SECOND + 1, vec![ItemId(11)], false),
        (2 * SECOND, vec![], false),
        (2 * SECOND, vec![ItemId(1), ItemId(12)], false),
        (2 * SECOND, vec![ItemId(1), ItemId(2), ItemId(3)], false),
        (2 * SECOND, vec![ItemId(11)], true),
    ] {
        h.state_mut().selection = selected.into_iter().collect();
        let project = h.state().session.project().clone();
        h.state_mut()
            .playback
            .seek(Tick(at), false, project)
            .unwrap();
        h.run_steps(3);
        assert_eq!(
            widget(&h, "transitions.add.cross_dissolve").enabled,
            enabled
        );
    }
    click(&mut h, "transitions.add.cross_dissolve");
    assert_eq!(transition(&h).track, project::TrackId(10));
}

#[test]
fn drag_to_cut_is_one_edit_and_cancel_misses_stale_or_reversed_drags_are_inert() {
    for mode in [
        "apply", "escape", "miss", "reverse", "stale", "tab", "edge", "outside", "locked",
        "present", "focus",
    ] {
        let mut editor = editor(2 * SECOND);
        if mode == "locked" {
            let track = editor.session.project().data().scene().timeline.main.id;
            editor
                .session
                .apply(engine::Command::SetTrackLocked {
                    track,
                    locked: true,
                })
                .unwrap();
        }
        if mode == "present" {
            let track = editor.session.project().data().scene().timeline.main.id;
            editor
                .session
                .apply(Command::AddTransition {
                    track,
                    from: ItemId(1),
                    to: ItemId(2),
                    kind: "cross_dissolve".into(),
                    duration: Tick(SECOND / 2),
                    alignment: TransitionAlignment::Center,
                })
                .unwrap();
        }
        let mut h = build(editor);
        open(&mut h);
        let history = h.state().session.history_len().0;
        let before = h.state().session.project().clone();
        let target = cut(&h);
        let start = begin_drag(&mut h, "dip_to_black");
        match mode {
            "escape" => {
                h.key_press(egui::Key::Escape);
                h.run_steps(2);
            }
            "stale" => {
                h.state_mut()
                    .session
                    .apply(Command::RenameProject {
                        name: "Changed".into(),
                    })
                    .unwrap();
                h.run_steps(2);
            }
            "tab" => {
                h.state_mut().preferences.assets_tab = AssetsTab::Media;
                h.run_steps(2);
            }
            "focus" => {
                h.input_mut().focused = false;
                h.run_steps(2);
            }
            _ => {}
        }
        let target = match mode {
            "miss" => target + egui::vec2(40.0, 0.0),
            "reverse" => start,
            "edge" => target + egui::vec2(12.0, 0.0),
            "outside" => target + egui::vec2(13.0, 0.0),
            _ => target,
        };
        drop_at(&mut h, target);
        let applied = matches!(mode, "apply" | "edge");
        assert_eq!(
            h.state()
                .session
                .project()
                .data()
                .scene()
                .timeline
                .transitions
                .len(),
            usize::from(applied || mode == "present"),
            "{mode}"
        );
        assert_eq!(
            h.state().session.history_len().0,
            history + usize::from(applied || mode == "stale"),
            "{mode}"
        );
        if applied {
            h.state_mut().activate("edit.undo").unwrap();
            h.run_steps(2);
            assert!(
                h.state()
                    .session
                    .project()
                    .data()
                    .scene()
                    .timeline
                    .transitions
                    .is_empty()
            );
            h.state_mut().activate("edit.redo").unwrap();
            h.run_steps(2);
            assert_eq!(transition(&h).kind, "dip_to_black");
        } else if mode != "stale" {
            assert_eq!(h.state().session.project(), &before, "{mode}");
        }
    }
}

#[test]
fn localized_search_and_category_filter_keep_preview_tiles_available() {
    let mut editor = editor(SECOND);
    editor.preferences.locale = Locale::Vi;
    let mut h = build(editor);
    open(&mut h);
    click(&mut h, "transitions.category.Basic");
    assert_eq!(widget(&h, "transitions.category.all").rect.height(), 24.0);
    assert_eq!(
        widget(&h, "transitions.category.Basic").rect.height(),
        24.0
    );
    click(&mut h, "assets.search");
    h.event(egui::Event::Text("trắng".into()));
    h.run_steps(3);
    assert!(
        widget(&h, "transitions.tile.dip_to_white")
            .rect
            .is_positive()
    );
    assert!(
        !h.state()
            .inspect()
            .unwrap()
            .widgets
            .iter()
            .any(|widget| widget.id == "transitions.tile.cross_dissolve")
    );
    h.event(egui::Event::Text(" no match".into()));
    h.run_steps(3);
    assert!(
        !h.state()
            .inspect()
            .unwrap()
            .widgets
            .iter()
            .any(|widget| widget.id.starts_with("transitions.tile."))
    );
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.key_press(egui::Key::Backspace);
    h.run_steps(3);
    let tile = widget(&h, "transitions.tile.cross_dissolve");
    h.hover_at(tile.rect.center());
    h.run_steps(8);
    assert!(
        h.state()
            .inspect()
            .unwrap()
            .widgets
            .iter()
            .any(|widget| widget.id == "transitions.preview.cross_dissolve.ready")
    );
}

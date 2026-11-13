#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
//! Desktop behaviour through the real panel, shared engine jobs and offline adapters.
use captions::CueTrack;
use egui_kittest::{
    Harness,
    kittest::{By, NodeT, Queryable},
};
use engine::{ByteBudget, Command, Session, WordId, providers::Cost};
use project::ItemParams;
use std::{
    collections::BTreeSet,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use time::{TICKS_PER_SECOND, Tick};
use vibecut::{
    AssetsTab, Editor, Subtitles, SubtitlesInspection, egui,
    i18n::{Locale, Text},
    preferences::{PreferenceStore, SubtitleOptions as Options},
    services::ai_access::{Choice, Opener, SharedConsent, TextOpener},
};
#[path = "support/subtitles.rs"]
mod fixture;
use fixture::{editor, editor_with_cues, lang};
fn harness(editor: Editor, scale: f32) -> Harness<'static, Editor> {
    crate::gpu::builder()
        .with_size(egui::vec2(1440.0, 1000.0))
        .with_pixels_per_point(scale)
        .build_ui_state(|ui, e: &mut Editor| e.ui(ui), editor)
}
/// Whether a widget with this automation id is on screen now.
fn has(h: &Harness<'_, Editor>, id: &str) -> bool {
    h.query_all(By::new().predicate(|n| n.author_id() == Some(id)))
        .next()
        .is_some()
}
fn click(h: &mut Harness<'_, Editor>, id: &str) {
    h.query_all(By::new().predicate(|n| n.author_id() == Some(id)))
        .next()
        .unwrap_or_else(|| panic!("missing {id}; {:?}", panel(h.state())))
        .click_accesskit();
    h.run_steps(3);
}
/// A pointer click holding `modifiers`, for Shift and Ctrl selection.
fn click_modifiers(h: &mut Harness<'_, Editor>, id: &str, modifiers: egui::Modifiers) {
    // The rows can sit below the fold of the scrolling panel: bring the row into view first,
    // then click it with the pointer so hit-testing decides what the click lands on.
    h.get(By::new().predicate(|n| n.author_id() == Some(id)))
        .scroll_to_me();
    h.run_steps(3);
    h.get(By::new().predicate(|n| n.author_id() == Some(id)))
        .click_modifiers(modifiers);
    h.run_steps(3);
}
fn wait(h: &mut Harness<'_, Editor>, ready: impl Fn(&Editor) -> bool) {
    let until = Instant::now() + Duration::from_secs(20);
    loop {
        h.run_steps(1);
        if ready(h.state()) {
            h.run_steps(3);
            return;
        }
        assert!(
            Instant::now() < until,
            "subtitle timeout {:?}",
            h.state().inspect().unwrap().subtitles
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}
/// The AccessKit node id egui assigned to the widget behind one automation id.
fn node_id(h: &Harness<'_, Editor>, id: &str) -> String {
    format!(
        "{:?}",
        h.get(By::new().predicate(|n| n.author_id() == Some(id)))
            .accesskit_node()
            .id()
    )
}
fn track(e: &Editor) -> CueTrack {
    e.session
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|t| &t.items)
        .find_map(|i| {
            if let ItemParams::Caption(p) = &i.content {
                Some(p.cues.clone())
            } else {
                None
            }
        })
        .unwrap()
}
fn panel(e: &Editor) -> SubtitlesInspection {
    e.inspect().unwrap().subtitles.unwrap()
}

#[test]
fn caption_bank_name_refusals_and_cap_preserve_the_real_project() {
    let mut h = harness(editor("vi"), 1.0);
    h.run_steps(3);
    h.state_mut().activate("subtitles.style.box").unwrap();
    h.run_steps(3);
    let item = panel(h.state()).item.unwrap();
    h.state_mut()
        .session
        .apply(Command::SaveCaptionPreset {
            item,
            name: "Tiếng Việt".into(),
        })
        .unwrap();
    h.run_steps(3);
    for name in [" Leading", "Tie\u{302}\u{301}ng Vie\u{323}\u{302}t"] {
        let before = h.state().session.project().clone();
        let history = h.state().session.history_len();
        click(&mut h, "subtitles.style.save");
        h.get(By::new().predicate(|n| n.author_id() == Some("subtitles.bank.name")))
            .type_text(name);
        h.key_press(egui::Key::Enter);
        h.run_steps(3);
        assert_eq!(h.state().session.project(), &before);
        assert_eq!(h.state().session.history_len(), history);
        assert!(
            h.query_all(By::new().predicate(|n| n.author_id() == Some("subtitles.bank.name")))
                .next()
                .is_some()
        );
        h.key_press(egui::Key::Escape);
        h.run_steps(3);
    }
    for n in 1..64 {
        h.state_mut()
            .session
            .apply(Command::SaveCaptionPreset {
                item,
                name: format!("Look {n}").into(),
            })
            .unwrap();
    }
    h.run_steps(3);
    let before = h.state().session.project().clone();
    assert_eq!(before.data().caption_presets.len(), 64);
    assert!(!h.state().action_enabled("subtitles.style.save"));
    assert!(h.state_mut().activate("subtitles.style.save").is_err());
    assert!(
        h.get(By::new().predicate(|n| n.author_id() == Some("subtitles.style.save")))
            .accesskit_node()
            .is_disabled()
    );
    assert!(h.state().action_enabled("subtitles.style.manage"));
    assert_eq!(h.state().session.project(), &before);
}

#[test]
fn caption_bank_name_invalidation_precedes_enter_and_blur() {
    struct HeldImport;
    impl vibecut::ImportProbe for HeldImport {
        fn import(
            &self,
            _: media::AssetId,
            _: std::path::PathBuf,
            cancelled: Arc<std::sync::atomic::AtomicBool>,
        ) -> media::Result<media::ImportedSource> {
            while !cancelled.load(Ordering::Acquire) {
                std::thread::sleep(Duration::from_millis(1));
            }
            Err(media::MediaError::Cancelled)
        }
    }
    for cause in [
        "time",
        "snapshot",
        "undo",
        "lock",
        "deleted",
        "scene",
        "project",
        "tab",
        "gesture",
        "export",
        "models",
        "saving",
        "import",
        "style",
        "bank",
        "bank-delete",
    ] {
        let mut e = editor("vi");
        if cause == "import" {
            e.playback = vibecut::services::playback::Playback::new(
                &e.session,
                ByteBudget::new(1 << 30).unwrap(),
                Arc::new(vibecut::services::playback::Files),
            )
            .unwrap();
            e.set_import_probe(Arc::new(HeldImport)).unwrap();
        }
        let mut h = harness(e, 1.0);
        h.run_steps(3);
        h.state_mut().activate("subtitles.style.box").unwrap();
        h.run_steps(3);
        let item = panel(h.state()).item.unwrap();
        h.state_mut()
            .session
            .apply(Command::SaveCaptionPreset {
                item,
                name: "Existing".into(),
            })
            .unwrap();
        h.run_steps(3);
        click(&mut h, "subtitles.style.save");
        h.get(By::new().predicate(|n| n.author_id() == Some("subtitles.bank.name")))
            .type_text("Stale input");
        h.run_steps(2);
        let root = tempfile::tempdir().unwrap();
        match cause {
            "style" => {
                h.state_mut()
                    .session
                    .apply(Command::ApplyCaptionStyle {
                        item,
                        style: engine::CaptionStyleApply::Preset {
                            preset: engine::CaptionPreset::Outline,
                        },
                    })
                    .unwrap();
            }
            "bank" | "bank-delete" => {
                let preset = h.state().session.project().data().caption_presets[0].id;
                let cmd = if cause == "bank" {
                    Command::RenameCaptionPreset {
                        preset,
                        name: "Changed".into(),
                    }
                } else {
                    Command::DeleteCaptionPreset { preset }
                };
                h.state_mut().session.apply(cmd).unwrap();
            }
            "models" => {
                h.state_mut().set_model_cache(std::sync::Arc::new(
                    vibecut::services::ai_access::ModelCache::new(
                        ByteBudget::new(64 << 20).unwrap(),
                    ),
                ));
                h.state_mut().activate("models.open").unwrap()
            }
            "saving" => {
                let session = Session::new((**h.state().session.project()).clone(), 20).unwrap();
                let mut saving = engine::recovery::Autosave::new(
                    &root.path().join("recovery"),
                    session,
                    ByteBudget::new(64 << 20).unwrap(),
                    Arc::new(engine::jobs::SystemClock::default()),
                    Default::default(),
                )
                .unwrap();
                saving.save_to(root.path().join("saved.vibecut")).unwrap();
                h.state_mut().session =
                    vibecut::services::lifecycle::EditorSession::Persistent(Box::new(saving));
                assert!(h.state_mut().activate("subtitles.style.save").is_err());
                h.state_mut().session.autosave().unwrap().flush().unwrap();
                h.run_steps(3); // Acknowledge save-as's new project identity before the stale key.
            }
            "time" => h
                .state_mut()
                .dispatch(vibecut::AppCommand::Seek(Tick(TICKS_PER_SECOND)))
                .unwrap(),
            "snapshot" => {
                h.state_mut()
                    .session
                    .apply(Command::RenameProject {
                        name: "Changed".into(),
                    })
                    .unwrap();
            }
            "undo" => {
                h.state_mut().session.undo().unwrap();
            }
            "lock" => {
                h.state_mut()
                    .session
                    .apply(Command::SetTrackLocked {
                        track: project::TrackId(1),
                        locked: true,
                    })
                    .unwrap();
            }
            "deleted" => {
                h.state_mut()
                    .session
                    .apply(Command::Delete { items: vec![item] })
                    .unwrap();
            }
            "scene" => {
                let out = h
                    .state_mut()
                    .session
                    .apply(Command::CreateScene {
                        name: "Other".into(),
                    })
                    .unwrap();
                let engine::CreatedId::Scene(scene) = out.created[0] else {
                    panic!("scene")
                };
                h.state_mut()
                    .session
                    .apply(Command::SwitchScene { scene })
                    .unwrap();
            }
            "project" => {
                *h.state_mut().session = Session::new(
                    project::Project::empty(project::ProjectId(99), "Other".into()).unwrap(),
                    20,
                )
                .unwrap();
            }
            "tab" => {
                h.state_mut().preferences.assets_tab = AssetsTab::Media;
                h.run_steps(3);
                h.state_mut().preferences.assets_tab = AssetsTab::Captions;
            }
            "gesture" => h.state_mut().session.begin_gesture().unwrap(),
            "export" => h.state_mut().activate("file.export").unwrap(),
            "import" => {
                h.state_mut()
                    .start_import(vec![std::env::temp_dir().join("393-missing.mp4")])
                    .unwrap();
                assert!(h.state().importing());
            }
            _ => panic!("unknown case"),
        }
        let before = h.state().session.project().clone();
        let history = h.state().session.history_len();
        h.key_press(egui::Key::Enter);
        h.run_steps(3);
        assert!(
            h.query_all(By::new().predicate(|n| n.author_id() == Some("subtitles.bank.name")))
                .next()
                .is_none(),
            "{cause}"
        );
        assert_eq!(h.state().session.project(), &before, "{cause}");
        assert_eq!(h.state().session.history_len(), history, "{cause}");
        if cause == "gesture" {
            h.state_mut().session.cancel_gesture().unwrap();
        }
    }
}

#[test]
fn caption_bank_preflight_preserves_properties_edits_and_pickers() {
    for control in [
        "focus",
        "unfocused-draft",
        "font",
        "color",
        "scrub",
        "reorder",
        "inline",
    ] {
        let mut e = editor("vi");
        let caption = e
            .session
            .project()
            .data()
            .scene()
            .timeline
            .iter()
            .flat_map(|t| &t.items)
            .find(|item| matches!(item.content, ItemParams::Caption(_)))
            .unwrap()
            .id;
        e.session
            .apply(Command::ApplyCaptionStyle {
                item: caption,
                style: engine::CaptionStyleApply::Preset {
                    preset: engine::CaptionPreset::Box,
                },
            })
            .unwrap();
        e.session
            .apply(Command::SaveCaptionPreset {
                item: caption,
                name: "Existing".into(),
            })
            .unwrap();
        let out = e
            .session
            .apply(Command::InsertItem {
                placement: engine::Placement::Auto,
                item: engine::NewItem {
                    start: Tick::ZERO,
                    source_range: time::SourceRange::new(Tick::ZERO, Tick(10 * TICKS_PER_SECOND))
                        .unwrap(),
                    source_asset: None,
                    content: ItemParams::Text(project::TextParams::new("Title")),
                    retime: Default::default(),
                    animations: Default::default(),
                },
            })
            .unwrap();
        let text = out
            .created
            .iter()
            .find_map(|id| {
                if let engine::CreatedId::Item(id) = id {
                    Some(*id)
                } else {
                    None
                }
            })
            .unwrap();
        e.select([text].into()).unwrap();
        if control == "reorder" {
            for id in ["blur1", "blur2"] {
                e.session
                    .apply(Command::AddEffect {
                        item: text,
                        effect: project::effect_definition("blur").unwrap().instance(id),
                    })
                    .unwrap();
            }
            e.preferences
                .properties_tabs
                .insert("text".into(), "effects".into());
        }
        let mut h = crate::gpu::builder()
            .with_size(egui::vec2(1440.0, 1000.0))
            .with_step_dt(0.05)
            .build_ui_state(|ui, e: &mut Editor| e.ui(ui), e);
        h.run_steps(3);
        if control != "inline" {
            click(&mut h, "subtitles.style.save");
            h.get(By::new().predicate(|n| n.author_id() == Some("subtitles.bank.name")))
                .type_text("Must not save");
            h.run_steps(2);
        }
        match control {
            "inline" => {
                let widgets = h.state().inspect().unwrap().widgets;
                let left = widgets
                    .iter()
                    .find(|w| w.id == "preview.transform.left")
                    .unwrap()
                    .rect
                    .center();
                let right = widgets
                    .iter()
                    .find(|w| w.id == "preview.transform.right")
                    .unwrap()
                    .rect
                    .center();
                let at = left.lerp(right, 0.5);
                for _ in 0..2 {
                    h.event(egui::Event::PointerMoved(at));
                    for pressed in [true, false] {
                        h.event(egui::Event::PointerButton {
                            pos: at,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        });
                    }
                }
                h.run_steps(3);
                assert!(
                    h.query_all(
                        By::new().predicate(|n| n.author_id() == Some("preview.text.edit"))
                    )
                    .next()
                    .is_some()
                );
                assert!(!h.state().session.gesture_active());
            }
            "focus" | "unfocused-draft" => {
                h.get(By::new().predicate(|n| n.author_id() == Some("inspector.text_size")))
                    .focus();
                h.run_steps(2);
                h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
                h.event(egui::Event::Text("83".into()));
                h.run_steps(2);
                if control == "unfocused-draft" {
                    h.state_mut().activate("inspector.details").unwrap();
                    h.run_steps(3);
                }
            }
            "font" => {
                click(&mut h, "inspector.text_font");
                assert!(h.state().inspect().unwrap().font_picker.open);
            }
            "color" => click(&mut h, "inspector.text_color.picker"),
            "scrub" | "reorder" => {
                let id = if control == "scrub" {
                    "inspector.text_size.scrub"
                } else {
                    "effects.row.blur2.handle"
                };
                let at = h
                    .state()
                    .inspect()
                    .unwrap()
                    .widgets
                    .iter()
                    .find(|w| w.id == id)
                    .unwrap()
                    .rect
                    .center();
                h.event(egui::Event::PointerMoved(at));
                h.run_steps(1);
                h.event(egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                });
                h.run_steps(1);
                h.event(egui::Event::PointerMoved(at + egui::vec2(20.0, 10.0)));
                h.run_steps(2);
            }
            _ => panic!("unknown case"),
        }
        let before = h.state().session.project().clone();
        let history = h.state().session.history_len();
        assert_eq!(
            before.data().caption_presets.len(),
            1,
            "{control}: stale blur must not save"
        );
        assert!(
            h.query_all(By::new().predicate(|n| n.author_id() == Some("subtitles.bank.name")))
                .next()
                .is_none(),
            "{control}"
        );
        for id in [
            "subtitles.style.save",
            "subtitles.style.manage",
            "subtitles.style.word_highlight",
        ] {
            assert!(!h.state().action_enabled(id), "{control}");
            assert!(h.state_mut().activate(id).is_err(), "{control}");
            assert_eq!(h.state().session.project(), &before, "{control}");
            assert_eq!(h.state().session.history_len(), history, "{control}");
            assert!(
                h.get(By::new().predicate(|n| n.author_id() == Some(id)))
                    .accesskit_node()
                    .is_disabled(),
                "{control}"
            );
        }
    }
}

#[test]
fn caption_presets_apply_all_cues_undo_reopen_and_render_localized_scenes() {
    let dir = tempfile::tempdir().unwrap();
    for locale in [Locale::En, Locale::Vi] {
        for scale in [1.0, 2.0] {
            let mut e = editor("vi");
            e.preferences.locale = locale;
            e.session
                .apply(Command::SetBackground {
                    color: project::Color([25, 50, 90, 255]),
                })
                .unwrap();
            e.session
                .apply(Command::SetCanvas {
                    width: 360,
                    height: 640,
                    preset: None,
                })
                .unwrap();
            let mut h = harness(e, scale);
            h.run_steps(3);
            click(&mut h, "subtitles.cue.cue-0");
            // A cue subset must not narrow a style operation.
            for (id, border) in [("outline", 1), ("box", 3)] {
                let before = h.state().session.project().clone();
                let history = h.state().session.history_len().0;
                click(&mut h, "subtitles.style");
                click(&mut h, &format!("subtitles.style.{id}"));
                let cues = track(h.state());
                assert_eq!(cues.styles.len(), 1);
                assert_eq!(cues.styles[0].border_style, border);
                assert_eq!(cues.styles[0].font_family.as_str(), "Inter");
                assert!(cues.styles[0].bold);
                assert!(
                    cues.cues
                        .iter()
                        .all(|cue| cue.style == Some(captions::CueStyleId(0)))
                );
                assert_eq!(h.state().session.history_len().0, history + 1);
                let styled = h.state().session.project().clone();
                h.state_mut().activate("edit.undo").unwrap();
                h.run_steps(3);
                assert_eq!(h.state().session.project(), &before);
                h.state_mut().activate("edit.redo").unwrap();
                h.run_steps(3);
                assert_eq!(h.state().session.project(), &styled);
                let file = dir.path().join("styled.vibecut");
                project::WriterLease::acquire(&file)
                    .unwrap()
                    .save(&styled)
                    .unwrap();
                assert_eq!(&project::load(&file).unwrap().project, styled.as_ref());
                h.state_mut()
                    .playback
                    .seek(Tick(2 * TICKS_PER_SECOND), false, styled)
                    .unwrap();
                wait(&mut h, |e| {
                    e.playback.frame().is_some_and(|frame| {
                        frame.generation == e.playback.generation()
                            && frame.at == Tick(2 * TICKS_PER_SECOND)
                    })
                });
                let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target");
                std::fs::create_dir_all(&root).unwrap();
                let image = h.render().unwrap();
                image
                    .save(root.join(format!("p1-59-caption-{locale:?}-{scale}x-{id}.png")))
                    .unwrap();
                if locale == Locale::En && scale == 1.0 && id == "box" {
                    image.save(root.join("p1-59-caption-style.png")).unwrap();
                }
            }
            let frame = h.state().playback.frame().unwrap().image.pixels.clone();
            for (name, blur) in [("Tiếng Việt", false), ("Same values", true)] {
                let before = h.state().session.project().clone();
                let history = h.state().session.history_len().0;
                click(&mut h, "subtitles.style.save");
                h.get(By::new().predicate(|n| n.author_id() == Some("subtitles.bank.name")))
                    .type_text(name);
                h.run_steps(2);
                if blur {
                    h.get(By::new().predicate(|n| n.author_id() == Some("preview.canvas")))
                        .click();
                } else {
                    h.key_press(egui::Key::Enter);
                }
                h.run_steps(3);
                assert_eq!(h.state().session.history_len().0, history + 1);
                assert_eq!(
                    h.state().session.project().data().scenes,
                    before.data().scenes
                );
                assert_eq!(
                    h.state()
                        .session
                        .project()
                        .data()
                        .caption_presets
                        .last()
                        .unwrap()
                        .name
                        .as_str(),
                    name
                );
                wait(&mut h, |e| {
                    e.playback.frame().is_some_and(|f| {
                        f.generation == e.playback.generation()
                            && f.at == Tick(2 * TICKS_PER_SECOND)
                    })
                });
                assert_eq!(h.state().playback.frame().unwrap().image.pixels, frame);
            }
            let bank = h.state().session.project().data().caption_presets.clone();
            assert_eq!(bank.len(), 2);
            assert_eq!(bank[0].style, bank[1].style);
            click(&mut h, "subtitles.style.manage");
            h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
            h.get(By::new().predicate(|n| n.author_id() == Some("subtitles.bank.name")))
                .type_text("Renamed look");
            h.run_steps(2);
            h.render()
                .unwrap()
                .save(
                    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
                        "../../target/p1-59-caption-bank-{locale:?}-{scale}x.png"
                    )),
                )
                .unwrap();
            click(&mut h, "subtitles.bank.confirm");
            assert_eq!(
                h.state().session.project().data().caption_presets[0]
                    .name
                    .as_str(),
                "Renamed look"
            );
            wait(&mut h, |e| {
                e.playback.frame().is_some_and(|f| {
                    f.generation == e.playback.generation() && f.at == Tick(2 * TICKS_PER_SECOND)
                })
            });
            assert_eq!(h.state().playback.frame().unwrap().image.pixels, frame);
            let before = h.state().session.project().clone();
            let history = h.state().session.history_len();
            click(&mut h, "subtitles.style.save");
            h.key_press(egui::Key::Escape);
            h.run_steps(3);
            assert_eq!(h.state().session.project(), &before);
            assert_eq!(h.state().session.history_len(), history);
            // Named rows select the live value instead of copying it into cue styles.
            h.state_mut().activate("subtitles.style.outline").unwrap();
            h.run_steps(3);
            let outlined = h.state().session.project().clone();
            let cue_values = track(h.state()).styles.clone();
            click(&mut h, "subtitles.style");
            click(&mut h, &format!("subtitles.style.saved.{}", bank[1].id.0));
            assert_eq!(track(h.state()).styles, cue_values);
            let ItemParams::Caption(selected) = &h
                .state()
                .session
                .project()
                .data()
                .scene()
                .timeline
                .iter()
                .flat_map(|track| &track.items)
                .find(|item| Some(item.id) == panel(h.state()).item)
                .unwrap()
                .content
            else {
                panic!("caption")
            };
            assert_eq!(selected.preset, Some(bank[1].id));
            let selected_project = h.state().session.project().clone();
            let overrides = captions::CaptionOverrides {
                size: Some(64.0),
                color: Some([240, 220, 40, 220]),
                ..Default::default()
            };
            let item = panel(h.state()).item.unwrap();
            h.state_mut()
                .session
                .apply(Command::ApplyCaptionStyle {
                    item,
                    style: engine::CaptionStyleApply::Saved {
                        preset: bank[1].id,
                        overrides: overrides.clone(),
                    },
                })
                .unwrap();
            let ItemParams::Caption(patched) = &h
                .state()
                .session
                .project()
                .data()
                .scene()
                .timeline
                .iter()
                .flat_map(|track| &track.items)
                .find(|value| value.id == item)
                .unwrap()
                .content
            else {
                panic!("caption")
            };
            assert_eq!(patched.overrides, overrides);
            assert_eq!(track(h.state()).styles, cue_values);
            h.state_mut().activate("edit.undo").unwrap();
            h.run_steps(3);
            assert_eq!(h.state().session.project(), &selected_project);
            h.state_mut().activate("edit.undo").unwrap();
            h.run_steps(3);
            assert_eq!(h.state().session.project(), &outlined);
            let first = panel(h.state()).item.unwrap();
            let out = h
                .state_mut()
                .session
                .apply(Command::DuplicateItems { items: vec![first] })
                .unwrap();
            let second = out
                .created
                .iter()
                .find_map(|id| {
                    if let engine::CreatedId::Item(id) = id {
                        Some(*id)
                    } else {
                        None
                    }
                })
                .unwrap();
            h.run_steps(3);
            click(&mut h, "subtitles.item");
            click(&mut h, &format!("subtitles.item.{}", second.0));
            let before = h.state().session.project().clone();
            click(&mut h, "subtitles.style");
            click(&mut h, &format!("subtitles.style.saved.{}", bank[1].id.0));
            let changed = h
                .state()
                .session
                .project()
                .data()
                .scene()
                .timeline
                .iter()
                .flat_map(|t| &t.items)
                .find(|i| i.id == second)
                .unwrap();
            let ItemParams::Caption(params) = &changed.content else {
                panic!("caption")
            };
            assert_eq!(params.preset, Some(bank[1].id));
            assert!(params.overrides.is_empty());
            let original = before
                .data()
                .scene()
                .timeline
                .iter()
                .flat_map(|t| &t.items)
                .find(|i| i.id == first)
                .unwrap();
            let current = h
                .state()
                .session
                .project()
                .data()
                .scene()
                .timeline
                .iter()
                .flat_map(|t| &t.items)
                .find(|i| i.id == first)
                .unwrap();
            assert_eq!(current, original);
            h.state_mut().activate("edit.undo").unwrap();
            h.run_steps(3);
            assert_eq!(h.state().session.project(), &before);
            let file = dir.path().join("bank.vibecut");
            project::WriterLease::acquire(&file)
                .unwrap()
                .save(h.state().session.project())
                .unwrap();
            let reopened = project::load(&file).unwrap().project;
            assert_eq!(
                reopened.data().caption_presets,
                h.state().session.project().data().caption_presets
            );
            *h.state_mut().session = Session::new(reopened, 30).unwrap();
            h.run_steps(3);
            let items = h
                .state()
                .session
                .project()
                .data()
                .scene()
                .timeline
                .iter()
                .flat_map(|t| &t.items)
                .filter(|i| matches!(i.content, ItemParams::Caption(_)))
                .map(|i| i.id)
                .collect();
            h.state_mut()
                .session
                .apply(Command::Delete { items })
                .unwrap();
            h.run_steps(3);
            assert!(h.state().action_enabled("subtitles.style.manage"));
            click(&mut h, "subtitles.style.manage");
            click(&mut h, "subtitles.bank.delete");
            assert_eq!(h.state().session.project().data().caption_presets.len(), 1);
            h.state_mut().activate("edit.undo").unwrap();
            h.run_steps(3);
            assert_eq!(h.state().session.project().data().caption_presets.len(), 2);
            h.state_mut().activate("edit.undo").unwrap();
            h.run_steps(3);
            click(&mut h, "subtitles.style");
            h.render()
                .unwrap()
                .save(
                    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
                        "../../target/p1-59-caption-{locale:?}-{scale}x-chooser.png"
                    )),
                )
                .unwrap();
        }
    }
}

#[test]
fn caption_style_override_popup_applies_resets_and_escape_cancels_one_undo() {
    fn caption(h: &Harness<Editor>, item: project::ItemId) -> project::CaptionParams {
        let ItemParams::Caption(params) = &h
            .state()
            .session
            .project()
            .data()
            .scene()
            .timeline
            .iter()
            .flat_map(|t| &t.items)
            .find(|i| i.id == item)
            .unwrap()
            .content
        else {
            panic!("caption")
        };
        params.clone()
    }
    let mut h = harness(editor("vi"), 1.0);
    h.run_steps(3);
    h.state_mut().activate("subtitles.style.outline").unwrap();
    h.run_steps(3);
    let item = panel(h.state()).item.unwrap();
    h.state_mut()
        .session
        .apply(Command::SaveCaptionPreset {
            item,
            name: "Look".into(),
        })
        .unwrap();
    h.run_steps(3);
    let preset = h.state().session.project().data().caption_presets[0].id;
    click(&mut h, "subtitles.style");
    click(&mut h, &format!("subtitles.style.saved.{}", preset.0));
    let selected = h.state().session.project().clone();
    let cue_values = track(h.state()).styles.clone();
    let history = h.state().session.history_len().0;
    fn field(h: &Harness<Editor>, id: &str, value: f64) {
        h.get(By::new().predicate(move |n| n.author_id() == Some(id)))
            .set_value_accesskit(value);
    }

    // The real Overrides popup commits one sparse patch as one undo step.
    click(&mut h, "subtitles.style.overrides");
    field(&h, "subtitles.style.size", 64.0);
    h.run_steps(2);
    click(&mut h, "subtitles.style.apply");
    assert_eq!(h.state().session.history_len().0, history + 1);
    assert_eq!(
        track(h.state()).styles,
        cue_values,
        "an override never rewrites cue values"
    );
    let patched = caption(&h, item);
    assert_eq!(patched.preset, Some(preset));
    assert_eq!(patched.overrides.size, Some(64.0));
    assert!(
        patched.overrides.max_lines.is_none(),
        "only the edited field is stored"
    );
    h.state_mut().activate("edit.undo").unwrap();
    h.run_steps(3);
    assert_eq!(h.state().session.project(), &selected);

    // Reset clears the patch instead of copying preset values into the item.
    click(&mut h, "subtitles.style.overrides");
    field(&h, "subtitles.style.size", 64.0);
    h.run_steps(2);
    click(&mut h, "subtitles.style.apply");
    click(&mut h, "subtitles.style.overrides");
    click(&mut h, "subtitles.style.reset");
    let reset = caption(&h, item);
    assert_eq!(reset.preset, Some(preset));
    assert!(reset.overrides.is_empty());
    assert_eq!(track(h.state()).styles, cue_values);
    assert_eq!(h.state().session.project(), &selected);

    // Escape drops the draft without a command or history entry.
    let before = h.state().session.project().clone();
    let history = h.state().session.history_len().0;
    click(&mut h, "subtitles.style.overrides");
    field(&h, "subtitles.style.size", 72.0);
    h.run_steps(2);
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert_eq!(h.state().session.project(), &before);
    assert_eq!(h.state().session.history_len().0, history);
    assert!(
        h.query_all(By::new().predicate(|n| n.author_id() == Some("subtitles.style.apply")))
            .next()
            .is_none(),
        "Escape closes the popup"
    );
}

#[test]
fn caption_style_actions_refuse_drafts_locks_transactions_and_busy_work() {
    let mut h = harness(editor("vi"), 1.0);
    h.run_steps(3);
    click(&mut h, "subtitles.cue.cue-0");
    let field = "subtitles.text.cue-0.vi";
    h.get(By::new().predicate(|n| n.author_id() == Some(field)))
        .focus();
    h.run_steps(2);
    let before = h.state().session.project().clone();
    assert!(!h.state().action_enabled("subtitles.style.box"));
    assert!(h.state_mut().activate("subtitles.style.box").is_err());
    h.get(By::new().predicate(|n| n.author_id() == Some(field)))
        .type_text(" pending");
    h.run_steps(2);
    assert!(h.state_mut().activate("subtitles.style.outline").is_err());
    assert_eq!(
        h.state().session.project(),
        &before,
        "preflight precedes draft flush"
    );
    h.ctx
        .memory_mut(|memory| memory.surrender_focus(memory.focused().unwrap()));
    h.run_steps(3);
    h.state_mut().activate("subtitles.select_all").unwrap();
    h.run_steps(3);
    let item = panel(h.state()).item.unwrap();
    h.state_mut()
        .session
        .apply(Command::SetTrackLocked {
            track: project::TrackId(1),
            locked: true,
        })
        .unwrap();
    h.run_steps(3);
    let before = h.state().session.project().clone();
    assert!(h.state_mut().activate("subtitles.style.box").is_err());
    assert_eq!(h.state().session.project(), &before);
    h.state_mut()
        .session
        .apply(Command::SetTrackLocked {
            track: project::TrackId(1),
            locked: false,
        })
        .unwrap();
    h.state_mut().session.begin_gesture().unwrap();
    assert!(h.state_mut().activate("subtitles.style.box").is_err());
    h.state_mut().session.cancel_gesture().unwrap();
    // Snapshot identity closes an open chooser after style/animation or target changes.
    h.run_steps(3);
    click(&mut h, "subtitles.style");
    h.state_mut()
        .session
        .apply(Command::SetItemField {
            item,
            patch: engine::ItemParamPatch::Opacity(0.5),
        })
        .unwrap();
    h.run_steps(3);
    assert_eq!(
        h.query_all(By::new().predicate(|n| n.author_id() == Some("subtitles.style.box")))
            .count(),
        0
    );
    let animation = h
        .state()
        .session
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|t| &t.items)
        .find(|i| i.id == item)
        .unwrap()
        .animations
        .clone();
    h.state_mut().activate("subtitles.style.box").unwrap();
    h.run_steps(3);
    assert_eq!(
        h.state()
            .session
            .project()
            .data()
            .scene()
            .timeline
            .iter()
            .flat_map(|t| &t.items)
            .find(|i| i.id == item)
            .unwrap()
            .animations,
        animation
    );
    assert_eq!(
        h.get(By::new().predicate(|n| n.author_id() == Some("subtitles.style")))
            .value()
            .as_deref(),
        Some("Box")
    );
    let mut content = h
        .state()
        .session
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|t| &t.items)
        .find(|i| i.id == item)
        .unwrap()
        .content
        .clone();
    let ItemParams::Caption(p) = &mut content else {
        panic!("caption")
    };
    p.cues.styles[0].primary_color = [20, 240, 60, 255];
    click(&mut h, "subtitles.style");
    h.state_mut()
        .session
        .apply(Command::SetItemParam {
            item,
            content: content.clone(),
        })
        .unwrap();
    h.run_steps(3);
    assert_eq!(
        h.get(By::new().predicate(|n| n.author_id() == Some("subtitles.style")))
            .value()
            .as_deref(),
        Some("Custom")
    );
    let ItemParams::Caption(p) = &mut content else {
        panic!("caption")
    };
    p.cues.cues[0].style = None;
    h.state_mut()
        .session
        .apply(Command::SetItemParam { item, content })
        .unwrap();
    h.run_steps(3);
    assert_eq!(
        h.get(By::new().predicate(|n| n.author_id() == Some("subtitles.style")))
            .value()
            .as_deref(),
        Some("Mixed")
    );
    let duplicated = h
        .state_mut()
        .session
        .apply(Command::DuplicateItems { items: vec![item] })
        .unwrap();
    let second = duplicated
        .created
        .iter()
        .find_map(|id| {
            if let engine::CreatedId::Item(item) = id {
                Some(*item)
            } else {
                None
            }
        })
        .unwrap();
    h.run_steps(3);
    click(&mut h, "subtitles.style");
    with_panel(h.state_mut(), |p, s, _| p.select_item(s, second).unwrap());
    h.run_steps(3);
    assert_eq!(
        h.query_all(By::new().predicate(|n| n.author_id() == Some("subtitles.style.box")))
            .count(),
        0
    );
    click(&mut h, "subtitles.style");
    click(&mut h, "subtitles.style.outline");
    let original = h
        .state()
        .session
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|t| &t.items)
        .find(|i| i.id == item)
        .unwrap();
    let ItemParams::Caption(p) = &original.content else {
        panic!("caption")
    };
    assert_eq!(p.cues.cues[0].style, None, "the other item remains mixed");
    click(&mut h, "subtitles.style");
    h.state_mut()
        .session
        .apply(Command::SetItemParam {
            item: second,
            content: ItemParams::Caption(project::CaptionParams {
                emphasis: Default::default(),
                word_highlight: false,
                preset: None,
                overrides: captions::CaptionOverrides::default(),
                cues: CueTrack::default(),
            }),
        })
        .unwrap();
    h.run_steps(3);
    assert!(
        h.query_all(By::new().predicate(|n| n.author_id() == Some("subtitles.style")))
            .next()
            .is_none()
    );
    let created = h
        .state_mut()
        .session
        .apply(Command::CreateScene {
            name: "Empty".into(),
        })
        .unwrap();
    let engine::CreatedId::Scene(scene) = created.created[0] else {
        panic!("scene")
    };
    h.state_mut()
        .session
        .apply(Command::SwitchScene { scene })
        .unwrap();
    h.run_steps(3);
    assert!(
        h.query_all(By::new().predicate(|n| n.author_id() == Some("subtitles.style")))
            .next()
            .is_none()
    );
    assert!(h.state_mut().activate("subtitles.style.box").is_err());
    let mut h = harness(editor("vi"), 1.0);
    h.run_steps(3);
    click(&mut h, "subtitles.style");
    *h.state_mut().session = Session::new(
        project::Project::empty(project::ProjectId(99), "Other".into()).unwrap(),
        10,
    )
    .unwrap();
    h.run_steps(3);
    assert!(
        h.query_all(By::new().predicate(|n| n.author_id() == Some("subtitles.style.box")))
            .next()
            .is_none()
    );
    assert!(!h.state().action_enabled("subtitles.style.box"));
    let gate: Gate = Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
    let mut h = harness(held_editor("vi", Arc::new(AtomicUsize::new(0)), &gate), 1.0);
    let _release = OpenOnDrop(gate);
    h.run_steps(3);
    click(&mut h, "subtitles.cue.cue-0");
    h.state_mut().activate("subtitles.style.box").unwrap();
    h.run_steps(3);
    let item = panel(h.state()).item.unwrap();
    h.state_mut()
        .session
        .apply(Command::SaveCaptionPreset {
            item,
            name: "Held job".into(),
        })
        .unwrap();
    h.run_steps(3);
    click(&mut h, "subtitles.style.save");
    h.get(By::new().predicate(|n| n.author_id() == Some("subtitles.bank.name")))
        .type_text("Stale job name");
    h.run_steps(2);
    let before = h.state().session.project().clone();
    let history = h.state().session.history_len();
    with_panel(h.state_mut(), |p, s, o| p.translate(s, o).unwrap());
    wait(&mut h, |e| panel(e).busy);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert!(
        h.query_all(By::new().predicate(|n| n.author_id() == Some("subtitles.bank.name")))
            .next()
            .is_none()
    );
    assert_eq!(h.state().session.project(), &before);
    assert_eq!(h.state().session.history_len(), history);
    for id in [
        "subtitles.style.save",
        "subtitles.style.manage",
        "subtitles.style.word_highlight",
    ] {
        assert!(h.state_mut().activate(id).is_err());
    }
    assert!(h.state_mut().activate("subtitles.style.box").is_err());
    h.state_mut().subtitles_mut().unwrap().cancel();
}

fn with_panel<T>(e: &mut Editor, f: impl FnOnce(&mut Subtitles, &mut Session, &Options) -> T) -> T {
    e.with_subtitles(f).unwrap()
}

#[test]
fn cue_actions_preserve_focused_caption_draft() {
    for (action, direct) in [
        ("subtitles.later", false),
        ("subtitles.cue.cue-0", false),
        ("subtitles.cue.cue-1", false),
        ("subtitles.later", true),
        ("subtitles.cue.cue-0", true),
        ("subtitles.cue.cue-1", true),
    ] {
        let mut h = harness(editor("vi"), 1.0);
        h.run_steps(3);
        // The chip shows player timecodes, not raw seconds, and the always-visible
        // keyboard paragraph is gone (the hint is the chip tooltip).
        let chip = h
            .get(By::new().predicate(|n| n.author_id() == Some("subtitles.cue.cue-0")))
            .accesskit_node()
            .label()
            .unwrap_or_default()
            .to_owned();
        let timecode = |s: &str| {
            s.len() == 11
                && s.matches(':').count() == 3
                && s.chars().all(|c| c.is_ascii_digit() || c == ':')
        };
        let (start, end) = chip.split_once(" – ").expect("range chip");
        assert!(timecode(start) && timecode(end), "{chip}");
        assert!(
            h.query_all(By::new().label_contains("Enter splits at the caret"))
                .next()
                .is_none()
        );
        click(&mut h, "subtitles.cue.cue-0");
        let field = "subtitles.text.cue-0.vi";
        h.get(By::new().predicate(|n| n.author_id() == Some(field)))
            .focus();
        h.run_steps(2);
        h.get(By::new().predicate(|n| n.author_id() == Some(field)))
            .type_text(" pending draft");
        h.run_steps(2);
        let expected = h
            .get(By::new().predicate(|n| n.author_id() == Some(field)))
            .value()
            .unwrap()
            .to_owned();
        let before = track(h.state()).cues[0].range;
        if direct {
            h.state_mut().activate(action).unwrap();
            h.run_steps(3);
        } else {
            click(&mut h, action);
        }
        let after = track(h.state());
        assert_eq!(
            after.cues[0].texts[&lang("vi")].text.as_str(),
            expected,
            "{action}"
        );
        if action == "subtitles.later" {
            assert!(after.cues[0].range.start() > before.start());
        } else {
            assert_eq!(after.cues[0].range, before);
        }
    }
}

#[test]
fn cue_edit_selection_seek_split_merge_nudge_and_history_use_real_controls() {
    for scale in [1.0, 2.0] {
        let mut h = harness(editor("vi"), scale);
        h.run_steps(4);
        let original = track(h.state());
        click(&mut h, "subtitles.cue.cue-0");
        assert_eq!(
            h.state().playback.position().unwrap(),
            Tick(TICKS_PER_SECOND)
        );
        let id = "subtitles.text.cue-0.vi";
        h.get(By::new().predicate(|n| n.author_id() == Some(id)))
            .scroll_to_me();
        h.run_steps(3);
        h.get(By::new().predicate(|n| n.author_id() == Some(id)))
            .focus();
        h.run_steps(2);
        h.get(By::new().predicate(|n| n.author_id() == Some(id)))
            .type_text(" mới");
        h.run_steps(2);
        h.state_mut()
            .session
            .apply(Command::RenameProject {
                name: "Concurrent project rename".into(),
            })
            .unwrap();
        h.run_steps(2);
        click(&mut h, "assets.tab.media");
        click(&mut h, "assets.tab.captions");
        assert!(track(h.state()).cues[0].texts[&lang("vi")].edited);
        assert!(
            track(h.state()).cues[0].texts[&lang("vi")]
                .text
                .as_str()
                .contains("mới")
        );
        h.state_mut().activate("edit.undo").unwrap();
        h.run_steps(2);
        assert_eq!(track(h.state()), original);
        let snapshot = h.state().session.project().clone();
        h.state_mut()
            .playback
            .seek(Tick(2 * TICKS_PER_SECOND), false, snapshot)
            .unwrap();
        click(&mut h, "subtitles.split");
        assert_eq!(track(h.state()).cues.len(), 4);
        click(&mut h, "subtitles.select_all");
        click(&mut h, "subtitles.merge");
        assert_eq!(track(h.state()).cues.len(), 1);
        h.state_mut().activate("edit.undo").unwrap();
        h.run_steps(2);
        assert_eq!(track(h.state()).cues.len(), 4);
        click(&mut h, "subtitles.cue.cue-2");
        let before = track(h.state());
        click(&mut h, "subtitles.earlier");
        let after = track(h.state());
        assert_eq!(
            before.cues.last().unwrap().range.start() - after.cues.last().unwrap().range.start(),
            h.state()
                .session
                .project()
                .data()
                .canvas
                .fps
                .frame_duration()
                .unwrap()
        );
        h.state_mut().activate("edit.undo").unwrap();
        h.run_steps(2);
        assert_eq!(track(h.state()), before);
        // Noncontiguous range is refused atomically; inverse nudge restores exact ticks.
        with_panel(h.state_mut(), |p, s, _| {
            p.select_cue(s, 0, false, false).unwrap();
            p.select_cue(s, 3, false, true).unwrap();
            assert!(p.merge(s).is_err());
        });
        assert_eq!(track(h.state()), before);
        with_panel(h.state_mut(), |p, s, _| {
            p.select_cue(s, 3, false, false).unwrap();
            p.nudge(s, true).unwrap();
            p.nudge(s, false).unwrap();
        });
        assert_eq!(track(h.state()), before);
        // A merge can remove the anchor row; Shift must extend from the surviving cue.
        with_panel(h.state_mut(), |p, s, _| {
            p.select_cue(s, 3, false, false).unwrap();
            p.select_cue(s, 2, true, false).unwrap();
            p.merge(s).unwrap();
            assert_eq!(p.inspect(s).selected.len(), 1);
            p.select_cue(s, 0, true, false).unwrap();
        });
        assert_eq!(panel(h.state()).selected.len(), 3);
    }
}

#[test]
fn import_is_atomic_bounded_and_export_uses_selected_bilingual_languages() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("Vietnamese 字.srt");
    std::fs::write(&input,"1\n00:00:00,000 --> 00:00:02,000\nXin chào\n\n2\n00:00:03,000 --> 00:00:05,000\nTạm biệt\n").unwrap();
    let mut h = harness(editor("en"), 1.0);
    h.run_steps(3);
    let before = h.state().session.project().clone();
    with_panel(h.state_mut(), |p, s, _| {
        p.import_file(s, input.clone(), "vi", Tick::ZERO).unwrap()
    });
    wait(&mut h, |e| !panel(e).busy);
    assert_eq!(panel(h.state()).completed, Some(2));
    let imported_item = panel(h.state()).item.unwrap();
    h.state_mut().activate("edit.undo").unwrap();
    h.run_steps(3);
    assert_eq!(h.state().session.project(), &before);
    h.state_mut().activate("edit.redo").unwrap();
    h.run_steps(3);
    with_panel(h.state_mut(), |p, s, _| {
        p.select_item(s, imported_item).unwrap()
    });
    assert_eq!(panel(h.state()).cue_count, 2);
    let item = panel(h.state()).item.unwrap();
    with_panel(h.state_mut(), |p, s, _| {
        p.select_item(s, item).unwrap();
    });
    // Source-language export needs no translator or credentials.
    h.state_mut().preferences.subtitles.target = "vi".into();
    let output = dir.path().join("captions.vtt");
    with_panel(h.state_mut(), |p, s, o| {
        p.export_subtitles(s, output.clone(), captions::Format::WebVtt, o, false)
            .unwrap()
    });
    wait(&mut h, |e| !panel(e).busy);
    let bytes = std::fs::read(&output).unwrap();
    assert!(bytes.starts_with(b"WEBVTT"));
    with_panel(h.state_mut(), |p, s, o| {
        p.export_subtitles(s, output.clone(), captions::Format::WebVtt, o, false)
            .unwrap()
    });
    wait(&mut h, |e| !panel(e).busy);
    assert_eq!(std::fs::read(&output).unwrap(), bytes);
    assert!(panel(h.state()).problem.is_some());
    // TXT is the caption-only download: one cue per line, no timings.
    let plain = dir.path().join("captions.txt");
    with_panel(h.state_mut(), |p, s, o| {
        p.export_subtitles(s, plain.clone(), captions::Format::Txt, o, false)
            .unwrap()
    });
    wait(&mut h, |e| !panel(e).busy);
    let text = std::fs::read_to_string(&plain).unwrap();
    assert!(!text.contains("-->"));
    assert_eq!(text.lines().count(), 2);
    assert!(text.contains("Xin chào"));
    let huge = dir.path().join("huge.srt");
    std::fs::File::create(&huge)
        .unwrap()
        .set_len(16 * 1024 * 1024 + 1)
        .unwrap();
    let before = h.state().session.project().clone();
    with_panel(h.state_mut(), |p, s, _| {
        p.import_file(s, huge, "vi", Tick::ZERO).unwrap()
    });
    wait(&mut h, |e| !panel(e).busy);
    assert_eq!(h.state().session.project(), &before);
    assert!(panel(h.state()).problem.is_some());
}

struct TranslateWorkflow {
    cancel_translation: bool,
    unknown_price: bool,
    calls: Arc<AtomicUsize>,
    /// Holds the provider call until the test opens the gate, so a job stays
    /// running while the busy block is on screen.
    gate: Option<Gate>,
}
/// A scratch models cache for the lent access; the fixtures never read or write it.
fn scratch_cache() -> Arc<std::sync::Mutex<engine::cache::ArtifactCache>> {
    static ROOT: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let dir = ROOT
        .get_or_init(|| tempfile::tempdir().unwrap())
        .path()
        .join(NEXT.fetch_add(1, Ordering::Relaxed).to_string());
    std::fs::create_dir_all(&dir).unwrap();
    let (cache, _) =
        engine::cache::ArtifactCache::open(&dir, ByteBudget::new(64 << 20).unwrap(), 8).unwrap();
    Arc::new(std::sync::Mutex::new(cache))
}
impl TranslateWorkflow {
    /// The lent-access seam: each run's access holds this fixture's translator behind the
    /// run's own consent, so `captions.translate` runs unchanged around it.
    fn opener(self) -> Opener {
        let fixture = Arc::new(self);
        Arc::new(move |choice, consent| {
            let Choice::Translate { provider, model } = choice else {
                panic!("the translation fixture was asked to transcribe");
            };
            let config = ai::ProviderConfig::new(
                provider.into(),
                model.into(),
                "fixture-v1".into(),
                ai::Location::Remote {
                    backend: "fixture".into(),
                    base_url: "https://provider.example".into(),
                    auth_ref: "fixture".into(),
                },
            )?;
            let translator = Arc::new(Translator {
                cancel_translation: fixture.cancel_translation,
                unknown_price: fixture.unknown_price,
                config,
                calls: fixture.calls.clone(),
                gate: fixture.gate.clone(),
            });
            let providers = engine::providers::Providers::translation_only_from_parts(
                engine::providers::choose_translation(provider, Some(model))?,
                translator,
                "context-window".into(),
            )?;
            Ok(engine::subtitles::CaptionAccess::new(
                Arc::new(providers),
                consent,
                scratch_cache(),
            ))
        })
    }
}
struct Translator {
    cancel_translation: bool,
    unknown_price: bool,
    config: ai::ProviderConfig,
    calls: Arc<AtomicUsize>,
    gate: Option<Gate>,
}
impl ai::Translator for Translator {
    fn config(&self) -> &ai::ProviderConfig {
        &self.config
    }
    fn capabilities(&self, _: &ai::Scope) -> ai::Capabilities {
        ai::Capabilities {
            model: self.config.model().clone(),
            mode: "context-window".into(),
            operation: ai::Operation::Translate,
            languages: ["vi", "en", "zh", "zh-Hans", "zh-Hant"].map(lang).to_vec(),
            target_languages: ["vi", "en", "zh", "zh-Hans", "zh-Hant"].map(lang).to_vec(),
            timing: BTreeSet::new(),
            diarization: ai::Support::Unsupported,
            live_audio_input: ai::Support::Unsupported,
            file_result_stream: ai::Support::Unsupported,
            max_upload_bytes: None,
            max_duration_micros: None,
            evidence: ai::Evidence::FixtureVerified,
            revision: "fixture".into(),
        }
    }
    fn estimate(&self, _: &ai::Scope) -> ai::CostEstimate {
        if self.unknown_price {
            ai::CostEstimate::Unknown
        } else {
            ai::CostEstimate::UpperBound(ai::Money::new("USD", 1_000).unwrap())
        }
    }
    fn translate(
        &self,
        window: &ai::TranslationWindow,
        _: &ai::Request<'_>,
        _: &dyn ai::ExecutionHooks,
    ) -> std::result::Result<ai::Translation, ai::Error> {
        if let Some(gate) = &self.gate {
            let (locked, opened) = &**gate;
            let mut open = locked.lock().unwrap();
            while !*open {
                open = opened.wait(open).unwrap();
            }
        }
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.cancel_translation {
            return Err(ai::Error::Cancelled);
        }
        Ok(ai::Translation {
            source_language: window.source_language.clone(),
            target_language: window.target_language.clone(),
            texts: window
                .segments
                .iter()
                .map(|s| {
                    (
                        s.id.clone(),
                        format!("{} [{}]", s.text.as_str(), window.target_language.as_str()).into(),
                    )
                })
                .collect(),
            number_warnings: Vec::new(),
        })
    }
}
fn translated_editor(source: &str, calls: Arc<AtomicUsize>) -> Editor {
    let mut e = editor(source);
    e.set_subtitles(Some(
        Subtitles::with_access(
            e.playback.budget().clone(),
            TranslateWorkflow {
                cancel_translation: false,
                calls,
                gate: None,
                unknown_price: false,
            }
            .opener(),
        )
        .unwrap(),
    ));
    e
}
type Gate = Arc<(std::sync::Mutex<bool>, std::sync::Condvar)>;
/// Opens the gate on unwind so a failed assertion cannot leave the provider
/// thread blocked while the editor joins it.
struct OpenOnDrop(Gate);
impl Drop for OpenOnDrop {
    fn drop(&mut self) {
        let (locked, opened) = &*self.0;
        *locked.lock().unwrap() = true;
        opened.notify_all();
    }
}
fn held_editor(source: &str, calls: Arc<AtomicUsize>, gate: &Gate) -> Editor {
    let mut e = editor(source);
    e.set_subtitles(Some(
        Subtitles::with_access(
            e.playback.budget().clone(),
            TranslateWorkflow {
                cancel_translation: false,
                unknown_price: false,
                calls,
                gate: Some(gate.clone()),
            }
            .opener(),
        )
        .unwrap(),
    ));
    e
}
#[test]
fn busy_block_does_not_shift_the_controls_below_it() {
    // The busy block draws a spinner, the job stage and Cancel above the cue
    // toolbar. It appears with an async job, so a press held across the update
    // must still land on the same widget.
    let gate: Gate = Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
    let calls = Arc::new(AtomicUsize::new(0));
    let mut h = harness(held_editor("vi", calls.clone(), &gate), 1.0);
    // Declared after the harness so it drops first and unblocks the provider
    // thread before the editor joins it, even when an assertion fails.
    let _release_on_exit = OpenOnDrop(gate.clone());
    h.run_steps(3);
    click(&mut h, "subtitles.cue.cue-0");
    let toolbar = node_id(&h, "subtitles.select_all");

    with_panel(h.state_mut(), |p, s, o| p.translate(s, o).unwrap());
    wait(&mut h, |e| panel(e).consent.is_some());
    click(&mut h, "subtitles.consent.approve");
    wait(&mut h, |e| panel(e).busy);
    assert_eq!(
        node_id(&h, "subtitles.select_all"),
        toolbar,
        "the cue toolbar must keep its widget id while the busy block is drawn"
    );
    assert_eq!(
        h.query_all(By::new().predicate(|n| n.author_id() == Some("subtitles.cancel")))
            .count(),
        1,
        "the busy block must offer Cancel while the job runs"
    );

    {
        let (locked, opened) = &*gate;
        *locked.lock().unwrap() = true;
        opened.notify_all();
    }
    wait(&mut h, |e| !panel(e).busy);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        node_id(&h, "subtitles.select_all"),
        toolbar,
        "the cue toolbar must keep its widget id after the busy block leaves"
    );
}
#[test]
fn translation_all_pairs_and_scripts_uses_fresh_consent_and_protects_manual_text() {
    for source in ["vi", "en", "zh"] {
        for target in ["vi", "en", "zh-Hans", "zh-Hant"] {
            if source == target {
                continue;
            }
            let calls = Arc::new(AtomicUsize::new(0));
            let mut e = translated_editor(source, calls.clone());
            e.preferences.subtitles.target = target.into();
            let mut h = harness(e, 1.0);
            h.run_steps(3);
            click(&mut h, "subtitles.cue.cue-0");
            with_panel(h.state_mut(), |p, s, o| p.translate(s, o).unwrap());
            wait(&mut h, |e| panel(e).consent.is_some());
            let disclosure = panel(h.state()).consent.unwrap();
            assert_eq!(disclosure.provider, "openai");
            assert!(disclosure.languages.contains(target));
            assert!(disclosure.upload_bytes > 0);
            assert_eq!(calls.load(Ordering::SeqCst), 0);
            click(&mut h, "subtitles.consent.approve");
            wait(&mut h, |e| !panel(e).busy);
            assert_eq!(calls.load(Ordering::SeqCst), 1);
            let result = track(h.state());
            assert!(result.cues[0].texts.contains_key(&lang(target)));
            assert!(!result.cues[1].texts.contains_key(&lang(target)));
            with_panel(h.state_mut(), |p, s, _| {
                p.edit_text(s, &"cue-0".into(), lang(target), "Human edit")
                    .unwrap()
            });
            // An all-protected selection is refused when the command is submitted:
            // no job, no upload and no provider invocation.
            let refused = with_panel(h.state_mut(), |p, s, o| p.translate(s, o));
            assert!(
                matches!(
                    &refused,
                    Err(vibecut::Error::Engine(engine::control::Error::Subtitles(
                        engine::subtitles::Error::Edit(engine::EditError::Invalid(
                            "no unprotected translation selection"
                        ))
                    )))
                ),
                "{refused:?}"
            );
            assert!(!panel(h.state()).busy);
            assert_eq!(calls.load(Ordering::SeqCst), 1);
            assert_eq!(
                track(h.state()).cues[0].texts[&lang(target)].text.as_str(),
                "Human edit"
            );
            let dir = tempfile::tempdir().unwrap();
            let output = dir.path().join("bilingual.srt");
            h.state_mut().preferences.subtitles.bilingual = true;
            // Translate the remaining cues explicitly before bilingual export.
            click(&mut h, "subtitles.select_all");
            with_panel(h.state_mut(), |p, s, o| p.translate(s, o).unwrap());
            wait(&mut h, |e| panel(e).consent.is_some());
            click(&mut h, "subtitles.consent.approve");
            wait(&mut h, |e| !panel(e).busy);
            with_panel(h.state_mut(), |p, s, o| {
                p.export_subtitles(s, output.clone(), captions::Format::Srt, o, false)
                    .unwrap()
            });
            wait(&mut h, |e| !panel(e).busy);
            let text = std::fs::read_to_string(output).unwrap();
            assert!(text.contains("Human edit"));
            assert!(text.contains(result.cues[0].texts[&lang(source)].text.as_str()));
        }
    }
}
#[test]
fn consent_denial_provider_change_cancel_and_stale_results_never_apply() {
    for action in ["deny", "change", "cancel", "stale", "provider-failure"] {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut e = translated_editor("vi", calls.clone());
        if action == "provider-failure" {
            e.set_subtitles(Some(
                Subtitles::with_access(
                    e.playback.budget().clone(),
                    TranslateWorkflow {
                        cancel_translation: true,
                        calls: calls.clone(),
                        gate: None,
                        unknown_price: false,
                    }
                    .opener(),
                )
                .unwrap(),
            ));
        }
        let mut h = harness(e, 1.0);
        h.run_steps(3);
        click(&mut h, "subtitles.cue.cue-0");
        let before = track(h.state());
        with_panel(h.state_mut(), |p, s, o| p.translate(s, o).unwrap());
        wait(&mut h, |e| panel(e).consent.is_some());
        // Tab changes do not hide the pending modal.
        h.state_mut().preferences.assets_tab = AssetsTab::Media;
        h.run_steps(3);
        assert!(
            h.query_all(
                By::new().predicate(|n| n.author_id() == Some("subtitles.consent.approve"))
            )
            .count()
                == 1
        );
        match action {
            "deny" => click(&mut h, "subtitles.consent.deny"),
            "change" => h.state_mut().preferences.subtitles.text_provider = "anthropic".into(),
            "cancel" => h.state_mut().subtitles_mut().unwrap().cancel(),
            "provider-failure" => click(&mut h, "subtitles.consent.approve"),
            _ => {
                h.state_mut()
                    .session
                    .apply(Command::RenameProject {
                        name: "Changed".into(),
                    })
                    .unwrap();
                click(&mut h, "subtitles.consent.approve");
            }
        }
        wait(&mut h, |e| !panel(e).busy);
        assert_eq!(track(h.state()), before);
        assert!(panel(h.state()).problem.is_some());
        assert_eq!(
            calls.load(Ordering::SeqCst),
            usize::from(matches!(action, "stale" | "provider-failure"))
        );
        if action == "provider-failure" {
            let error = panel(h.state()).error.unwrap();
            assert!(matches!(
                (error.as_ref() as &(dyn std::error::Error + 'static))
                    .downcast_ref::<engine::subtitles::Error>(),
                Some(engine::subtitles::Error::Transcript(
                    engine::transcript::Error::Ai(ai::Error::Cancelled)
                ))
            ));
            assert_eq!(error.to_string(), "AI execution failed");
            assert_eq!(error.code(), engine::control::ErrorCode::Ai);
            assert!(!error.job_policy().terminal_cancelled);
            assert_eq!(panel(h.state()).job_state, None);
        }
    }
}
#[test]
fn capabilities_locale_and_choices_persist_without_loading_models() {
    let mut h = harness(editor("vi"), 2.0);
    h.run_steps(3);
    assert_eq!(Options::default().source, "auto");
    click(&mut h, "subtitles.source");
    assert_eq!(
        h.query_all(By::new().predicate(|n| n.author_id() == Some("subtitles.source.auto")))
            .count(),
        1,
        "the spoken-language list always offers Auto"
    );
    click(&mut h, "subtitles.source.auto");
    assert_eq!(h.state().preferences.subtitles.source, "auto");
    click(&mut h, "subtitles.transcriber");
    click(
        &mut h,
        "subtitles.transcriber.gipformer.gipformer-68m-rnnt-int8",
    );
    assert_eq!(h.state().preferences.subtitles.transcription, "gipformer");
    click(&mut h, "subtitles.source");
    click(&mut h, "subtitles.source.en");
    assert!(
        !h.state()
            .inspect()
            .unwrap()
            .widgets
            .iter()
            .find(|w| w.id == "subtitles.generate")
            .unwrap()
            .enabled
    );
    click(&mut h, "subtitles.transcriber");
    assert!(
        h.query_all(
            By::new().predicate(|n| n.author_id()
                == Some("subtitles.transcriber.gipformer.gipformer-68m-rnnt-int8"))
        )
        .next()
        .is_none()
    );
    click(
        &mut h,
        "subtitles.transcriber.whisper.cpp.whisper-large-v3-turbo",
    );
    let project = h.state().session.project().data().id;
    assert_eq!(h.state().preferences.subtitles.source_for(project), "en");
    assert_eq!(
        h.state()
            .preferences
            .subtitles
            .source_for(project::ProjectId(99)),
        "auto"
    );
    let dir = tempfile::tempdir().unwrap();
    let store = PreferenceStore::new(dir.path().join("prefs.json"));
    store.save(&h.state().preferences).unwrap();
    assert_eq!(
        store.load_with_warning().0.subtitles,
        h.state().preferences.subtitles
    );
    assert_eq!(
        store.load_with_warning().0.subtitles.source_for(project),
        "en",
        "the spoken-language choice persists per project"
    );
    assert!(has(&h, "subtitles.generate") && has(&h, "subtitles.identify_fillers"));
    assert!(!has(&h, "subtitles.translate") && !has(&h, "subtitles.import"));
    click(&mut h, "captions.section.translate");
    assert!(has(&h, "subtitles.translate") && has(&h, "subtitles.force"));
    assert!(!has(&h, "subtitles.generate"));
    click(&mut h, "captions.section.add");
    assert!(has(&h, "subtitles.import") && !has(&h, "subtitles.translate"));
    click(&mut h, "captions.section.auto");
    assert!(has(&h, "subtitles.generate") && has(&h, "subtitles.delete_current"));
    h.state_mut().preferences.locale = Locale::Vi;
    h.run_steps(3);
    assert_eq!(h.state().preferences.subtitles.transcription, "whisper.cpp");
    assert!(h.state().models().is_none());
}

#[derive(Clone, Copy)]
enum GenerateFailure {
    NoSpeech,
    LowConfidence,
    Silent,
}
/// Recognition is the fixture's: one sentence per speech window, in the requested language.
struct Speech {
    config: ai::ProviderConfig,
}
impl ai::Transcriber for Speech {
    fn config(&self) -> &ai::ProviderConfig {
        &self.config
    }
    fn capabilities(&self, _: &ai::Scope) -> ai::Capabilities {
        ai::Capabilities {
            model: self.config.model().clone(),
            mode: "transcribe".into(),
            operation: ai::Operation::Transcribe,
            languages: ["vi", "en", "zh"].map(lang).to_vec(),
            target_languages: Vec::new(),
            timing: BTreeSet::from([ai::TimingUnit::Segment]),
            diarization: ai::Support::Unsupported,
            live_audio_input: ai::Support::Unsupported,
            file_result_stream: ai::Support::Unsupported,
            max_upload_bytes: None,
            max_duration_micros: None,
            evidence: ai::Evidence::FixtureVerified,
            revision: "fixture".into(),
        }
    }
    fn estimate(&self, _: &ai::Scope) -> ai::CostEstimate {
        ai::CostEstimate::UpperBound(ai::Money::new("USD", 1_000).unwrap())
    }
    fn transcribe(
        &self,
        audio: ai::AudioWindow<'_>,
        request: &ai::Request<'_>,
        _: &dyn ai::ExecutionHooks,
    ) -> std::result::Result<Vec<ai::RecognitionSegment>, ai::Error> {
        Ok(vec![ai::RecognitionSegment {
            id: "fixture:0".into(),
            text: format!(
                "one complete sentence in {}.",
                request.scope.source_language.as_str()
            )
            .into(),
            range: Some(ai::SampleRange::new(audio.range.start(), audio.range.end()).unwrap()),
            timing: Vec::new(),
            raw_tokens: Vec::new(),
        }])
    }
}
/// The lent-access seam of generation. `missing` and the failures refuse the run the way the
/// engine reports them (a typed error from composing the access); every other run lends a
/// fixture transcriber, so `captions.generate` extracts real speech windows from the clip's
/// tone file, asks consent for remote providers and recognises through `Speech`.
/// A failure refuses every run, including the auto-detect retry.
fn generate_opener(missing: bool, failure: Option<GenerateFailure>) -> Opener {
    Arc::new(move |choice, consent| {
        let Choice::Transcribe { provider, model } = choice else {
            panic!("the generation fixture was asked to translate");
        };
        if missing {
            return Err(engine::providers::Error::ModelMissing("whisper-large-v3-turbo").into());
        }
        match failure {
            Some(GenerateFailure::NoSpeech) => {
                return Err(engine::transcript::Error::NoSpeech.into());
            }
            Some(GenerateFailure::LowConfidence) => {
                return Err(engine::transcript::Error::LowConfidence.into());
            }
            Some(GenerateFailure::Silent) => {
                return Err(engine::transcript::Error::NoSpeech.into());
            }
            _ => {}
        }
        let local = engine::providers::provider_options_for(
            engine::providers::CatalogOperation::Transcribe,
            None,
        )
        .iter()
        .find(|option| option.id == provider)
        .is_none_or(|option| option.local);
        let config = ai::ProviderConfig::new(
            provider.into(),
            model.into(),
            "fixture-v1".into(),
            if local {
                ai::Location::Local {
                    backend: "fixture".into(),
                    model_ref: "fixture".into(),
                }
            } else {
                ai::Location::Remote {
                    backend: "fixture".into(),
                    base_url: "https://provider.example".into(),
                    auth_ref: "fixture".into(),
                }
            },
        )?;
        let providers = engine::providers::Providers::from_parts(
            engine::providers::choose_transcription(provider, Some(model))?,
            "transcribe".into(),
            Arc::new(Speech { config }),
            None,
            None,
            None,
        )?;
        Ok(engine::subtitles::CaptionAccess::new(
            Arc::new(providers),
            consent,
            scratch_cache(),
        ))
    })
}
/// Ten seconds of a 440 Hz tone as a real WAV file, so speech windows can be extracted.
fn tone_wav() -> std::path::PathBuf {
    static ROOT: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    let path = ROOT
        .get_or_init(|| tempfile::tempdir().unwrap())
        .path()
        .join("subtitle-synthetic.wav");
    if !path.exists() {
        const RATE: u32 = 16_000;
        let samples: Vec<u8> = (0..RATE * 10)
            .flat_map(|n| {
                let phase = f64::from(n) * 440.0 * std::f64::consts::TAU / f64::from(RATE);
                ((phase.sin() * 8_000.0) as i16).to_le_bytes()
            })
            .collect();
        let mut wav = Vec::with_capacity(44 + samples.len());
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + samples.len() as u32).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&RATE.to_le_bytes());
        wav.extend_from_slice(&(RATE * 2).to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&(samples.len() as u32).to_le_bytes());
        wav.extend_from_slice(&samples);
        std::fs::write(&path, wav).unwrap();
    }
    path
}
fn source_clip(e: &mut Editor) -> project::ItemId {
    let asset = media::SourceAsset::import(media::AssetId(2), tone_wav()).unwrap();
    let duration = asset.probe().duration.unwrap();
    e.session.apply(Command::AddAsset { asset }).unwrap();
    e.session
        .apply(Command::InsertItem {
            placement: engine::Placement::Auto,
            item: engine::NewItem {
                retime: Default::default(),
                animations: Default::default(),
                source_asset: Some(media::AssetId(2)),
                source_range: time::SourceRange::new(Tick::ZERO, duration).unwrap(),
                start: Tick::ZERO,
                content: ItemParams::Audio(project::AudioParams::default()),
            },
        })
        .unwrap()
        .created
        .into_iter()
        .find_map(|id| {
            if let engine::CreatedId::Item(id) = id {
                Some(id)
            } else {
                None
            }
        })
        .unwrap()
}

#[test]
fn generate_explicit_choices_insert_once_and_missing_models_offer_setup_without_download() {
    for option in engine::providers::provider_options_for(
        engine::providers::CatalogOperation::Transcribe,
        Some(&lang("vi")),
    ) {
        let mut e = editor("vi");
        let source = source_clip(&mut e);
        e.select([source].into()).unwrap();
        e.preferences.subtitles.transcription = option.id.into();
        e.preferences.subtitles.transcription_model = option.models[0].clone();
        e.set_subtitles(Some(
            Subtitles::with_access(e.playback.budget().clone(), generate_opener(false, None))
                .unwrap(),
        ));
        let mut h = harness(e, 1.0);
        h.run_steps(3);
        let before = h.state().session.project().clone();
        click(&mut h, "subtitles.generate");
        if !option.local {
            wait(&mut h, |e| panel(e).consent.is_some());
            assert_eq!(panel(h.state()).consent.unwrap().provider, option.id);
            click(&mut h, "subtitles.consent.approve");
        }
        wait(&mut h, |e| !panel(e).busy);
        // The fixture already carries a caption; generation adds a new item after it.
        let count = h
            .state()
            .session
            .project()
            .data()
            .scene()
            .timeline
            .iter()
            .flat_map(|t| &t.items)
            .filter_map(|i| match &i.content {
                ItemParams::Caption(p) => Some((i.id, p.cues.cues.len())),
                _ => None,
            })
            .max_by_key(|(id, _)| *id)
            .unwrap()
            .1;
        assert!(count > 0);
        assert_eq!(panel(h.state()).completed, Some(count));
        assert!(
            h.state()
                .inspect()
                .unwrap()
                .import_toast
                .unwrap()
                .contains(&count.to_string())
        );
        h.state_mut().activate("edit.undo").unwrap();
        h.run_steps(3);
        assert_eq!(h.state().session.project(), &before);
    }
    let mut e = editor("vi");
    let source = source_clip(&mut e);
    e.select([source].into()).unwrap();
    e.set_subtitles(Some(
        Subtitles::with_access(e.playback.budget().clone(), generate_opener(true, None)).unwrap(),
    ));
    let mut h = harness(e, 1.0);
    h.run_steps(3);
    click(&mut h, "subtitles.generate");
    wait(&mut h, |e| !panel(e).busy);
    assert_eq!(
        panel(h.state()).problem,
        Some(vibecut::i18n::Text::ShortsModelMissing)
    );
    assert!(h.state().models().is_none());
    assert!(
        h.query_all(By::new().predicate(|n| n.author_id() == Some("subtitles.models")))
            .count()
            == 1
    );
}

#[test]
fn caption_language_auto_retry_covers_no_speech_low_confidence_and_silent_clips() {
    for (failure, source) in [
        (GenerateFailure::NoSpeech, "vi"),
        (GenerateFailure::LowConfidence, "en"),
        (GenerateFailure::Silent, "vi"),
        (GenerateFailure::Silent, "auto"),
    ] {
        let mut e = editor("vi");
        let clip = source_clip(&mut e);
        e.select([clip].into()).unwrap();
        e.preferences.subtitles.source = source.into();
        e.set_subtitles(Some(
            Subtitles::with_access(
                e.playback.budget().clone(),
                generate_opener(false, Some(failure)),
            )
            .unwrap(),
        ));
        let mut h = harness(e, 1.0);
        h.run_steps(3);
        click(&mut h, "subtitles.generate");
        wait(&mut h, |e| !panel(e).busy);
        let expected = match failure {
            GenerateFailure::LowConfidence => vibecut::i18n::Text::SubLowConfidence,
            _ => vibecut::i18n::Text::SubNoSpeech,
        };
        assert_eq!(panel(h.state()).problem, Some(expected), "{source}");
        let retry = h
            .query_all(By::new().predicate(|n| n.author_id() == Some("subtitles.retry_auto")))
            .count();
        assert_eq!(retry, usize::from(source != "auto"), "{source}");
        if source != "auto" {
            click(&mut h, "subtitles.retry_auto");
            wait(&mut h, |e| !panel(e).busy);
            assert_eq!(h.state().preferences.subtitles.source, "auto");
            // The engine reports the same typed outcome for the auto-detect run.
            assert_eq!(panel(h.state()).problem, Some(expected), "{source}");
        }
    }
}

#[test]
fn cue_caret_split_line_break_and_neighbour_merge_follow_word_boundaries() {
    for (text, caret, left, right) in [
        ("Chào bạn", 4, "Chào", "bạn"),
        ("Hello friend", 6, "Hello", "friend"),
    ] {
        let mut h = harness(editor("vi"), 1.0);
        h.run_steps(3);
        click(&mut h, "subtitles.cue.cue-0");
        with_panel(h.state_mut(), |p, s, _| {
            p.edit_text(s, &"cue-0".into(), lang("vi"), text).unwrap()
        });
        let before = track(h.state());
        let at = with_panel(h.state_mut(), |p, s, _| {
            p.split_text(s, &"cue-0".into(), lang("vi"), text, caret)
                .unwrap()
        });
        let after = track(h.state());
        assert_eq!(after.cues.len(), 4, "{text}");
        assert_eq!(after.cues[0].texts[&lang("vi")].text.as_str(), left);
        assert_eq!(after.cues[1].texts[&lang("vi")].text.as_str(), right);
        assert_eq!(after.cues[0].range.end(), at);
        assert_eq!(after.cues[1].range.start(), at);
        assert!(after.cues[0].range.duration() > Tick::ZERO);
        assert!(after.cues[1].range.duration() > Tick::ZERO);
        h.state_mut().activate("edit.undo").unwrap();
        h.run_steps(2);
        assert_eq!(track(h.state()), before);
    }
    // A word with no internal boundary refuses instead of inventing one.
    let mut h = harness(editor("vi"), 1.0);
    h.run_steps(3);
    click(&mut h, "subtitles.cue.cue-0");
    let before = track(h.state());
    with_panel(h.state_mut(), |p, s, _| {
        assert!(
            p.split_text(s, &"cue-0".into(), lang("vi"), "Hello", 3)
                .is_err()
        );
    });
    assert_eq!(track(h.state()), before);
    // A one-tick cue cannot become two nonempty cues.
    let mut cues = before.clone();
    cues.cues[0].range = time::SourceRange::new(Tick(10), Tick(11)).unwrap();
    let item = panel(h.state()).item.unwrap();
    h.state_mut()
        .session
        .apply(Command::SetItemParam {
            item,
            content: ItemParams::Caption(project::CaptionParams {
                emphasis: Default::default(),
                word_highlight: false,
                preset: None,
                overrides: captions::CaptionOverrides::default(),
                cues,
            }),
        })
        .unwrap();
    h.run_steps(2);
    with_panel(h.state_mut(), |p, s, _| {
        assert!(
            p.split_text(s, &"cue-0".into(), lang("vi"), "Chào bạn", 4)
                .is_err()
        );
    });
    // Native word timing decides the cut time, not a proportional guess.
    let mut h = harness(editor("vi"), 1.0);
    h.run_steps(3);
    click(&mut h, "subtitles.cue.cue-0");
    let mut cues = track(h.state());
    let first =
        time::SourceRange::new(Tick(TICKS_PER_SECOND), Tick(TICKS_PER_SECOND * 3 / 2)).unwrap();
    let second =
        time::SourceRange::new(Tick(TICKS_PER_SECOND * 2), Tick(TICKS_PER_SECOND * 5 / 2)).unwrap();
    let cue = &mut cues.cues[0];
    cue.texts.get_mut(&lang("vi")).unwrap().text = "one two".into();
    cue.word_timing = captions::CueWordTiming::Words {
        language: lang("vi"),
        text: "one two".into(),
        records: [(0..3, first), (4..7, second)]
            .into_iter()
            .map(|(bytes, range)| captions::VerifiedCueWord {
                bytes,
                source_range: range,
                projection: captions::WordProjection::Visible(range),
                provenance: captions::TimingProvenance::Native,
            })
            .collect(),
    };
    let item = panel(h.state()).item.unwrap();
    h.state_mut()
        .session
        .apply(Command::SetItemParam {
            item,
            content: ItemParams::Caption(project::CaptionParams {
                emphasis: Default::default(),
                word_highlight: false,
                preset: None,
                overrides: captions::CaptionOverrides::default(),
                cues,
            }),
        })
        .unwrap();
    h.run_steps(2);
    let at = with_panel(h.state_mut(), |p, s, _| {
        p.split_text(s, &"cue-0".into(), lang("vi"), "one two", 3)
            .unwrap()
    });
    assert_eq!(at, first.end());
    let after = track(h.state());
    assert_eq!(after.cues[0].range.end(), first.end());
    assert_eq!(after.cues[1].range.start(), first.end());
    // Enter in the real field splits at the caret; Shift+Enter only breaks lines.
    let mut h = harness(editor("vi"), 1.0);
    h.run_steps(3);
    click(&mut h, "subtitles.cue.cue-0");
    let field = "subtitles.text.cue-0.vi";
    h.get(By::new().predicate(|n| n.author_id() == Some(field)))
        .focus();
    h.run_steps(2);
    h.key_press(egui::Key::Home);
    h.run_steps(1);
    for _ in 0..4 {
        h.key_press(egui::Key::ArrowRight);
        h.run_steps(1);
    }
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert_eq!(track(h.state()).cues.len(), 4);
    assert_eq!(
        track(h.state()).cues[0].texts[&lang("vi")].text.as_str(),
        "Chào"
    );
    h.state_mut().activate("edit.undo").unwrap();
    h.run_steps(2);
    assert_eq!(track(h.state()).cues.len(), 3);
    let mut h = harness(editor("vi"), 1.0);
    h.run_steps(3);
    click(&mut h, "subtitles.cue.cue-0");
    h.get(By::new().predicate(|n| n.author_id() == Some(field)))
        .focus();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::SHIFT, egui::Key::Enter);
    h.run_steps(2);
    click(&mut h, "subtitles.cue.cue-1");
    assert_eq!(track(h.state()).cues.len(), 3);
    assert!(
        track(h.state()).cues[0].texts[&lang("vi")]
            .text
            .as_str()
            .contains('\n')
    );
    // Neighbour merges join text and spans; first/last boundaries refuse.
    for down in [false, true] {
        let mut h = harness(editor("vi"), 1.0);
        h.run_steps(3);
        click(&mut h, "subtitles.cue.cue-1");
        let before = track(h.state());
        with_panel(h.state_mut(), |p, s, _| {
            if down { p.merge_down(s) } else { p.merge_up(s) }.unwrap()
        });
        let after = track(h.state());
        assert_eq!(after.cues.len(), 2);
        if down {
            assert_eq!(after.cues[1].range.end(), before.cues[2].range.end());
            assert_eq!(
                after.cues[1].texts[&lang("vi")].text.as_str(),
                format!(
                    "{}\n{}",
                    before.cues[1].texts[&lang("vi")].text.as_str(),
                    before.cues[2].texts[&lang("vi")].text.as_str()
                )
            );
        } else {
            assert_eq!(after.cues[0].range.start(), before.cues[0].range.start());
            assert_eq!(
                after.cues[0].texts[&lang("vi")].text.as_str(),
                format!(
                    "{}\n{}",
                    before.cues[0].texts[&lang("vi")].text.as_str(),
                    before.cues[1].texts[&lang("vi")].text.as_str()
                )
            );
        }
        h.state_mut().activate("edit.undo").unwrap();
        h.run_steps(2);
        assert_eq!(track(h.state()), before);
    }
    let mut h = harness(editor("vi"), 1.0);
    h.run_steps(3);
    click(&mut h, "subtitles.cue.cue-0");
    with_panel(h.state_mut(), |p, s, _| assert!(p.merge_up(s).is_err()));
    click(&mut h, "subtitles.cue.cue-2");
    with_panel(h.state_mut(), |p, s, _| assert!(p.merge_down(s).is_err()));
    assert_eq!(track(h.state()).cues.len(), 3);
    // Backspace at the start and Delete at the end are the same merges.
    let mut h = harness(editor("vi"), 1.0);
    h.run_steps(3);
    click(&mut h, "subtitles.cue.cue-1");
    let field = "subtitles.text.cue-1.vi";
    h.get(By::new().predicate(|n| n.author_id() == Some(field)))
        .focus();
    h.run_steps(2);
    h.key_press(egui::Key::Home);
    h.run_steps(1);
    h.key_press(egui::Key::Backspace);
    h.run_steps(3);
    assert_eq!(track(h.state()).cues.len(), 2);
    let mut h = harness(editor("vi"), 1.0);
    h.run_steps(3);
    click(&mut h, "subtitles.cue.cue-1");
    h.get(By::new().predicate(|n| n.author_id() == Some("subtitles.text.cue-1.vi")))
        .focus();
    h.run_steps(2);
    h.key_press(egui::Key::End);
    h.run_steps(1);
    h.key_press(egui::Key::Delete);
    h.run_steps(3);
    assert_eq!(track(h.state()).cues.len(), 2);
}

#[test]
fn concurrent_caption_edits_retain_the_draft_until_explicit_reload() {
    let mut h = harness(editor("vi"), 1.0);
    h.run_steps(3);
    click(&mut h, "subtitles.cue.cue-0");
    let field = "subtitles.text.cue-0.vi";
    h.get(By::new().predicate(|n| n.author_id() == Some(field)))
        .focus();
    h.run_steps(2);
    h.get(By::new().predicate(|n| n.author_id() == Some(field)))
        .type_text(" pending draft");
    h.run_steps(2);
    let mut updated = track(h.state());
    updated.cues[0].texts.get_mut(&lang("vi")).unwrap().text = "External caption change".into();
    let item = panel(h.state()).item.unwrap();
    h.state_mut()
        .session
        .apply(Command::SetItemParam {
            item,
            content: ItemParams::Caption(project::CaptionParams {
                emphasis: Default::default(),
                word_highlight: false,
                preset: None,
                overrides: captions::CaptionOverrides::default(),
                cues: updated,
            }),
        })
        .unwrap();
    h.run_steps(3);
    assert_eq!(
        panel(h.state()).problem,
        Some(vibecut::i18n::Text::SubDraftConflict)
    );
    assert!(
        h.get(By::new().predicate(|n| n.author_id() == Some(field)))
            .value()
            .unwrap()
            .contains("pending draft")
    );
    assert_eq!(
        track(h.state()).cues[0].texts[&lang("vi")].text.as_str(),
        "External caption change"
    );
    click(&mut h, "subtitles.dismiss");
    assert_eq!(
        h.get(By::new().predicate(|n| n.author_id() == Some(field)))
            .value()
            .unwrap(),
        "External caption change"
    );
    assert_eq!(
        track(h.state()).cues[0].texts[&lang("vi")].text.as_str(),
        "External caption change"
    );
}

#[test]
fn translation_unknown_price_requires_the_actual_model_checkbox_in_both_locales() {
    for (locale, scale) in [
        (Locale::En, 1.0),
        (Locale::Vi, 1.0),
        (Locale::En, 2.0),
        (Locale::Vi, 2.0),
    ] {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut e = editor("vi");
        e.preferences.locale = locale;
        e.preferences
            .ai_models
            .select("openai", ai::Operation::Translate, "fixture-unknown-price")
            .unwrap();
        e.preferences.resolve_models();
        e.set_subtitles(Some(
            Subtitles::with_access(
                e.playback.budget().clone(),
                TranslateWorkflow {
                    cancel_translation: false,
                    calls: calls.clone(),
                    gate: None,
                    unknown_price: true,
                }
                .opener(),
            )
            .unwrap(),
        ));
        let mut h = harness(e, scale);
        h.run_steps(3);
        click(&mut h, "subtitles.cue.cue-0");
        with_panel(h.state_mut(), |p, s, o| p.translate(s, o).unwrap());
        wait(&mut h, |e| panel(e).consent.is_some());
        let disclosure = panel(h.state()).consent.unwrap();
        assert_eq!(disclosure.model, "fixture-unknown-price");
        assert!(
            h.query_by_label(&format!(
                "{} · {} · {}",
                disclosure.provider, disclosure.model, disclosure.languages
            ))
            .is_some()
        );
        assert!(
            h.query_by_label(vibecut::i18n::Text::ShortsUnknownCost.text(locale))
                .is_some()
        );
        assert_eq!(disclosure.cost, Cost::Unknown);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(
            h.get(By::new().predicate(|n| n.author_id() == Some("subtitles.consent.approve")))
                .accesskit_node()
                .is_disabled()
        );
        let output = std::env::var_os("CARGO_TARGET_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target")
            })
            .join("ui-audit");
        std::fs::create_dir_all(&output).unwrap();
        h.render()
            .unwrap()
            .save(output.join(format!("captions-unknown-price-{locale:?}@{scale}x.png")))
            .unwrap();
        click(&mut h, "subtitles.consent.unknown_price");
        h.run_steps(2);
        assert!(
            !h.get(By::new().predicate(|n| n.author_id() == Some("subtitles.consent.approve")))
                .accesskit_node()
                .is_disabled()
        );
        click(&mut h, "subtitles.consent.approve");
        wait(&mut h, |e| !panel(e).busy);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(track(h.state()).cues[0].texts.contains_key(&lang("en")));
    }
}

#[test]
fn word_highlight_toggle_uses_selected_item_and_warning_clears_on_undo_disable_and_locale() {
    for (locale, scale) in [
        (Locale::En, 1.0),
        (Locale::En, 2.0),
        (Locale::Vi, 1.0),
        (Locale::Vi, 2.0),
    ] {
        let mut e = editor("vi");
        e.preferences.locale = locale;
        let mut h = harness(e, scale);
        h.run_steps(3);
        click(&mut h, "subtitles.style.word_highlight");
        let item = panel(h.state()).item.unwrap();
        let value = h
            .state()
            .session
            .project()
            .data()
            .scene()
            .timeline
            .iter()
            .flat_map(|track| &track.items)
            .find(|value| value.id == item)
            .unwrap();
        assert!(matches!(&value.content,ItemParams::Caption(params) if params.word_highlight));
        assert!(
            h.query_all(By::new().predicate(|n| {
                n.author_id()
                    .is_some_and(|id| id.starts_with("subtitles.word_highlight.warning."))
            }))
            .next()
            .is_some()
        );
        let output = std::env::var_os("CARGO_TARGET_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| {
                std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target")
            })
            .join("ui-audit");
        std::fs::create_dir_all(&output).unwrap();
        h.render()
            .unwrap()
            .save(output.join(format!("captions-word-missing-{locale:?}@{scale}x.png")))
            .unwrap();
        h.state_mut().session.undo().unwrap();
        h.run_steps(3);
        assert!(
            h.query_all(By::new().predicate(|n| {
                n.author_id()
                    .is_some_and(|id| id.starts_with("subtitles.word_highlight.warning."))
            }))
            .next()
            .is_none()
        );
        h.render()
            .unwrap()
            .save(output.join(format!("captions-word-cleared-{locale:?}@{scale}x.png")))
            .unwrap();
        h.state_mut().session.redo().unwrap();
        h.run_steps(3);
        click(&mut h, "subtitles.style.word_highlight");
        assert!(
            h.query_all(By::new().predicate(|n| {
                n.author_id()
                    .is_some_and(|id| id.starts_with("subtitles.word_highlight.warning."))
            }))
            .next()
            .is_none()
        );
        click(&mut h, "subtitles.cue.cue-0");
        let field = "subtitles.text.cue-0.vi";
        h.get(By::new().predicate(|n| n.author_id() == Some(field)))
            .focus();
        h.run_steps(2);
        let before = h.state().session.project().clone();
        assert!(
            h.state_mut()
                .activate("subtitles.style.word_highlight")
                .is_err()
        );
        assert!(
            Arc::ptr_eq(&before, h.state().session.project()),
            "refuse before inline preflush"
        );
    }
}

#[test]
fn verified_word_warning_tracks_manual_source_changes_and_actual_undo() {
    let mut e = fixture::editor_with_cues("vi", fixture::verified_cues());
    e.preferences.subtitles.bilingual = true;
    let mut h = harness(e, 1.0);
    h.run_steps(3);
    click(&mut h, "subtitles.style.word_highlight");
    let warning = || {
        By::new().predicate(|n| {
            n.author_id()
                .is_some_and(|id| id.starts_with("subtitles.word_highlight.warning."))
        })
    };
    assert!(h.query_all(warning()).next().is_none());
    with_panel(h.state_mut(), |panel, session, _| {
        panel
            .edit_text(session, &"cue-0".into(), lang("vi"), "Xin chào")
            .unwrap()
    });
    h.run_steps(3);
    assert!(
        h.query_all(warning()).next().is_some(),
        "same-count edited source is not alignment proof"
    );
    h.state_mut().session.undo().unwrap();
    h.run_steps(3);
    assert!(h.query_all(warning()).next().is_none());
    with_panel(h.state_mut(), |panel, session, _| {
        panel.select_cue(session, 0, false, false).unwrap();
        panel.select_cue(session, 1, true, false).unwrap();
        panel.merge(session).unwrap();
    });
    h.run_steps(3);
    let item = panel(h.state()).item.unwrap();
    let value = h
        .state()
        .session
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| &track.items)
        .find(|value| value.id == item)
        .unwrap();
    let ItemParams::Caption(params) = &value.content else {
        panic!("caption");
    };
    let captions::CueWordTiming::Words { records, .. } = &params.cues.cues[0].word_timing else {
        panic!("native evidence lost on manual merge");
    };
    assert_eq!(records.len(), 4);
    assert!(
        h.query_all(warning()).next().is_some(),
        "manual merge preserves edited protection"
    );
    h.state_mut().session.undo().unwrap();
    h.run_steps(3);
    assert!(h.query_all(warning()).next().is_none());
}

/// One Vietnamese cue of two measured words; the first word is `first`.
fn timed_cue(id: &str, start: i64, text: &str, first: &str, edited: bool) -> captions::Cue {
    let source = lang("vi");
    let split = text.find(' ').unwrap();
    assert!(text.starts_with(first));
    let at = |tick: i64| Tick(tick * TICKS_PER_SECOND);
    let left = time::SourceRange::new(at(start), at(start + 1)).unwrap();
    let right = time::SourceRange::new(at(start + 1), at(start + 2)).unwrap();
    captions::Cue {
        word_timing: captions::CueWordTiming::Words {
            language: source.clone(),
            text: text.into(),
            records: vec![
                captions::VerifiedCueWord {
                    bytes: 0..split,
                    source_range: left,
                    projection: captions::WordProjection::Visible(left),
                    provenance: captions::TimingProvenance::Native,
                },
                captions::VerifiedCueWord {
                    bytes: split + 1..text.len(),
                    source_range: right,
                    projection: captions::WordProjection::Visible(right),
                    provenance: captions::TimingProvenance::Forced,
                },
            ],
        },
        id: id.into(),
        range: time::SourceRange::new(at(start), at(start + 2)).unwrap(),
        source_lang: source.clone(),
        texts: std::collections::BTreeMap::from([(
            source,
            captions::CueText {
                text: text.into(),
                edited,
                translation_ref: None,
            },
        )]),
        style: None,
    }
}

/// Three cues: a filled pause first, second and in an edited cue (edited last).
fn filler_cues() -> CueTrack {
    CueTrack {
        styles: Vec::new(),
        cues: vec![
            timed_cue("cue-0", 1, "ừm chào", "ừm", false),
            timed_cue("cue-1", 3, "chào ừm", "chào", false),
            timed_cue("cue-2", 5, "ừm bạn", "ừm", true),
        ],
    }
}

/// The checkbox automation ids of the cut list, in list order.
fn filler_toggles(h: &Harness<'_, Editor>) -> Vec<String> {
    h.query_all(By::new().predicate(|n| {
        n.author_id()
            .is_some_and(|id| id.starts_with("subtitles.fillers.toggle."))
    }))
    .filter_map(|n| n.accesskit_node().author_id().map(str::to_owned))
    .collect()
}

/// The cues of every caption item in the active scene, in timeline order. A word cut splits
/// the item, and the original id stays on the earliest piece, which may hold no cue.
fn scene_cues(e: &Editor) -> Vec<captions::Cue> {
    e.session
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|t| &t.items)
        .filter_map(|i| match &i.content {
            ItemParams::Caption(p) => Some(p.cues.cues.clone()),
            _ => None,
        })
        .flatten()
        .collect()
}

#[test]
fn filler_cut_list_removes_only_checked_candidates_as_one_undo_step() {
    let mut h = harness(editor_with_cues("vi", filler_cues()), 1.0);
    h.run_steps(3);
    click(&mut h, "subtitles.fillers");
    click(&mut h, "subtitles.fillers.find");
    let toggles = filler_toggles(&h);
    assert_eq!(toggles.len(), 3, "every lexicon candidate starts checked");
    // Unchecking the middle and the edited candidate leaves one checked id to remove.
    click(&mut h, &toggles[1]);
    click(&mut h, &toggles[2]);
    let before = h.state().session.project().clone();
    let history = h.state().session.history_len().0;
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/p1-ai-02-fillers.png");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    h.render().unwrap().save(&path).unwrap();

    click(&mut h, "subtitles.fillers.apply");
    assert_eq!(h.state().session.history_len().0, history + 1);
    // A cut removes measured words from the timeline and never rewrites cue text, so the
    // removal shows in the transcript read model: exactly cue-0's first word is gone.
    let words: Vec<(String, String)> = h
        .state()
        .session
        .transcript()
        .unwrap()
        .cues
        .iter()
        .flat_map(|cue| {
            cue.words.iter().map(move |word| {
                (
                    cue.id.as_str().to_owned(),
                    word.text.as_ref().unwrap().as_str().to_owned(),
                )
            })
        })
        .collect();
    let expected = [
        ("cue-0", "chào"),
        ("cue-1", "chào"),
        ("cue-1", "ừm"),
        ("cue-2", "ừm"),
        ("cue-2", "bạn"),
    ]
    .map(|(cue, word)| (cue.to_owned(), word.to_owned()));
    assert_eq!(words, expected, "the checked filled pause is removed");
    let cues = scene_cues(h.state());
    assert_eq!(
        cues[1].texts.values().next().unwrap().text.as_str(),
        "chào ừm",
        "unchecked candidates keep their words"
    );
    assert!(
        filler_toggles(&h).is_empty(),
        "the list clears after a removal"
    );
    h.get_by_label("Removed 1");
    click(&mut h, "subtitles.fillers.find");
    assert_eq!(
        filler_toggles(&h).len(),
        2,
        "a fresh read finds what remains"
    );

    h.state_mut().session.undo().unwrap();
    h.run_steps(3);
    assert_eq!(h.state().session.project(), &before);
}

#[test]
fn filler_cut_list_asks_before_removing_edited_captions() {
    let mut h = harness(editor_with_cues("vi", filler_cues()), 1.0);
    h.run_steps(3);
    click(&mut h, "subtitles.fillers");
    click(&mut h, "subtitles.fillers.find");
    let before = h.state().session.project().clone();
    let history = h.state().session.history_len().0;

    click(&mut h, "subtitles.fillers.apply");
    h.get_by_label("Some are in edited captions.");
    assert_eq!(h.state().session.project(), &before);
    click(&mut h, "subtitles.fillers.confirm.cancel");
    assert_eq!(h.state().session.project(), &before);
    assert_eq!(h.state().session.history_len().0, history);

    click(&mut h, "subtitles.fillers.apply");
    click(&mut h, "subtitles.fillers.confirm");
    assert_eq!(h.state().session.history_len().0, history + 1);
    assert!(filler_toggles(&h).is_empty());
    h.get_by_label("Removed 3");
    click(&mut h, "subtitles.fillers.find");
    h.get_by_label("No fillers found");
}

#[test]
fn filler_cut_list_reports_missing_timings_and_empty_results() {
    let mut h = harness(editor("vi"), 1.0);
    h.run_steps(3);
    click(&mut h, "subtitles.fillers");
    click(&mut h, "subtitles.fillers.find");
    h.get_by_label("Word timings needed");
    assert!(filler_toggles(&h).is_empty());

    let track = CueTrack {
        styles: Vec::new(),
        cues: vec![timed_cue("cue-0", 1, "chào bạn", "chào", false)],
    };
    let mut h = harness(editor_with_cues("vi", track), 1.0);
    h.run_steps(3);
    click(&mut h, "subtitles.fillers");
    click(&mut h, "subtitles.fillers.find");
    h.get_by_label("No fillers found");
    assert!(filler_toggles(&h).is_empty());
}

/// The consent sheet of the captions panel is open for a classification or a job.
fn approval_sheet_open(e: &Editor) -> bool {
    panel(e).consent.is_some()
}
fn subtitles_idle(e: &Editor) -> bool {
    !panel(e).busy
}

/// A text provider that never reaches the network: it flags every word spelled "ừm", or fails,
/// and counts the calls it receives.
struct ScriptedFillers {
    config: ai::ProviderConfig,
    fail: bool,
    calls: Arc<AtomicUsize>,
}
impl ai::text::fillers::FillerClassifier for ScriptedFillers {
    fn config(&self) -> &ai::ProviderConfig {
        &self.config
    }
    fn capabilities(&self, _: &ai::Scope) -> ai::Capabilities {
        ai::Capabilities {
            model: self.config.model().clone(),
            mode: ai::text::fillers::MODE.into(),
            operation: ai::Operation::ClassifyFillers,
            languages: ["vi", "en", "zh", "zh-Hans", "zh-Hant"].map(lang).to_vec(),
            target_languages: Vec::new(),
            timing: BTreeSet::new(),
            diarization: ai::Support::Unsupported,
            live_audio_input: ai::Support::Unsupported,
            file_result_stream: ai::Support::Unsupported,
            max_upload_bytes: None,
            max_duration_micros: None,
            evidence: ai::Evidence::FixtureVerified,
            revision: "fixture".into(),
        }
    }
    fn estimate(&self, _: &ai::Scope) -> ai::CostEstimate {
        ai::CostEstimate::Free
    }
    fn classify(
        &self,
        input: &ai::text::fillers::FillerInput,
        _: &ai::Request<'_>,
        _: &dyn ai::ExecutionHooks,
    ) -> Result<ai::text::fillers::FillerProposal, ai::Error> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            return Err(ai::Error::Server);
        }
        Ok(ai::text::fillers::FillerProposal {
            fillers: input
                .segments
                .iter()
                .flat_map(|segment| &segment.words)
                .filter(|word| word.text.as_str() == "ừm")
                .map(|word| ai::text::fillers::FillerFinding {
                    id: word.id.clone(),
                    kind: captions::FillerKind::FilledPause,
                })
                .collect(),
        })
    }
}

/// The chapter adapter of the filler access; the cut list never asks for chapters.
struct NoChapters(ai::ProviderConfig);
impl ai::text::chapters::ChapterGenerator for NoChapters {
    fn config(&self) -> &ai::ProviderConfig {
        &self.0
    }
    fn capabilities(&self, _: &ai::Scope) -> ai::Capabilities {
        panic!("chapter capabilities are unused by the cut list")
    }
    fn estimate(&self, _: &ai::Scope) -> ai::CostEstimate {
        ai::CostEstimate::Free
    }
    fn propose(
        &self,
        _: &ai::text::chapters::ChapterInput,
        _: &ai::Request<'_>,
        _: &dyn ai::ExecutionHooks,
    ) -> Result<ai::text::chapters::ChapterProposal, ai::Error> {
        Err(ai::Error::Unsupported)
    }
}

/// The lent text access of each run: the scripted classifier behind that run's own sheet.
fn filler_text_opener(fail: bool, calls: Arc<AtomicUsize>) -> TextOpener {
    Arc::new(move |provider, model, consent| {
        let config = ai::ProviderConfig::new(
            provider.into(),
            model.into(),
            "fixture-v1".into(),
            ai::Location::Remote {
                backend: "fixture".into(),
                base_url: "https://provider.example".into(),
                auth_ref: "fixture".into(),
            },
        )?;
        Ok(engine::providers::TextAccess::new(
            Arc::new(NoChapters(config.clone())),
            Arc::new(ScriptedFillers {
                config,
                fail,
                calls: calls.clone(),
            }),
            Box::new(SharedConsent(consent)),
        ))
    })
}

/// Opens the AI route on the filler fixture editor, with the scripted text provider injected.
fn ai_filler_harness(fail: bool, calls: Arc<AtomicUsize>) -> Harness<'static, Editor> {
    let mut e = editor_with_cues("vi", filler_cues());
    let panel = Subtitles::with_access(e.playback.budget().clone(), generate_opener(false, None))
        .unwrap()
        .with_text_access(filler_text_opener(fail, calls));
    e.set_subtitles(Some(panel));
    let mut h = harness(e, 1.0);
    h.run_steps(3);
    click(&mut h, "subtitles.fillers");
    h
}

#[test]
fn filler_cut_list_finds_with_ai_only_after_the_upload_is_approved() {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut h = ai_filler_harness(false, calls.clone());
    let before = h.state().session.project().clone();
    click(&mut h, "subtitles.fillers.ai");
    wait(&mut h, approval_sheet_open);
    h.get_by_label("Approve the upload to continue.");
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "nothing is sent before approval"
    );
    assert_eq!(h.state().session.project(), &before);

    click(&mut h, "subtitles.consent.approve");
    wait(&mut h, subtitles_idle);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        filler_toggles(&h).len(),
        3,
        "the classified words start checked"
    );

    let history = h.state().session.history_len().0;
    click(&mut h, "subtitles.fillers.apply");
    click(&mut h, "subtitles.fillers.confirm");
    assert_eq!(h.state().session.history_len().0, history + 1);
    assert_ne!(h.state().session.project(), &before);
    h.get_by_label("Removed 3");
}

#[test]
fn filler_cut_list_reports_a_declined_upload_and_sends_nothing() {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut h = ai_filler_harness(false, calls.clone());
    let before = h.state().session.project().clone();
    click(&mut h, "subtitles.fillers.ai");
    wait(&mut h, approval_sheet_open);
    click(&mut h, "subtitles.consent.deny");
    wait(&mut h, subtitles_idle);
    h.get_by_label("Upload not approved. Nothing was sent.");
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(filler_toggles(&h).is_empty());
    assert_eq!(h.state().session.project(), &before);
}

#[test]
fn filler_cut_list_reports_a_provider_failure_after_approval() {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut h = ai_filler_harness(true, calls.clone());
    let before = h.state().session.project().clone();
    click(&mut h, "subtitles.fillers.ai");
    wait(&mut h, approval_sheet_open);
    click(&mut h, "subtitles.consent.approve");
    wait(&mut h, subtitles_idle);
    h.get_by_label("Couldn't find fillers with AI. Try again.");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(filler_toggles(&h).is_empty());
    assert_eq!(h.state().session.project(), &before);
}

/// Transcript pane: word and cue ids of the caption item shown in `h`.
fn transcript_word(h: &Harness<'_, Editor>, cue: &str, word: usize) -> String {
    let item = panel(h.state()).item.unwrap().0;
    format!("subtitles.transcript.word.{item}.{cue}.{word}")
}

fn transcript_cue(h: &Harness<'_, Editor>, cue: &str) -> String {
    let item = panel(h.state()).item.unwrap().0;
    format!("subtitles.transcript.cue.{item}.{cue}")
}

/// Words the engine reads for the active scene, across every caption cue.
fn transcript_words(e: &Editor) -> usize {
    e.session
        .transcript()
        .unwrap()
        .cues
        .iter()
        .map(|cue| cue.words.len())
        .sum()
}

fn transcript_refusal(h: &Harness<'_, Editor>) -> Option<Text> {
    panel(h.state()).transcript_refusal
}

#[test]
fn transcript_pane_shows_no_words_for_an_empty_caption_track() {
    let track = CueTrack {
        styles: Vec::new(),
        cues: Vec::new(),
    };
    let mut h = harness(editor_with_cues("en", track), 1.0);
    h.run_steps(3);
    assert_eq!(transcript_words(h.state()), 0);
    assert!(
        h.query_all(By::new().predicate(|n| n.author_id() == Some("subtitles.transcript.empty")))
            .next()
            .is_some()
    );
    assert_eq!(panel(h.state()).transcript_selected, 0);
    assert!(transcript_refusal(&h).is_none());
}

#[test]
fn transcript_shift_selection_cut_parks_the_playhead_and_undo_restores_the_words() {
    let mut h = harness(editor_with_cues("en", transcript_cues(false)), 1.0);
    h.run_steps(3);
    let before = transcript_words(h.state());
    assert_eq!(before, 4);
    let anchor = transcript_word(&h, "cue-0", 1);
    click(&mut h, &anchor);
    assert_eq!(panel(h.state()).transcript_selected, 1);
    let second = transcript_word(&h, "cue-1", 0);
    click_modifiers(&mut h, &second, egui::Modifiers::SHIFT);
    assert_eq!(panel(h.state()).transcript_selected, 2);
    assert_eq!(
        h.state().playback.position().unwrap(),
        Tick(3 * TICKS_PER_SECOND)
    );
    let history = h.state().session.history_len().0;
    click(&mut h, "subtitles.transcript.delete");
    assert_eq!(transcript_words(h.state()), before - 2);
    assert_eq!(h.state().session.history_len().0, history + 1);
    assert_eq!(panel(h.state()).transcript_selected, 0);
    assert!(transcript_refusal(&h).is_none());
    assert_eq!(
        h.state().playback.position().unwrap(),
        Tick(2 * TICKS_PER_SECOND)
    );
    h.state_mut().session.undo().unwrap();
    h.run_steps(3);
    assert_eq!(transcript_words(h.state()), before);
}

#[test]
fn transcript_merge_joins_selected_adjacent_cues_as_one_undo_step() {
    let mut h = harness(editor_with_cues("en", transcript_cues(false)), 1.0);
    h.run_steps(3);
    let first = transcript_cue(&h, "cue-0");
    click(&mut h, &first);
    let second = transcript_cue(&h, "cue-1");
    click_modifiers(&mut h, &second, egui::Modifiers::COMMAND);
    let history = h.state().session.history_len().0;
    click(&mut h, "subtitles.transcript.merge");
    h.run_steps(3);
    assert_eq!(h.state().session.transcript().unwrap().cues.len(), 1);
    assert_eq!(h.state().session.history_len().0, history + 1);
    assert_eq!(transcript_words(h.state()), 4);
    assert!(transcript_refusal(&h).is_none());
}

#[test]
fn transcript_cut_of_edited_text_is_refused_with_its_reason_and_changes_nothing() {
    let mut h = harness(editor_with_cues("en", transcript_cues(true)), 1.0);
    h.run_steps(3);
    let word = transcript_word(&h, "cue-0", 0);
    click(&mut h, &word);
    let before = h.state().session.project().clone();
    let history = h.state().session.history_len();
    click(&mut h, "subtitles.transcript.delete");
    assert_eq!(transcript_refusal(&h), Some(Text::SubTranscriptProtected));
    assert_eq!(h.state().session.project(), &before);
    assert_eq!(h.state().session.history_len(), history);
    assert_eq!(panel(h.state()).transcript_selected, 1);
}

#[test]
fn transcript_cut_across_scenes_is_refused_and_keeps_the_project() {
    let mut h = harness(editor_with_cues("en", transcript_cues(false)), 1.0);
    h.run_steps(3);
    let word = transcript_word(&h, "cue-0", 0);
    click(&mut h, &word);
    let original = h.state().session.project().data().current_scene_id;
    h.state_mut()
        .session
        .apply(Command::CreateScene {
            name: "Scene 2".into(),
        })
        .unwrap();
    let other = h
        .state()
        .session
        .project()
        .data()
        .scenes
        .iter()
        .map(|scene| scene.id)
        .find(|id| *id != original)
        .unwrap();
    if h.state().session.project().data().current_scene_id != other {
        h.state_mut()
            .session
            .apply(Command::SwitchScene { scene: other })
            .unwrap();
    }
    h.run_steps(3);
    let before = h.state().session.project().clone();
    let history = h.state().session.history_len();
    click(&mut h, "subtitles.transcript.delete");
    assert_eq!(transcript_refusal(&h), Some(Text::SubTranscriptScene));
    assert_eq!(h.state().session.project(), &before);
    assert_eq!(h.state().session.history_len(), history);
}

#[test]
fn transcript_selection_follows_an_external_cut_and_undo_restores_the_view() {
    let mut h = harness(editor_with_cues("en", transcript_cues(false)), 1.0);
    h.run_steps(3);
    let item = panel(h.state()).item.unwrap();
    let word = transcript_word(&h, "cue-1", 0);
    click(&mut h, &word);
    assert_eq!(panel(h.state()).transcript_selected, 1);
    let cut = WordId {
        item,
        cue: "cue-1".into(),
        word: 0,
    };
    h.state_mut()
        .session
        .apply(Command::TranscriptDeleteRange {
            word_ids: vec![cut],
        })
        .unwrap();
    h.run_steps(3);
    assert_eq!(panel(h.state()).transcript_selected, 0);
    assert_eq!(transcript_words(h.state()), 3);
    h.state_mut().session.undo().unwrap();
    h.run_steps(3);
    assert_eq!(transcript_words(h.state()), 4);
}

#[test]
fn transcript_locked_track_marks_its_cues_and_refuses_a_merge() {
    let mut h = harness(editor_with_cues("en", transcript_cues(false)), 1.0);
    h.run_steps(3);
    let track = h.state().session.transcript().unwrap().cues[0].track;
    h.state_mut()
        .session
        .apply(Command::SetTrackLocked {
            track,
            locked: true,
        })
        .unwrap();
    h.run_steps(3);
    let item = panel(h.state()).item.unwrap().0;
    let marker = format!("subtitles.transcript.locked.{item}.cue-0");
    assert!(
        h.query_all(By::new().predicate(|n| n.author_id() == Some(marker.as_str())))
            .next()
            .is_some(),
        "the locked track marks its cue"
    );
    let first = transcript_cue(&h, "cue-0");
    click(&mut h, &first);
    let second = transcript_cue(&h, "cue-1");
    click_modifiers(&mut h, &second, egui::Modifiers::COMMAND);
    let before = h.state().session.project().clone();
    let history = h.state().session.history_len();
    click(&mut h, "subtitles.transcript.merge");
    assert_eq!(transcript_refusal(&h), Some(Text::SubTranscriptLocked));
    assert_eq!(h.state().session.project(), &before);
    assert_eq!(h.state().session.history_len(), history);
    assert_eq!(h.state().session.transcript().unwrap().cues.len(), 2);
}

/// Offscreen screenshot evidence; wgpu rendering stays out of CI.
#[test]
#[ignore = "renders through wgpu; run with --ignored to refresh target/p1-ai-03-transcript-edit.png"]
fn transcript_pane_renders_a_selection_and_its_refusal() {
    let mut h = harness(editor_with_cues("en", transcript_cues(true)), 1.0);
    h.run_steps(3);
    let word = transcript_word(&h, "cue-0", 0);
    click(&mut h, &word);
    click(&mut h, "subtitles.transcript.delete");
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let target = root.join("target");
    std::fs::create_dir_all(&target).unwrap();
    let image = h.render().expect("offscreen render");
    image
        .save(target.join("p1-ai-03-transcript-edit.png"))
        .unwrap();
    assert!(image.width() > 0 && image.height() > 0);
}

/// Two time-adjacent English cues, "Hello world" (1-3 s) and "good day" (3-5 s), each
/// with two measured words. `edited_first` protects the first cue's source text.
fn transcript_cues(edited_first: bool) -> captions::CueTrack {
    captions::CueTrack {
        styles: Vec::new(),
        cues: vec![
            verified_english_cue("cue-0", 1, "Hello world", 5, edited_first),
            verified_english_cue("cue-1", 3, "good day", 4, false),
        ],
    }
}

fn verified_english_cue(
    id: &str,
    start: i64,
    text: &str,
    split: usize,
    edited: bool,
) -> captions::Cue {
    let source = lang("en");
    let at = |seconds: i64| Tick(seconds * TICKS_PER_SECOND);
    let left = time::SourceRange::new(at(start), at(start + 1)).unwrap();
    let right = time::SourceRange::new(at(start + 1), at(start + 2)).unwrap();
    captions::Cue {
        word_timing: captions::CueWordTiming::Words {
            language: source.clone(),
            text: text.into(),
            records: vec![
                captions::VerifiedCueWord {
                    bytes: 0..split,
                    source_range: left,
                    projection: captions::WordProjection::Visible(left),
                    provenance: captions::TimingProvenance::Native,
                },
                captions::VerifiedCueWord {
                    bytes: split + 1..text.len(),
                    source_range: right,
                    projection: captions::WordProjection::Visible(right),
                    provenance: captions::TimingProvenance::Native,
                },
            ],
        },
        id: id.into(),
        range: time::SourceRange::new(at(start), at(start + 2)).unwrap(),
        source_lang: source.clone(),
        texts: std::collections::BTreeMap::from([(
            source,
            captions::CueText {
                text: text.into(),
                edited,
                translation_ref: None,
            },
        )]),
        style: None,
    }
}

/// Types into one automation id after its accessibility click focuses it.
fn type_into(h: &mut Harness<'_, Editor>, id: &str, text: &str) {
    click(h, id);
    h.get(By::new().predicate(|n| n.author_id() == Some(id)))
        .type_text(text);
    h.run_steps(3);
}

/// The plan of the last preview, while its inputs and scope are still the current ones.
fn previewed(h: &mut Harness<'_, Editor>) -> Option<captions::FindPlan> {
    with_panel(h.state_mut(), |panel, _, _| panel.find_plan().cloned())
}

fn source_texts(e: &Editor) -> Vec<String> {
    track(e)
        .cues
        .iter()
        .map(|cue| cue.texts[&cue.source_lang].text.as_str().to_owned())
        .collect()
}

#[test]
fn find_replace_previews_then_writes_one_undo_step_and_skips_edited_cues() {
    let mut h = harness(editor("en"), 1.0);
    h.run_steps(3);
    with_panel(h.state_mut(), |panel, session, _| {
        panel
            .edit_text(session, &"cue-1".into(), lang("en"), "Hello edited")
            .unwrap()
    });
    h.run_steps(3);
    let before = track(h.state());
    type_into(&mut h, "subtitles.find.query", "Hello");
    type_into(&mut h, "subtitles.find.replacement", "Hi");
    click(&mut h, "subtitles.find.preview");
    let plan = previewed(&mut h).unwrap();
    assert_eq!(plan.matches, 3);
    assert_eq!(plan.replaced_matches(), 2);
    assert_eq!(
        plan.cues_with(|o| matches!(o, captions::CueOutcome::Protected)),
        1
    );
    let undo_before = h.state().session.history_len().0;
    click(&mut h, "subtitles.find.replace");
    assert_eq!(h.state().session.history_len().0, undo_before + 1);
    assert_eq!(
        source_texts(h.state()),
        vec!["Hi friend", "Hello edited", "Hi friend"]
    );
    let after = track(h.state());
    for (a, b) in before.cues.iter().zip(&after.cues) {
        assert_eq!(a.range, b.range, "find-replace never moves a cue");
    }
    assert!(previewed(&mut h).is_none(), "a written preview is consumed");
    h.state_mut().session.undo().unwrap();
    h.run_steps(3);
    assert_eq!(track(h.state()), before);
}

#[test]
fn find_replace_scope_follows_the_multi_selection_and_a_change_stales_the_preview() {
    let mut h = harness(editor("en"), 1.0);
    h.run_steps(3);
    type_into(&mut h, "subtitles.find.query", "Hello");
    type_into(&mut h, "subtitles.find.replacement", "Hi");
    click(&mut h, "subtitles.find.preview");
    assert_eq!(previewed(&mut h).unwrap().matches, 3);
    with_panel(h.state_mut(), |panel, session, _| {
        panel.select_cue(session, 0, false, false).unwrap();
        panel.select_cue(session, 1, true, false).unwrap();
    });
    h.run_steps(3);
    assert!(
        previewed(&mut h).is_none(),
        "a changed selection is a changed scope"
    );
    let untouched = track(h.state());
    let refused = with_panel(h.state_mut(), |panel, session, _| {
        panel.find_replace(session).is_err()
    });
    assert!(refused, "replace without a current preview is refused");
    assert_eq!(track(h.state()), untouched);
    click(&mut h, "subtitles.find.preview");
    let plan = previewed(&mut h).unwrap();
    assert_eq!(plan.cues.len(), 2);
    assert_eq!(plan.matches, 2);
}

#[test]
fn find_replace_matches_vietnamese_text_and_keeps_its_diacritics() {
    let mut h = harness(editor("vi"), 1.0);
    h.run_steps(3);
    type_into(&mut h, "subtitles.find.query", "bạn");
    type_into(&mut h, "subtitles.find.replacement", "bè");
    click(&mut h, "subtitles.find.preview");
    assert_eq!(previewed(&mut h).unwrap().matches, 3);
    click(&mut h, "subtitles.find.replace");
    assert_eq!(
        source_texts(h.state()),
        vec!["Chào bè", "Chào bè", "Chào bè"]
    );
}

/// Offscreen screenshot evidence; wgpu rendering stays out of CI.
#[test]
#[ignore = "renders through wgpu; run with --ignored to refresh target/p2-ai-09-bulk-captions.png"]
fn find_replace_section_renders_its_preview() {
    let mut h = harness(editor("en"), 1.0);
    h.run_steps(3);
    type_into(&mut h, "subtitles.find.query", "Hello");
    type_into(&mut h, "subtitles.find.replacement", "Hi");
    click(&mut h, "subtitles.find.preview");
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let target = root.join("target");
    std::fs::create_dir_all(&target).unwrap();
    let image = h.render().expect("offscreen render");
    image
        .save(target.join("p2-ai-09-bulk-captions.png"))
        .unwrap();
    assert!(image.width() > 0 && image.height() > 0);
}

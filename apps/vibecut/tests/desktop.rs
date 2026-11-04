#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]
use egui_kittest::{
    Harness,
    kittest::{By, Queryable},
};
use engine::{ByteBudget, Command, CreatedId, NewItem, Session};
use project::{Color, ItemParams, OverlayParams, Project, ProjectId, TrackKind};
use std::{
    collections::BTreeSet,
    sync::Arc,
    time::{Duration, Instant},
};
use time::{SourceRange, TICKS_PER_SECOND, Tick};
use vibecut::{
    Editor,
    catalog::{Locale, PreferenceStore, Preferences},
    egui,
    playback::{Files, Playback},
};
fn session() -> Session {
    let mut session =
        Session::new(Project::empty(ProjectId(1), "Test".into()).unwrap(), 10).unwrap();
    let outcome = session
        .apply(Command::AddTrack {
            name: "Overlay".into(),
            kind: TrackKind::Overlay,
        })
        .unwrap();
    let CreatedId::Track(track) = outcome.created[0] else {
        panic!("track")
    };
    session
        .apply(Command::InsertItem {
            track,
            item: NewItem {
                animations: Default::default(),
                source_asset: None,
                source_range: SourceRange::new(Tick::ZERO, Tick(TICKS_PER_SECOND * 10)).unwrap(),
                start: Tick::ZERO,
                content: ItemParams::Overlay(OverlayParams {
                    color: Color([255, 0, 0, 255]),
                }),
            },
        })
        .unwrap();
    session
}
fn editor() -> Editor {
    let session = session();
    let mut playback = Playback::new(
        &session,
        ByteBudget::new(64 << 20).unwrap(),
        Arc::new(Files),
    )
    .unwrap();
    playback.set_device_available(true);
    Editor::new(session, playback, Preferences::default())
}
#[test]
fn every_widget_has_unique_locale_stable_id_and_input_reaches_engine() {
    let mut harness = Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .build_ui_state(|ui, editor: &mut Editor| editor.ui(ui), editor());
    harness.run_steps(3);
    let before = harness.state().inspect().unwrap();
    let ids: BTreeSet<_> = before.widgets.iter().map(|w| w.id).collect();
    assert_eq!(ids.len(), before.widgets.len());
    assert!(
        ids.contains("edit.undo")
            && ids.contains("transport.scrub")
            && ids.contains("layout.timeline")
    );
    for id in &ids {
        assert_eq!(
            harness
                .query_all(By::new().predicate(|n| n.author_id() == Some(*id)))
                .count(),
            1,
            "id {id}"
        );
    }
    let initial = harness.state().session.project().clone();
    harness
        .get(By::new().predicate(|n| n.author_id() == Some("edit.undo")))
        .click();
    harness.run_steps(3);
    assert_ne!(harness.state().session.project(), &initial);
    harness
        .get(By::new().predicate(|n| n.author_id() == Some("edit.redo")))
        .click();
    harness.run_steps(3);
    assert_eq!(harness.state().session.project(), &initial);
    harness
        .get(By::new().predicate(|n| n.author_id() == Some("appearance.locale")))
        .click();
    harness.run_steps(3);
    assert_eq!(harness.state().preferences.locale, Locale::Vi);
    assert_eq!(
        harness
            .state()
            .inspect()
            .unwrap()
            .widgets
            .iter()
            .map(|w| w.id)
            .collect::<BTreeSet<_>>(),
        ids
    );
    harness
        .get(By::new().predicate(|n| n.author_id() == Some("transport.forward")))
        .click();
    harness.run_steps(3);
    assert_eq!(
        harness.state().playback.position().unwrap(),
        Tick(TICKS_PER_SECOND)
    );
}
#[test]
fn preview_uses_budget_skips_late_video_and_invalidates_seek_and_clear() {
    let mut editor = editor();
    let port = editor.playback.audio_port();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        editor.playback.poll().unwrap();
        if editor.playback.frame().is_some() {
            break;
        }
        assert!(Instant::now() < deadline, "{:?}", editor.playback.error());
        std::thread::sleep(Duration::from_millis(2));
    }
    let first_epoch = editor.playback.frame().unwrap().epoch;
    editor.activate("transport.forward").unwrap();
    assert!(editor.playback.frame().is_none());
    editor.activate("transport.play").unwrap();
    let mut audio = [0.0; 960];
    for _ in 0..201 {
        port.fill(&mut audio, 0);
        editor.playback.poll().unwrap();
    }
    assert!(editor.playback.position().unwrap() >= Tick(TICKS_PER_SECOND * 3));
    editor.playback.clear().unwrap();
    assert!(editor.playback.playing());
    assert!(editor.playback.frame().is_none());
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        editor.playback.poll().unwrap();
        if let Some(frame) = editor.playback.frame() {
            assert!(frame.epoch > first_epoch);
            break;
        }
        assert!(Instant::now() < deadline, "{:?}", editor.playback.error());
        std::thread::sleep(Duration::from_millis(2));
    }
    let memory = editor.playback.budget().snapshot().unwrap();
    assert!(memory.used <= memory.limit);
    editor.activate("transport.pause").unwrap();
    let at = editor.playback.position().unwrap();
    port.fill(&mut audio, 0);
    assert!(audio.iter().all(|x| *x == 0.0));
    assert_eq!(editor.playback.position().unwrap(), at);
    assert!(
        editor
            .playback
            .seek(Tick(-1), false, editor.session.project().clone())
            .is_err()
    );
}
#[test]
fn frame_exceeding_budget_is_typed_and_shutdown_joins_workers() {
    let session = session();
    let budget = ByteBudget::new(2 << 20).unwrap();
    let mut playback = Playback::new(&session, budget.clone(), Arc::new(Files)).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        playback.poll().unwrap();
        if playback.error().is_some() {
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(playback.frame().is_none());
    assert!(budget.snapshot().unwrap().used <= 2 << 20);
    drop(playback);
    assert_eq!(budget.snapshot().unwrap().used, 0);
}
#[test]
fn preference_store_roundtrips_locale_and_refuses_hostile_size() {
    let dir = tempfile::tempdir().unwrap();
    let store = PreferenceStore::new(dir.path().join("preferences.json"));
    let p = Preferences {
        locale: Locale::Vi,
        ..Default::default()
    };
    store.save(&p).unwrap();
    assert_eq!(store.load().unwrap().locale, Locale::Vi);
    std::fs::write(store.path(), vec![b' '; 4097]).unwrap();
    assert!(store.load().is_err());
}

struct PanickingSources;
impl vibecut::playback::Sources for PanickingSources {
    fn create(&self) -> Box<dyn render::SourceProvider> {
        panic!("injected worker failure")
    }
}
#[test]
fn engine_catches_preview_worker_panic_and_app_reports_failure() {
    let session = session();
    let mut playback = Playback::new(
        &session,
        ByteBudget::new(64 << 20).unwrap(),
        Arc::new(PanickingSources),
    )
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Err(error) = playback.poll() {
            assert!(matches!(error, vibecut::Error::Worker(_)));
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
}
#[test]
fn capture_headless_has_bounded_typed_timeout() {
    let context = egui::Context::default();
    let capture =
        vibecut::capture::Capture::request(&context, None, Duration::from_millis(1)).unwrap();
    std::thread::sleep(Duration::from_millis(2));
    assert!(matches!(
        capture.poll(),
        Err(vibecut::Error::CaptureTimeout)
    ));
    assert!(vibecut::capture::Capture::request(&context, None, Duration::from_secs(6)).is_err());
}

#[test]
fn failed_device_pauses_without_wall_clock_substitution() {
    let mut editor = editor();
    editor.activate("transport.play").unwrap();
    editor.playback.audio_port().fill(&mut [0.0; 960], 0);
    editor.playback.audio_port().fail();
    assert!(matches!(
        editor.playback.poll(),
        Err(vibecut::Error::PreviewUnavailable(_))
    ));
    assert!(!editor.playback.playing());
    let at = editor.playback.position().unwrap();
    editor.playback.poll().unwrap();
    assert_eq!(editor.playback.position().unwrap(), at);
    assert!(editor.activate("transport.play").is_err());
}
#[test]
fn repeated_forward_backward_seeks_and_clears_retain_no_unbounded_frames() {
    let mut editor = editor();
    for n in 0..200 {
        let at = Tick((n % 10) * TICKS_PER_SECOND);
        editor
            .playback
            .seek(at, false, editor.session.project().clone())
            .unwrap();
        editor.playback.poll().unwrap();
        if n % 5 == 0 {
            editor.playback.clear().unwrap();
        }
        let snapshot = editor.playback.budget().snapshot().unwrap();
        assert!(snapshot.used <= snapshot.limit);
    }
    let budget = editor.playback.budget().clone();
    drop(editor);
    assert_eq!(budget.snapshot().unwrap().used, 0);
}

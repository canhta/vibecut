//! Uses the real dialog, persisted preference action and localized widget tree.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
use super::*;
use crate::{
    editor::{Editor, context::bench::Bench},
    i18n::Locale,
    preferences::Preferences,
    services::playback::{Files, Playback},
};
use egui_kittest::kittest::{By, Queryable};
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};
struct Reply {
    calls: AtomicUsize,
    body: Mutex<Vec<u8>>,
    status: u16,
}
impl transport::HttpTransport for Reply {
    fn send(
        &self,
        request: transport::HttpRequest,
    ) -> Result<transport::HttpResponse, transport::Error> {
        assert_eq!(request.method, "GET");
        assert!(request.body.is_empty());
        self.calls.fetch_add(1, Ordering::SeqCst);
        // A zero fixture status represents no HTTP response while offline.
        if self.status == 0 {
            return Err(transport::Error::Network);
        }
        Ok(transport::HttpResponse {
            status: self.status,
            headers: vec![],
            body: self.body.lock().unwrap().clone(),
        })
    }
}
fn paint(
    state: &mut super::super::State,
    cx: &Context<'_>,
    ui: &mut egui::Ui,
) -> Vec<super::super::Action> {
    show(
        &mut state.preferences,
        cx,
        ui.ctx(),
        &engine::providers::Credentials::new(Some("synthetic".into()), None, None),
    )
    .into_iter()
    .map(super::super::Action::Preferences)
    .collect()
}
fn bench(locale: Locale, http: Arc<Reply>) -> Bench<super::super::State, super::super::Action> {
    let session = engine::Session::new(
        project::Project::empty(project::ProjectId(23), "Settings".into()).unwrap(),
        10,
    )
    .unwrap();
    let playback = Playback::new(
        &session,
        engine::ByteBudget::new(64 << 20).unwrap(),
        Arc::new(Files),
    )
    .unwrap();
    let mut editor = Editor::new(
        session,
        playback,
        Preferences {
            locale,
            ..Default::default()
        },
    );
    editor.dialogs.preferences.models_open = true;
    editor.dialogs.preferences.transport = Some(http);
    Bench::hosted(
        editor,
        |editor| std::mem::take(&mut editor.dialogs),
        |editor, state| editor.dialogs = state,
        paint,
        super::super::apply,
        egui::vec2(1280.0, 900.0),
    )
}
#[test]
fn localized_model_preferences_custom_selection_persists_and_refresh_is_explicit() {
    for locale in [Locale::En, Locale::Vi] {
        let http = Arc::new(Reply {
            calls: AtomicUsize::new(0),
            body: Mutex::new(br#"{"data":[{"id":"fixture-listed"}]}"#.to_vec()),
            status: 200,
        });
        let mut bench = bench(locale, http.clone());
        bench.harness_mut().run_steps(4);
        assert_eq!(http.calls.load(Ordering::SeqCst), 0);
        assert!(
            !bench
                .editor()
                .dialogs
                .preferences
                .catalogs
                .get("anthropic")
                .unwrap()
                .has_key()
        );
        bench.click("preferences.ai.openai.transcribe.model");
        bench.click("preferences.ai.openai.transcribe.custom");
        let id = "preferences.ai.openai.transcribe.input";
        bench.click(id);
        bench
            .harness()
            .get(By::new().predicate(|node| node.author_id() == Some(id)))
            .type_text("fixture-custom");
        bench.harness_mut().run_steps(6);
        assert_eq!(
            bench
                .editor()
                .dialogs
                .preferences
                .draft
                .as_ref()
                .map(|draft| draft.value.as_str()),
            Some("fixture-custom")
        );
        bench.click("preferences.ai.openai.transcribe.apply");
        let selected = bench
            .editor()
            .preferences
            .ai_models
            .model("openai", Operation::Transcribe)
            .to_owned();
        assert_eq!(selected, "fixture-custom");
        assert_eq!(http.calls.load(Ordering::SeqCst), 0);
        bench.click("preferences.ai.openai.refresh");
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while bench
            .editor()
            .dialogs
            .preferences
            .catalogs
            .get("openai")
            .unwrap()
            .loading()
        {
            bench.step();
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert_eq!(http.calls.load(Ordering::SeqCst), 1);
        bench.click("preferences.ai.openai.transcribe.model");
        bench.click("preferences.ai.openai.transcribe.option.1");
        assert_eq!(
            bench
                .editor()
                .preferences
                .ai_models
                .model("openai", Operation::Transcribe),
            "fixture-listed"
        );
        let options =
            bench
                .editor()
                .preferences
                .ai_models
                .options("openai", Operation::Transcribe, &[]);
        assert!(options.contains(&selected));
    }
}
#[test]
fn localized_refresh_failure_and_empty_results_keep_custom_available() {
    for locale in [Locale::En, Locale::Vi] {
        for (status, body, message) in [
            (200, br#"{"data":[]}"#.as_slice(), Text::ModelRemoteEmpty),
            (0, b"".as_slice(), Text::ModelRemoteListFailed),
            (
                401,
                b"synthetic-private-body".as_slice(),
                Text::ModelRemoteListFailed,
            ),
        ] {
            let http = Arc::new(Reply {
                calls: AtomicUsize::new(0),
                body: Mutex::new(body.to_vec()),
                status,
            });
            let mut bench = bench(locale, http.clone());
            bench.harness_mut().run_steps(4);
            bench.click("preferences.ai.openai.refresh");
            let deadline = std::time::Instant::now() + Duration::from_secs(3);
            while bench
                .editor()
                .dialogs
                .preferences
                .catalogs
                .get("openai")
                .unwrap()
                .loading()
            {
                bench.step();
                assert!(std::time::Instant::now() < deadline);
                std::thread::yield_now();
            }
            bench.step();
            assert!(
                bench
                    .harness()
                    .query_by_label(message.text(locale))
                    .is_some()
            );
            bench.click("preferences.ai.openai.transcribe.model");
            bench.click("preferences.ai.openai.transcribe.custom");
            assert!(
                bench
                    .harness()
                    .query(
                        By::new().predicate(|node| node.author_id()
                            == Some("preferences.ai.openai.transcribe.input"))
                    )
                    .is_some()
            );
        }
    }
}

#[test]
fn settings_modal_applies_preferences_and_shortcut_transfers_through_the_picker_seam() {
    for locale in [Locale::En, Locale::Vi] {
        let http = Arc::new(Reply {
            calls: AtomicUsize::new(0),
            body: Mutex::new(Vec::new()),
            status: 200,
        });
        let mut bench = bench(locale, http.clone());
        bench.editor_mut().dialogs.preferences.open = true;
        bench.editor_mut().dialogs.preferences.models_open = false;
        bench.harness_mut().run_steps(4);
        bench.click("settings.tab.draft");
        bench.click("settings.draft.fps");
        bench.click("settings.draft.fps.60");
        assert_eq!(bench.editor().preferences.default_fps, 60);
        bench.click("settings.tab.performance");
        bench.click("settings.performance.hardware");
        bench.click("settings.performance.hardware.Off");
        assert_eq!(
            bench.editor().preferences.hardware_acceleration,
            crate::preferences::HardwareAcceleration::Off
        );
        bench.click("settings.performance.cache");
        bench.click("settings.performance.cache.1024");
        assert_eq!(bench.editor().preferences.cache_limit_bytes, 1024 << 20);
        bench.click("settings.performance.cache.clear");
        // Settings clears through the one cache-maintenance path the timeline owns.
        // The frame drains the request at once, so the timeline's phase shows it; this
        // bench has no media access, so the refused clear surfaces as a failed phase.
        let cache = bench.editor().timeline.cache_ui();
        assert_eq!(
            cache.phase,
            crate::editor::context::CachePhase::Failed,
            "Clear cache requests the shared clear"
        );
        assert!(cache.cause.is_some());
        bench.harness_mut().run_steps(2);
        let name = if matches!(locale, Locale::En) {
            "en"
        } else {
            "vi"
        };
        let shot = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(format!("../../target/p1-75-settings-{name}.png"));
        bench.harness_mut().render().unwrap().save(&shot).unwrap();
        bench.click("settings.tab.general");
        bench.click("settings.general.shortcuts.export");
        assert!(matches!(
            bench.editor_mut().take_lifecycle_action(),
            Some(crate::services::lifecycle::EditorAction::KeybindingsExport)
        ));
        let dir = tempfile::tempdir().unwrap();
        let exported = dir.path().join("keys.json");
        bench
            .editor_mut()
            .keybindings_export_picked(exported.clone())
            .unwrap();
        let profile = std::fs::read_to_string(&exported).unwrap();
        assert!(profile.contains("\"version\":1"));
        let merge = dir.path().join("merge.json");
        std::fs::write(
            &merge,
            r#"{"format":"vibecut-keybindings","version":1,"bindings":{"timeline.snap":["Ctrl+Shift+K"],"future.action":["F9"]},"isCustomized":["timeline.snap"]}"#,
        )
        .unwrap();
        bench
            .editor_mut()
            .keybindings_import_picked(merge, false)
            .unwrap();
        assert_eq!(
            bench.editor().preferences.keybindings.bindings["timeline.snap"],
            ["Ctrl+Shift+K"]
        );
        assert!(matches!(
            bench.editor().dialogs.preferences.shortcut_notice,
            Some(super::settings::ShortcutNotice::Imported { unknown: 1 })
        ));
        // A corrupt file is refused before any binding changes.
        let corrupt = dir.path().join("corrupt.json");
        std::fs::write(&corrupt, "{").unwrap();
        let before = bench.editor().preferences.keybindings.clone();
        assert!(
            bench
                .editor_mut()
                .keybindings_import_picked(corrupt, true)
                .is_err()
        );
        assert_eq!(bench.editor().preferences.keybindings, before);
    }
}

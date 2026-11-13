//! Uses the real dialog, persisted preference action and localized widget tree.
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
use super::*;
use crate::{
    editor::{Editor, frame::bench::Bench},
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
        crate::editor::Editor::apply_dialogs,
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
        // The text-task row stores one provider; translation and filler runs read it.
        assert_eq!(bench.editor().preferences.subtitles.text_provider, "openai");
        bench.click("preferences.text.provider.combo");
        bench.click("preferences.text.provider.anthropic");
        assert_eq!(
            bench.editor().preferences.subtitles.text_provider,
            "anthropic"
        );
        let subtitles = &bench.editor().preferences.subtitles;
        let models = &bench.editor().preferences.ai_models;
        assert_eq!(
            subtitles.translation_model,
            models.model("anthropic", Operation::Translate)
        );
        assert_eq!(
            subtitles.fillers_model,
            models.model("anthropic", Operation::ClassifyFillers)
        );
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
        assert!(matches!(cache.problem(), Some(Text::CacheUnavailable)));
        bench.harness_mut().run_steps(2);
        // Hardware acceleration and the storage limit say they apply after restart.
        assert!(
            bench
                .harness()
                .query_by_label(Text::SettingsHardwareHint.text(locale))
                .is_some()
        );
        assert!(
            bench
                .harness()
                .query_by_label(Text::SettingsCacheHint.text(locale))
                .is_some()
        );
        assert!(
            bench
                .harness()
                .query_by_label(Text::CacheUnavailable.text(locale))
                .is_some()
        );
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
        // Replace drops the merged customization and keeps only the file's entries.
        let replace = dir.path().join("replace.json");
        std::fs::write(
            &replace,
            r#"{"format":"vibecut-keybindings","version":1,"bindings":{"timeline.snap":["Ctrl+Shift+J"]},"isCustomized":["timeline.snap"]}"#,
        )
        .unwrap();
        bench
            .editor_mut()
            .keybindings_import_picked(replace, true)
            .unwrap();
        assert_eq!(
            bench.editor().preferences.keybindings.bindings["timeline.snap"],
            ["Ctrl+Shift+J"]
        );
        assert!(matches!(
            bench.editor().dialogs.preferences.shortcut_notice,
            Some(super::settings::ShortcutNotice::Imported { unknown: 0 })
        ));
        // A corrupt file and a newer-schema file are refused before any binding changes.
        let newer = dir.path().join("newer.json");
        std::fs::write(
            &newer,
            r#"{"format":"vibecut-keybindings","version":2,"bindings":{"timeline.snap":["F8"]},"isCustomized":["timeline.snap"]}"#,
        )
        .unwrap();
        let corrupt = dir.path().join("corrupt.json");
        std::fs::write(&corrupt, "{").unwrap();
        let before = bench.editor().preferences.keybindings.clone();
        for refused in [corrupt, newer] {
            assert!(
                bench
                    .editor_mut()
                    .keybindings_import_picked(refused, true)
                    .is_err()
            );
            assert!(matches!(
                bench.editor().dialogs.preferences.shortcut_notice,
                Some(super::settings::ShortcutNotice::Refused)
            ));
            assert_eq!(bench.editor().preferences.keybindings, before);
        }
        bench.harness_mut().run_steps(2);
        assert!(
            bench
                .harness()
                .query_by_label(Text::ShortcutsRefused.text(locale))
                .is_some()
        );
    }
}

#[test]
fn settings_cache_rows_show_clearing_disabled_clear_and_bytes_freed() {
    use crate::editor::context::{CachePhase, CacheUi};
    for locale in [Locale::En, Locale::Vi] {
        let http = Arc::new(Reply {
            calls: AtomicUsize::new(0),
            body: Mutex::new(Vec::new()),
            status: 200,
        });
        let mut bench = bench(locale, http);
        bench.editor_mut().dialogs.preferences.open = true;
        bench.editor_mut().dialogs.preferences.models_open = false;
        bench.harness_mut().run_steps(4);
        bench.click("settings.tab.performance");
        // A maintenance worker is clearing: the row says so and clear cannot start another.
        bench.editor_mut().timeline.script_cache(CacheUi {
            phase: CachePhase::Clearing,
            ..Default::default()
        });
        bench.harness_mut().run_steps(2);
        assert!(
            bench
                .harness()
                .query_by_label(Text::CacheClearing.text(locale))
                .is_some()
        );
        bench.click("settings.performance.cache.clear");
        bench.harness_mut().run_steps(2);
        assert_eq!(
            bench.editor().timeline.cache_ui().phase,
            CachePhase::Clearing,
            "a disabled clear button starts no request"
        );
        // A finished clear reports the bytes it freed and what is still in use.
        let report = engine::cache::CacheMaintenanceReport {
            removed_bytes: 3 << 20,
            disk_bytes: 5 << 20,
            in_use: 1,
            ..Default::default()
        };
        bench.editor_mut().timeline.script_cache(CacheUi {
            phase: CachePhase::Idle,
            report: Some(Arc::new(report)),
            ..Default::default()
        });
        bench.harness_mut().run_steps(2);
        let freed = crate::i18n::format(
            locale,
            Text::SettingsCacheFreed,
            &[("freed", "3"), ("after", "2"), ("kept", "1")],
        );
        assert!(bench.harness().query_by_label(&freed).is_some());
    }
}

#[test]
fn settings_clear_during_an_active_job_reports_busy_end_to_end() {
    use crate::editor::context::CachePhase;
    use egui_kittest::kittest::NodeT;
    for locale in [Locale::En, Locale::Vi] {
        let http = Arc::new(Reply {
            calls: AtomicUsize::new(0),
            body: Mutex::new(Vec::new()),
            status: 200,
        });
        let mut bench = bench(locale, http);
        // The real media cache: the active job holds its lock while Settings clears.
        let dir = tempfile::tempdir().unwrap();
        let budget = engine::ByteBudget::new(64 << 20).unwrap();
        let (artifacts, _) =
            engine::cache::ArtifactCache::open(&dir.path().join("waveform"), budget.clone(), 128)
                .unwrap();
        let thumbnails = engine::thumbnails::ThumbnailCache::open(
            &dir.path().join("thumbnails"),
            budget.clone(),
            128,
        )
        .unwrap();
        let access = Arc::new(Mutex::new(engine::control::MediaAccess::new(
            artifacts,
            thumbnails,
            engine::luts::LutStore::open(&dir.path().join("luts")).unwrap(),
        )));
        bench.editor_mut().set_media(access.clone());
        bench.editor_mut().dialogs.preferences.open = true;
        bench.editor_mut().dialogs.preferences.models_open = false;
        bench.harness_mut().run_steps(4);
        bench.click("settings.tab.performance");
        // An active job holds the media cache while the clear the button routes arrives. The
        // frame's own display reads lock the cache too, so the job's hold spans the routed
        // request alone, as a worker's short hold does, and ends before the next frame.
        let clear =
            By::new().predicate(|n| n.author_id() == Some("settings.performance.cache.clear"));
        assert!(!bench.harness().get(clear).accesskit_node().is_disabled());
        {
            let _job = access.lock().unwrap();
            bench
                .editor_mut()
                .timeline
                .request_cache(crate::editor::context::CacheAction::Clear);
        }
        bench.harness_mut().run_steps(2);
        let cache = bench.editor().timeline.cache_ui();
        assert_eq!(cache.phase, CachePhase::Failed);
        assert!(matches!(cache.problem(), Some(Text::CacheBusy)));
        assert!(
            bench
                .harness()
                .query_by_label(Text::CacheBusy.text(locale))
                .is_some()
        );
    }
}

#[test]
fn cache_problem_names_a_busy_clear_typed() {
    use crate::editor::context::CacheUi;
    let busy = CacheUi {
        cause: Some(std::sync::Arc::new(engine::control::Error::Runtime(
            engine::RuntimeError::CacheBusy,
        ))),
        ..Default::default()
    };
    assert!(matches!(busy.problem(), Some(Text::CacheBusy)));
    assert!(CacheUi::default().problem().is_none());
}

#[test]
fn invalid_custom_model_draft_shows_inline_notice() {
    for locale in [Locale::En, Locale::Vi] {
        let http = Arc::new(Reply {
            calls: AtomicUsize::new(0),
            body: Mutex::new(Vec::new()),
            status: 200,
        });
        let mut bench = bench(locale, http);
        bench.harness_mut().run_steps(4);
        bench.click("preferences.ai.openai.transcribe.model");
        bench.click("preferences.ai.openai.transcribe.custom");
        bench.harness_mut().run_steps(4);
        // An empty draft is not a valid model id, so the row names the problem inline.
        assert!(
            bench
                .harness()
                .query_by_label(Text::ModelRemoteInvalid.text(locale))
                .is_some()
        );
    }
}

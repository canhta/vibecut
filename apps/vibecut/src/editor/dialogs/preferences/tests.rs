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
    editor.dialogs.preferences.open = true;
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

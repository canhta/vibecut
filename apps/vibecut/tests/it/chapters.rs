#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
//! Chapters section through the real panel: generation waits for consent, the edited title is
//! what export writes, and every refusal and failure is shown in place.
use crate::subtitles::fixture;
use captions::CueTrack;
use egui_kittest::{
    Harness,
    kittest::{By, Queryable},
};
use engine::providers::TextAccess;
use fixture::{editor, editor_with_cues, lang};
use std::{
    collections::BTreeSet,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
use vibecut::{
    Editor, Subtitles, SubtitlesInspection, egui,
    services::{
        ai_access::{Opener, SharedConsent, TextOpener, TextTask},
        lifecycle::EditorAction,
    },
};

fn harness(editor: Editor) -> Harness<'static, Editor> {
    crate::gpu::builder()
        .with_size(egui::vec2(1440.0, 1000.0))
        .with_pixels_per_point(1.0)
        .build_ui_state(|ui, e: &mut Editor| e.ui(ui), editor)
}

fn click(h: &mut Harness<'_, Editor>, id: &str) {
    h.query_all(By::new().predicate(|n| n.author_id() == Some(id)))
        .next()
        .unwrap_or_else(|| panic!("missing {id}; {:?}", panel(h.state())))
        .click_accesskit();
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
        assert!(Instant::now() < until, "chapters timeout");
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn panel(e: &Editor) -> SubtitlesInspection {
    e.inspect().unwrap().subtitles.unwrap()
}

fn approval_sheet_open(e: &Editor) -> bool {
    panel(e).consent.is_some()
}

fn subtitles_idle(e: &Editor) -> bool {
    !panel(e).busy
}

/// A fresh, empty folder under the system temp directory for one test's sidecars.
fn scratch_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("vibecut-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// The transcription side is never needed by chapters; a run that reaches it is a bug.
fn no_transcriber() -> Opener {
    Arc::new(|_, _| {
        Err(engine::control::Error::Edit(
            engine::EditError::Unsupported("the chapter tests never transcribe"),
        ))
    })
}

/// Holds a provider call inside the adapter until the test releases it. The adapter announces
/// that it is inside the call first, so the test acts at a known point rather than after a delay.
struct Gate {
    entered: Mutex<mpsc::SyncSender<()>>,
    release: Mutex<mpsc::Receiver<()>>,
}
impl Gate {
    fn hold(&self) {
        let _ = self.entered.lock().unwrap().send(());
        let _ = self.release.lock().unwrap().recv();
    }
}

/// A chapter adapter that never reaches the network: it proposes one chapter at the first
/// transcript segment, or fails, and counts the calls it receives.
struct ScriptedChapters {
    config: ai::ProviderConfig,
    fail: bool,
    calls: Arc<AtomicUsize>,
    gate: Option<Arc<Gate>>,
}
impl ai::text::chapters::ChapterGenerator for ScriptedChapters {
    fn config(&self) -> &ai::ProviderConfig {
        &self.config
    }
    fn capabilities(&self, _: &ai::Scope) -> ai::Capabilities {
        ai::Capabilities {
            model: self.config.model().clone(),
            mode: ai::text::chapters::MODE.into(),
            operation: ai::Operation::GenerateChapters,
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
    fn propose(
        &self,
        input: &ai::text::chapters::ChapterInput,
        _: &ai::Request<'_>,
        _: &dyn ai::ExecutionHooks,
    ) -> Result<ai::text::chapters::ChapterProposal, ai::Error> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if let Some(gate) = &self.gate {
            gate.hold();
        }
        if self.fail {
            return Err(ai::Error::Server);
        }
        let first = input.segments.first().ok_or(ai::Error::Unsupported)?;
        Ok(ai::text::chapters::ChapterProposal {
            title: "Generated title".into(),
            description: "Generated description".into(),
            chapters: vec![ai::text::chapters::ChapterProposalEntry {
                start_segment_id: first.id.clone(),
                title: "Opening".into(),
            }],
        })
    }
}

/// The filler adapter of the chapter access; the chapters section never classifies fillers.
struct NoFillers(ai::ProviderConfig);
impl ai::text::fillers::FillerClassifier for NoFillers {
    fn config(&self) -> &ai::ProviderConfig {
        &self.0
    }
    fn capabilities(&self, _: &ai::Scope) -> ai::Capabilities {
        panic!("filler capabilities are unused by chapters")
    }
    fn estimate(&self, _: &ai::Scope) -> ai::CostEstimate {
        ai::CostEstimate::Free
    }
    fn classify(
        &self,
        _: &ai::text::fillers::FillerInput,
        _: &ai::Request<'_>,
        _: &dyn ai::ExecutionHooks,
    ) -> Result<ai::text::fillers::FillerProposal, ai::Error> {
        Err(ai::Error::Unsupported)
    }
}

/// The lent text access of each run: the scripted chapter adapter behind that run's own sheet.
fn chapter_text_opener(fail: bool, calls: Arc<AtomicUsize>) -> TextOpener {
    text_opener(fail, calls, None)
}

fn text_opener(fail: bool, calls: Arc<AtomicUsize>, gate: Option<Arc<Gate>>) -> TextOpener {
    Arc::new(move |task, provider, model, consent| {
        assert_eq!(task, TextTask::Chapters);
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
        Ok(TextAccess::new(
            Arc::new(ScriptedChapters {
                config: config.clone(),
                fail,
                calls: calls.clone(),
                gate: gate.clone(),
            }),
            Arc::new(NoFillers(config)),
            Box::new(SharedConsent(consent)),
        ))
    })
}

/// The Captions tab with a Vietnamese transcript and the chapters section expanded.
fn chapter_harness(text: TextOpener, track: Option<CueTrack>) -> Harness<'static, Editor> {
    let mut e = match track {
        Some(track) => editor_with_cues("vi", track),
        None => editor("vi"),
    };
    let panel = Subtitles::with_access(e.playback.budget().clone(), no_transcriber())
        .unwrap()
        .with_text_access(text);
    e.set_subtitles(Some(panel));
    let mut h = harness(e);
    h.run_steps(3);
    click(&mut h, "captions.section.chapters");
    h
}

/// Replace the title draft with `text`: focus the field, select all, type.
fn retype_title(h: &mut Harness<'_, Editor>, text: &str) {
    h.get(By::new().predicate(|n| n.author_id() == Some("subtitles.chapters.title")))
        .focus();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text(text.into()));
    h.run_steps(3);
}

/// Generate, then approve the one upload, and wait for the chapter list to load.
fn generate_approved(h: &mut Harness<'_, Editor>) {
    click(h, "subtitles.chapters.generate");
    wait(h, approval_sheet_open);
    click(h, "subtitles.consent.approve");
    wait(h, subtitles_idle);
}

#[test]
fn chapters_generate_waits_for_approval_and_export_writes_the_edited_title() {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut h = chapter_harness(chapter_text_opener(false, calls.clone()), None);
    let before = h.state().session.project().clone();
    click(&mut h, "subtitles.chapters.generate");
    wait(&mut h, approval_sheet_open);
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
        h.state().session.project(),
        &before,
        "generation never edits the project"
    );

    retype_title(&mut h, "Edited title");
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/p1-ai-05-chapters.png");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    h.render().unwrap().save(&path).unwrap();
    let dir = scratch_dir("chapters-edited");
    h.state_mut()
        .subtitles_mut()
        .unwrap()
        .export_chapters_to(dir.clone());
    h.run_steps(3);
    h.get_by_label("Saved 2 files");
    assert_eq!(
        h.state().session.project(),
        &before,
        "export never edits the project"
    );

    let json = std::fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.extension().is_some_and(|ext| ext == "json"))
        .unwrap();
    let document: serde_json::Value =
        serde_json::from_slice(&std::fs::read(json).unwrap()).unwrap();
    assert_eq!(document["title"], "Edited title");
    assert_eq!(document["description"], "Generated description");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn chapters_export_asks_the_window_for_a_folder_and_writes_nothing_yet() {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut h = chapter_harness(chapter_text_opener(false, calls), None);
    generate_approved(&mut h);
    let before = h.state().session.project().clone();
    click(&mut h, "subtitles.chapters.export");
    assert_eq!(
        h.state_mut().take_lifecycle_action(),
        Some(EditorAction::ChapterFolder),
        "export asks the window's picker for a folder"
    );
    assert_eq!(h.state().session.project(), &before);
}

#[test]
fn chapters_export_refuses_an_existing_pair_and_says_so() {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut h = chapter_harness(chapter_text_opener(false, calls), None);
    generate_approved(&mut h);
    let dir = scratch_dir("chapters-existing");
    h.state_mut()
        .subtitles_mut()
        .unwrap()
        .export_chapters_to(dir.clone());
    h.run_steps(3);
    h.get_by_label("Saved 2 files");

    h.state_mut()
        .subtitles_mut()
        .unwrap()
        .export_chapters_to(dir.clone());
    h.run_steps(3);
    h.get_by_label("Chapter files already exist in this folder.");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn chapters_declined_upload_sends_nothing_and_says_so() {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut h = chapter_harness(chapter_text_opener(false, calls.clone()), None);
    let before = h.state().session.project().clone();
    click(&mut h, "subtitles.chapters.generate");
    wait(&mut h, approval_sheet_open);
    click(&mut h, "subtitles.consent.deny");
    wait(&mut h, subtitles_idle);
    h.get_by_label("Upload not approved. Nothing was sent.");
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(h.state().session.project(), &before);
}

#[test]
fn chapters_provider_failure_after_approval_shows_one_line() {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut h = chapter_harness(chapter_text_opener(true, calls.clone()), None);
    click(&mut h, "subtitles.chapters.generate");
    wait(&mut h, approval_sheet_open);
    click(&mut h, "subtitles.consent.approve");
    wait(&mut h, subtitles_idle);
    h.get_by_label("Couldn't write chapters. Try again.");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn chapters_without_transcript_shows_the_empty_state_and_sends_nothing() {
    let calls = Arc::new(AtomicUsize::new(0));
    let empty = CueTrack {
        styles: Vec::new(),
        cues: Vec::new(),
    };
    let mut h = chapter_harness(chapter_text_opener(false, calls.clone()), Some(empty));
    click(&mut h, "subtitles.chapters.generate");
    h.get_by_label("No transcript");
    assert!(!panel(h.state()).busy);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

/// Whether any widget's accessible label contains `text`. A text label's words are its
/// AccessKit value, which `label_contains` reads.
fn has_label(h: &Harness<'_, Editor>, text: &str) -> bool {
    h.query_all(By::new().label_contains(text)).next().is_some()
}

#[test]
fn chapters_cancel_during_generation_stops_the_run_and_loads_nothing() {
    let calls = Arc::new(AtomicUsize::new(0));
    let (entered_send, entered_recv) = mpsc::sync_channel(1);
    let (release_send, release_recv) = mpsc::channel();
    let gate = Arc::new(Gate {
        entered: Mutex::new(entered_send),
        release: Mutex::new(release_recv),
    });
    let mut h = chapter_harness(text_opener(false, calls.clone(), Some(gate)), None);
    click(&mut h, "subtitles.chapters.generate");
    wait(&mut h, approval_sheet_open);
    click(&mut h, "subtitles.consent.approve");
    // The provider call is held inside the adapter: the run is in flight, and so is Cancel.
    entered_recv.recv_timeout(Duration::from_secs(20)).unwrap();
    h.run_steps(3);
    assert!(panel(h.state()).busy);
    h.get_by_label("Writing chapters…");
    click(&mut h, "subtitles.chapters.cancel");
    release_send.send(()).unwrap();
    wait(&mut h, subtitles_idle);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(
        h.query_all(By::new().predicate(|n| n.author_id() == Some("subtitles.chapters.title")))
            .next()
            .is_none(),
        "a cancelled run loads no chapters"
    );
    assert!(!has_label(&h, "Couldn't write chapters"));
}

#[test]
fn chapters_unsupported_language_is_refused_before_any_call() {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut h = chapter_harness(
        chapter_text_opener(false, calls.clone()),
        Some(fixture::cues("ja")),
    );
    click(&mut h, "subtitles.chapters.generate");
    h.get_by_label("Not available for this language");
    assert!(!panel(h.state()).busy);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[test]
fn chapters_export_failure_is_typed_and_shows_no_success_line() {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut h = chapter_harness(chapter_text_opener(false, calls), None);
    generate_approved(&mut h);
    let root = scratch_dir("chapters-failed");
    let missing = root.join("not-created");
    h.state_mut()
        .subtitles_mut()
        .unwrap()
        .export_chapters_to(missing.clone());
    h.run_steps(3);
    h.get_by_label("Couldn't save chapters. Check the folder and try again.");
    assert!(
        !has_label(&h, "Saved "),
        "a failed export shows no success line"
    );
    assert!(!missing.exists(), "nothing is created for a refused folder");
    std::fs::remove_dir_all(&root).unwrap();
}

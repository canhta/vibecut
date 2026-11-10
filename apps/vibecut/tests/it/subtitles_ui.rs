#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
//! Desktop behaviour through the real panel, shared engine jobs and offline adapters.
use captions::CueTrack;
use egui_kittest::{
    Harness,
    kittest::{By, NodeT, Queryable},
};
use engine::{ByteBudget, Command, Session, providers::Cost, transcript::Cancellation};
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
    Editor, egui,
    i18n::Locale,
    panels::AssetsTab,
    preferences::PreferenceStore,
    subtitles::{LocalWorkflow, Options, Subtitles, TaskEvents, Work, Workflow},
};
#[path = "support/subtitles.rs"]
mod fixture;
use fixture::{editor, lang};
fn harness(editor: Editor, scale: f32) -> Harness<'static, Editor> {
    crate::gpu::builder()
        .with_size(egui::vec2(1440.0, 1000.0))
        .with_pixels_per_point(scale)
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
fn panel(e: &Editor) -> subtitles_inspect::Inspection {
    e.inspect().unwrap().subtitles.unwrap()
}
use vibecut::subtitles as subtitles_inspect;
fn with_panel<T>(e: &mut Editor, f: impl FnOnce(&mut Subtitles, &mut Session, &Options) -> T) -> T {
    f(
        e.subtitles.as_mut().unwrap(),
        &mut e.session,
        &e.preferences.subtitles,
    )
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
    calls: Arc<AtomicUsize>,
    /// Holds the provider call until the test opens the gate, so a job stays
    /// running while the busy block is on screen.
    gate: Option<Gate>,
}
impl Workflow for TranslateWorkflow {
    fn run(
        &self,
        work: Work,
        budget: ByteBudget,
        events: &TaskEvents,
        cancel: &Cancellation,
    ) -> engine::Result<Command> {
        let Work::Translate {
            project,
            item,
            ids,
            options,
            glossary,
            force,
        } = work
        else {
            return LocalWorkflow.run(work, budget, events, cancel);
        };
        let config = ai::ProviderConfig::new(
            options.translation.clone().into(),
            options.translation_model.clone().into(),
            "fixture-v1".into(),
            ai::Location::Remote {
                backend: "fixture".into(),
                base_url: "https://provider.example".into(),
                auth_ref: "fixture".into(),
            },
        )?;
        let adapter = engine::transcript::SubtitleTranslator::from_parts(
            Arc::new(Translator {
                config,
                calls: self.calls.clone(),
                gate: self.gate.clone(),
            }),
            "context-window".into(),
        );
        let mut session = Session::new((*project).clone(), 1)?;
        let mut runner = engine::jobs::JobRunner::new(
            engine::jobs::RunnerConfig {
                concurrency: [1, 1, 1],
                queue: 4,
                retained: 8,
                network_attempts: 1,
                backoff: Duration::from_millis(10),
                progress_interval: Duration::from_millis(10),
            },
            budget,
            Arc::new(engine::jobs::SystemClock::default()),
        )?;
        engine::transcript::translate_selected(
            &mut session,
            &adapter,
            &mut runner,
            events,
            events,
            cancel,
            &engine::transcript::TranslationSelection {
                item,
                ids,
                target: lang(&options.target),
                glossary,
                force,
            },
            &mut Cost::Free,
        )?;
        let content = session
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
        Ok(Command::SetItemParam { item, content })
    }
}
struct Translator {
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
        ai::CostEstimate::UpperBound(ai::Money::new("USD", 1_000).unwrap())
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
    e.subtitles = Some(
        Subtitles::with_workflow(
            e.playback.budget().clone(),
            Arc::new(TranslateWorkflow { calls, gate: None }),
        )
        .unwrap(),
    );
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
    e.subtitles = Some(
        Subtitles::with_workflow(
            e.playback.budget().clone(),
            Arc::new(TranslateWorkflow {
                calls,
                gate: Some(gate.clone()),
            }),
        )
        .unwrap(),
    );
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
            // An all-protected selection refuses without an upload or provider invocation.
            with_panel(h.state_mut(), |p, s, o| p.translate(s, o).unwrap());
            wait(&mut h, |e| !panel(e).busy);
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
    for action in ["deny", "change", "cancel", "stale"] {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut h = harness(translated_editor("vi", calls.clone()), 1.0);
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
            "change" => h.state_mut().preferences.subtitles.translation = "anthropic".into(),
            "cancel" => h.state_mut().subtitles.as_mut().unwrap().cancel(),
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
        assert_eq!(calls.load(Ordering::SeqCst), usize::from(action == "stale"));
    }
}
#[test]
fn capabilities_locale_and_choices_persist_without_loading_models() {
    let mut h = harness(editor("vi"), 2.0);
    h.run_steps(3);
    assert_eq!(Options::default().source, "auto");
    click(&mut h, "subtitles.generate.settings");
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
        store.load().unwrap().subtitles,
        h.state().preferences.subtitles
    );
    assert_eq!(
        store.load().unwrap().subtitles.source_for(project),
        "en",
        "the spoken-language choice persists per project"
    );
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
struct GenerateWorkflow {
    missing: bool,
    failure: Option<GenerateFailure>,
}
impl Workflow for GenerateWorkflow {
    fn run(
        &self,
        work: Work,
        budget: ByteBudget,
        events: &TaskEvents,
        cancel: &Cancellation,
    ) -> engine::Result<Command> {
        let Work::Generate {
            asset,
            range,
            start,
            options,
        } = work
        else {
            return LocalWorkflow.run(work, budget, events, cancel);
        };
        if self.missing {
            return Err(engine::providers::Error::ModelMissing("whisper-large-v3-turbo").into());
        }
        match self.failure {
            Some(GenerateFailure::NoSpeech) if options.source != "auto" => {
                return Err(engine::transcript::Error::NoSpeech.into());
            }
            Some(GenerateFailure::LowConfidence) if options.source != "auto" => {
                return Err(engine::transcript::Error::LowConfidence.into());
            }
            Some(GenerateFailure::Silent) => {
                return Err(engine::transcript::Error::NoSpeech.into());
            }
            _ => {}
        }
        engine::transcript::Progress::stage(events, engine::transcript::Stage::Transcribe);
        if ["openai", "elevenlabs"].contains(&options.transcription.as_str()) {
            engine::providers::ConsentPrompt::approve(
                events,
                &engine::providers::UploadRequest {
                    operation: engine::providers::CatalogOperation::Transcribe,
                    provider: options.transcription.clone(),
                    model: options.transcription_model,
                    languages: options.source.clone(),
                    audio_seconds: 10.0,
                    upload_bytes: 320000,
                    cost: Cost::UpperBound {
                        currency: "USD".into(),
                        micros: 1000,
                    },
                    terms: vec![("policy", "https://provider.example/policy".into())],
                },
            )?;
        }
        if cancel.cancelled() {
            return Err(engine::EditError::Cancelled);
        }
        Ok(Command::InsertItem {
            placement: engine::Placement::Auto,
            item: engine::NewItem {
                retime: Default::default(),
                animations: Default::default(),
                source_asset: Some(asset.id()),
                source_range: range,
                start,
                content: ItemParams::Caption(project::CaptionParams {
                    // Auto-detect resolves to a concrete spoken language; the
                    // fixture uses vi so the inserted cues stay realistic.
                    cues: fixture::cues(if options.source == "auto" {
                        "vi"
                    } else {
                        &options.source
                    }),
                }),
            },
        })
    }
}
fn source_clip(e: &mut Editor) -> project::ItemId {
    let duration = Tick(10 * TICKS_PER_SECOND);
    e.session
        .apply(Command::AddAsset {
            asset: testkit::source::asset(2, std::env::temp_dir().join("subtitle-synthetic.wav"))
                .format("wav")
                .duration(duration)
                .content_hash(media::ContentHash::from_bytes(b"speech fixture"))
                .stream(
                    testkit::source::audio()
                        .codec("pcm_s16le")
                        .time_base(media::TimeBase::new(1, 48000).unwrap())
                        .duration(duration)
                        .start(Tick::ZERO)
                        .channels(1)
                        .build(),
                )
                .build(),
        })
        .unwrap();
    e.session
        .apply(Command::InsertItem {
            placement: engine::Placement::Auto,
            item: engine::NewItem {
                retime: Default::default(),
                animations: Default::default(),
                source_asset: Some(media::AssetId(2)),
                source_range: time::SourceRange::new(Tick::ZERO, duration).unwrap(),
                start: Tick::ZERO,
                content: ItemParams::Audio(project::AudioParams {
                    fades: Default::default(),
                    volume: 0.0,
                    muted: false,
                    source: Default::default(),
                }),
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
        e.selection.insert(source);
        e.preferences.subtitles.transcription = option.id.into();
        e.preferences.subtitles.transcription_model = option.models[0].clone();
        e.subtitles = Some(
            Subtitles::with_workflow(
                e.playback.budget().clone(),
                Arc::new(GenerateWorkflow {
                    missing: false,
                    failure: None,
                }),
            )
            .unwrap(),
        );
        let mut h = harness(e, 1.0);
        h.run_steps(3);
        click(&mut h, "subtitles.generate.settings");
        let before = h.state().session.project().clone();
        click(&mut h, "subtitles.generate");
        if !option.local {
            wait(&mut h, |e| panel(e).consent.is_some());
            assert_eq!(panel(h.state()).consent.unwrap().provider, option.id);
            click(&mut h, "subtitles.consent.approve");
        }
        wait(&mut h, |e| !panel(e).busy);
        assert_eq!(panel(h.state()).completed, Some(3));
        assert!(
            h.state()
                .inspect()
                .unwrap()
                .import_toast
                .unwrap()
                .contains('3')
        );
        h.state_mut().activate("edit.undo").unwrap();
        h.run_steps(3);
        assert_eq!(h.state().session.project(), &before);
    }
    let mut e = editor("vi");
    let source = source_clip(&mut e);
    e.selection.insert(source);
    e.subtitles = Some(
        Subtitles::with_workflow(
            e.playback.budget().clone(),
            Arc::new(GenerateWorkflow {
                missing: true,
                failure: None,
            }),
        )
        .unwrap(),
    );
    let mut h = harness(e, 1.0);
    h.run_steps(3);
    click(&mut h, "subtitles.generate.settings");
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
    for (failure, source, recovers) in [
        (GenerateFailure::NoSpeech, "vi", true),
        (GenerateFailure::LowConfidence, "en", true),
        (GenerateFailure::Silent, "vi", false),
        (GenerateFailure::Silent, "auto", false),
    ] {
        let mut e = editor("vi");
        let clip = source_clip(&mut e);
        e.selection.insert(clip);
        e.preferences.subtitles.source = source.into();
        e.subtitles = Some(
            Subtitles::with_workflow(
                e.playback.budget().clone(),
                Arc::new(GenerateWorkflow {
                    missing: false,
                    failure: Some(failure),
                }),
            )
            .unwrap(),
        );
        let mut h = harness(e, 1.0);
        h.run_steps(3);
        click(&mut h, "subtitles.generate.settings");
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
            if recovers {
                assert_eq!(panel(h.state()).completed, Some(3), "{source}");
                assert_eq!(panel(h.state()).problem, None, "{source}");
            } else {
                assert_eq!(
                    panel(h.state()).problem,
                    Some(vibecut::i18n::Text::SubNoSpeech)
                );
            }
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
            content: ItemParams::Caption(project::CaptionParams { cues }),
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
    cue.word_ranges = vec![first, second];
    let item = panel(h.state()).item.unwrap();
    h.state_mut()
        .session
        .apply(Command::SetItemParam {
            item,
            content: ItemParams::Caption(project::CaptionParams { cues }),
        })
        .unwrap();
    h.run_steps(2);
    let at = with_panel(h.state_mut(), |p, s, _| {
        p.split_text(s, &"cue-0".into(), lang("vi"), "one two", 3)
            .unwrap()
    });
    assert_eq!(at, first.end());
    let after = track(h.state());
    assert_eq!(after.cues[0].word_ranges, vec![first]);
    assert_eq!(after.cues[1].word_ranges, vec![second]);
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
            content: ItemParams::Caption(project::CaptionParams { cues: updated }),
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

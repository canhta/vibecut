#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
//! Desktop behaviour through the real panel, shared engine jobs and offline adapters.
use captions::CueTrack;
use egui_kittest::{
    Harness,
    kittest::{By, Queryable},
};
use engine::{
    ByteBudget, Command, Session,
    subtitles::{self, Cancellation, Cost},
};
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
    Editor,
    catalog::{Locale, PreferenceStore},
    egui,
    panels::AssetsTab,
    subtitles::{LocalWorkflow, Options, Subtitles, TaskEvents, Work, Workflow},
};
#[path = "support/subtitles.rs"]
mod fixture;
use fixture::{editor, lang};
fn harness(editor: Editor, scale: f32) -> Harness<'static, Editor> {
    Harness::builder()
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
            .execute(
                "project.rename",
                serde_json::json!({"name":"Concurrent project rename"}),
            )
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
        let adapter = subtitles::SubtitleTranslator::from_parts(
            Arc::new(Translator {
                config,
                calls: self.calls.clone(),
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
        subtitles::translate_selected(
            &mut session,
            &adapter,
            &mut runner,
            events,
            events,
            cancel,
            &subtitles::TranslationSelection {
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
            Arc::new(TranslateWorkflow { calls }),
        )
        .unwrap(),
    );
    e
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
                    .execute("project.rename", serde_json::json!({"name":"Changed"}))
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
    click(&mut h, "subtitles.generate.settings");
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
    let dir = tempfile::tempdir().unwrap();
    let store = PreferenceStore::new(dir.path().join("prefs.json"));
    store.save(&h.state().preferences).unwrap();
    assert_eq!(
        store.load().unwrap().subtitles,
        h.state().preferences.subtitles
    );
    h.state_mut().preferences.locale = Locale::Vi;
    h.run_steps(3);
    assert_eq!(h.state().preferences.subtitles.transcription, "whisper.cpp");
    assert!(h.state().models().is_none());
}

struct GenerateWorkflow {
    missing: bool,
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
            return Err(engine::EditError::ModelMissing("whisper-large-v3-turbo"));
        }
        engine::subtitles::Progress::stage(events, subtitles::Stage::Transcribe);
        if ["openai", "elevenlabs"].contains(&options.transcription.as_str()) {
            engine::subtitles::ConsentPrompt::approve(
                events,
                &subtitles::UploadRequest {
                    operation: subtitles::CatalogOperation::Transcribe,
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
                animations: Default::default(),
                source_asset: Some(asset.id()),
                source_range: range,
                start,
                content: ItemParams::Caption(project::CaptionParams {
                    cues: fixture::cues(&options.source),
                }),
            },
        })
    }
}
fn source_clip(e: &mut Editor) -> project::ItemId {
    let duration = Tick(10 * TICKS_PER_SECOND);
    e.session
        .apply(Command::AddAsset {
            asset: media::SourceAsset::try_from(media::AssetData {
                ephemeral: false,
                id: media::AssetId(2),
                path: std::env::temp_dir().join("subtitle-synthetic.wav"),
                content_hash: media::ContentHash::from_bytes(b"speech fixture"),
                probe: media::SourceProbe {
                    format: "wav".into(),
                    duration: Some(duration),
                    streams: vec![media::SourceStream {
                        index: 0,
                        kind: media::StreamKind::Audio,
                        codec: "pcm_s16le".into(),
                        time_base: media::TimeBase::new(1, 48000).unwrap(),
                        duration: Some(duration),
                        start: Some(Tick::ZERO),
                        frame_rate: None,
                        frame_count: None,
                        variable_frame_rate: None,
                        video: None,
                        audio: Some(media::AudioInfo {
                            sample_rate: 48000,
                            channels: 1,
                            initial_padding: 0,
                            trailing_padding: 0,
                        }),
                    }],
                },
            })
            .unwrap(),
        })
        .unwrap();
    e.session
        .apply(Command::InsertItem {
            placement: engine::Placement::Auto,
            item: engine::NewItem {
                animations: Default::default(),
                source_asset: Some(media::AssetId(2)),
                source_range: time::SourceRange::new(Tick::ZERO, duration).unwrap(),
                start: Tick::ZERO,
                content: ItemParams::Audio(project::AudioParams {
                    gain_db: 0.0,
                    muted: false,
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
    for option in
        subtitles::provider_options_for(subtitles::CatalogOperation::Transcribe, Some(&lang("vi")))
    {
        let mut e = editor("vi");
        let source = source_clip(&mut e);
        e.selection.insert(source);
        e.preferences.subtitles.transcription = option.id.into();
        e.preferences.subtitles.transcription_model = option.models[0].clone();
        e.subtitles = Some(
            Subtitles::with_workflow(
                e.playback.budget().clone(),
                Arc::new(GenerateWorkflow { missing: false }),
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
            Arc::new(GenerateWorkflow { missing: true }),
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
        Some(vibecut::catalog::Text::ShortsModelMissing)
    );
    assert!(h.state().models().is_none());
    assert!(
        h.query_all(By::new().predicate(|n| n.author_id() == Some("subtitles.models")))
            .count()
            == 1
    );
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
        Some(vibecut::catalog::Text::SubDraftConflict)
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

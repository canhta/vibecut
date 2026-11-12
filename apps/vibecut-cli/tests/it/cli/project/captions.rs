//! Verifies native captions command behaviour.
use crate::cli::fixtures::new_project;
use crate::cli::process::{cli, code, error_json, os, stdout_json};
use std::fs;
use time::Tick;

#[test]
fn captions_import_reads_ass_and_prints_counts_and_style() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("ass.vibecut");
    new_project(&project);
    let ass = dir.path().join("styled.ass");
    fs::write(
        &ass,
        "\
[Script Info]\nPlayResX: 1920\nPlayResY: 1080\n\
[V4+ Styles]\n\
Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding\n\
Style: Default,Arial,60,&H00FFFFFF,&H000000FF,&H00000000,&H00000000,-1,0,0,0,100,100,0,0,1,0,0,2,50,50,40,1\n\
[Events]\n\
Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n\
Dialogue: 0,0:00:00.00,0:00:01.50,Default,,0,0,0,,Hello\n\
Dialogue: 0,broken,0:00:02.00,Default,,0,0,0,,Skipped\n",
    )
    .unwrap();
    let output = cli(&[
        os("captions"),
        os("import"),
        project.as_os_str(),
        ass.as_os_str(),
        os("--language"),
        os("vi"),
        os("--start"),
        os("100"),
        os("--json"),
    ]);
    assert_eq!(
        code(&output),
        0,
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value = stdout_json(&output);
    assert_eq!(value["cues"], 1);
    assert_eq!(value["styles"], 1);
    assert_eq!(value["skipped_cues"], 1);
    assert_eq!(value["warnings"][0]["kind"], "malformed_cue");

    let loaded = project::load(&project).unwrap().project;
    let track = loaded
        .data()
        .scene()
        .timeline
        .iter()
        .find(|track| track.kind == project::TrackKind::Text)
        .unwrap();
    let item = track.items.first().unwrap();
    assert_eq!(item.start, Tick(100));
    let project::ItemParams::Caption(params) = &item.content else {
        panic!("expected caption item")
    };
    let cue = params.cues.cues.first().unwrap();
    assert_eq!(cue.texts.values().next().unwrap().text.as_str(), "Hello");
    let style = params.cues.style_of(cue).unwrap();
    assert_eq!(style.alignment.numpad, 2);
    assert!(style.bold);
    assert_eq!(params.cues.styles.len(), 1);
    assert!(!params.word_highlight, "imports never opt in");
    let enabled = cli(&[
        os("exec"),
        project.as_os_str(),
        os("captions.set_word_highlight"),
        os(&serde_json::json!({"item":item.id,"enabled":true}).to_string()),
        os("--json"),
    ]);
    assert_eq!(code(&enabled), 0, "{:?}", enabled);
    let saved = project::load(&project).unwrap().project;
    let saved_item = saved
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| &track.items)
        .find(|value| value.id == item.id)
        .unwrap();
    assert!(
        matches!(&saved_item.content,project::ItemParams::Caption(params) if params.word_highlight)
    );
    // The saved command feeds the real CLI exporter; no preset-only export parser.
    for (preset, border) in [("outline", 1), ("box", 3)] {
        let args = serde_json::json!({"item":item.id,"style":{"kind":"preset","preset":preset}})
            .to_string();
        if preset == "box" {
            let mut control = super::control::ControlSession::start(&project, &[]);
            let initial = control
                .send(serde_json::json!({"request_id":"inspect","command":"inspect","params":{}}));
            let applied = control.send(serde_json::json!({"request_id":"style","command":"captions.apply_style","params":serde_json::from_str::<serde_json::Value>(&args).unwrap(),"base_rev":initial["rev_after"],"idempotency_key":"style"}));
            assert_eq!(
                applied["value"]["value"]["outcome"]["changed"], true,
                "{applied}"
            );
            assert_eq!(control.finish().0, 0);
        } else {
            let applied = cli(&[
                os("exec"),
                project.as_os_str(),
                os("captions.apply_style"),
                os(&args),
                os("--json"),
            ]);
            assert_eq!(code(&applied), 0, "{:?}", applied);
        }
        let named = format!("{preset} saved");
        let args = serde_json::json!({"item":item.id,"name":named}).to_string();
        let saved_preset = cli(&[
            os("exec"),
            project.as_os_str(),
            os("captions.save_preset"),
            os(&args),
        ]);
        assert_eq!(code(&saved_preset), 0, "{:?}", saved_preset);
        let bank = project::load(&project)
            .unwrap()
            .project
            .data()
            .caption_presets
            .clone();
        let id = bank.last().unwrap().id;
        assert!(
            String::from_utf8(saved_preset.stdout)
                .unwrap()
                .contains(&format!("created caption preset {}", id.0))
        );
        for (command, args) in [
            (
                "captions.rename_preset",
                serde_json::json!({"preset":id,"name":format!("{preset} renamed")}),
            ),
            (
                "captions.apply_style",
                serde_json::json!({"item":item.id,"style":{"kind":"preset","preset":if preset=="box" {"outline"}else{"box"}}}),
            ),
            (
                "captions.apply_style",
                serde_json::json!({"item":item.id,"style":{"kind":"saved","preset":id}}),
            ),
        ] {
            let args = args.to_string();
            let out = cli(&[
                os("exec"),
                project.as_os_str(),
                os(command),
                os(&args),
                os("--json"),
            ]);
            assert_eq!(code(&out), 0, "{:?}", out);
        }
        let bytes = fs::read(&project).unwrap();
        let mut control = super::control::ControlSession::start(&project, &[os("--no-save")]);
        let initial = control
            .send(serde_json::json!({"request_id":"bank-inspect","command":"inspect","params":{}}));
        let request = serde_json::json!({"request_id":"bank-save","command":"captions.save_preset","params":{"item":item.id,"name":"Control copy"},"base_rev":initial["rev_after"],"idempotency_key":"bank-save"});
        let accepted = control.send(request.clone());
        assert_eq!(accepted["exit_code"], 0, "{accepted}");
        assert_eq!(control.send(request.clone()), accepted);
        let mut stale = request;
        stale["idempotency_key"] = serde_json::json!("bank-stale");
        assert_eq!(control.send(stale)["exit_code"], 3);
        let undone=control.send(serde_json::json!({"request_id":"bank-undo","command":"undo","params":{},"base_rev":accepted["rev_after"],"idempotency_key":"bank-undo"}));
        assert_eq!(undone["exit_code"], 0, "{undone}");
        let before = control
            .send(serde_json::json!({"request_id":"before-frame","command":"inspect","params":{}}));
        let refused = control.send(serde_json::json!({"payload":"x".repeat(16<<20)}));
        assert_eq!(refused["error"]["code"], "limit");
        let after = control
            .send(serde_json::json!({"request_id":"after-frame","command":"inspect","params":{}}));
        assert_eq!(
            before["value"], after["value"],
            "oversize drain preserves project and bank"
        );
        if preset == "box" && crate::media_fixture::tools() {
            let protected = dir.path().join("protected.mp4");
            fs::write(&protected, b"existing destination").unwrap();
            let settings = render::OutputOptions::default();
            let refused=control.send(serde_json::json!({"request_id":"protected","command":"export","params":{"settings":settings,"path":protected,"overwrite":false},"base_rev":after["rev_after"],"idempotency_key":"protected"}));
            assert_eq!(
                refused["exit_code"], 0,
                "export accepts a job before publication admission"
            );
            let refused_job = refused["value"]["value"]["job"].clone();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
            for n in 0.. {
                let status=control.send(serde_json::json!({"request_id":format!("refusal-status-{n}"),"command":"job_status","params":{"job":refused_job}}));
                let state = status["value"]["value"]["snapshot"]["state"]
                    .as_str()
                    .unwrap();
                if state == "failed" {
                    break;
                }
                assert!(std::time::Instant::now() < deadline, "{status}");
            }
            assert_eq!(fs::read(&protected).unwrap(), b"existing destination");
            cancel_bank_export(
                project::load(&project).unwrap().project,
                dir.path().join("cancel.mp4"),
            );
            assert_eq!(fs::read(&project).unwrap(), bytes);
        }
        assert_eq!(control.finish().0, 0);
        assert_eq!(fs::read(&project).unwrap(), bytes);
        let selected = project::load(&project).unwrap().project;
        let found = selected
            .data()
            .scene()
            .timeline
            .iter()
            .flat_map(|t| &t.items)
            .find(|i| i.id == item.id)
            .unwrap();
        let project::ItemParams::Caption(p) = &found.content else {
            panic!("caption")
        };
        assert_eq!(p.preset, Some(id));
        // Materialize the built-in look again so the selected bank entry can be deleted.
        let args = serde_json::json!({"item":item.id,"style":{"kind":"preset","preset":preset}})
            .to_string();
        assert_eq!(
            code(&cli(&[
                os("exec"),
                project.as_os_str(),
                os("captions.apply_style"),
                os(&args),
                os("--json")
            ])),
            0
        );
        let copied = project::load(&project).unwrap().project;
        let args = serde_json::json!({"preset":id}).to_string();
        assert_eq!(
            code(&cli(&[
                os("exec"),
                project.as_os_str(),
                os("captions.delete_preset"),
                os(&args),
                os("--json")
            ])),
            0
        );
        assert_eq!(
            project::load(&project).unwrap().project.data().scenes,
            copied.data().scenes
        );
        let saved = project::load(&project).unwrap().project;
        let found = saved
            .data()
            .scene()
            .timeline
            .iter()
            .flat_map(|t| &t.items)
            .find(|i| i.id == item.id)
            .unwrap();
        let project::ItemParams::Caption(p) = &found.content else {
            panic!("caption")
        };
        assert_eq!(p.cues.styles[0].border_style, border);
        assert_eq!(p.cues.styles[0].font_family.as_str(), "Inter");
        assert!(
            p.cues
                .cues
                .iter()
                .all(|c| c.style.is_some_and(|id| id.0 == 0))
        );
        if crate::media_fixture::tools() {
            let output = dir.path().join(format!("{preset}.mp4"));
            let settings = render::OutputOptions {
                resolution: Some([360, 640]),
                ..Default::default()
            };
            let settings = serde_json::to_string(&settings).unwrap();
            let exported = cli(&[
                os("export"),
                project.as_os_str(),
                os("--out"),
                output.as_os_str(),
                os("--settings"),
                os(&settings),
                os("--json"),
            ]);
            assert_eq!(code(&exported), 0, "{:?}", exported);
            let diagnostics = stdout_json(&exported);
            assert_eq!(
                diagnostics["word_highlight_warnings"]["warnings"][0]["reason"],
                "missing_timing"
            );
            assert!(
                !diagnostics["word_highlight_warnings"]
                    .to_string()
                    .contains("Hello")
            );
            let frames = crate::cli::fixtures::decoded_rgb(&output, 360, 640, &[0, 1]);
            assert!(
                frames[0].iter().all(|value| *value < 10),
                "frame zero precedes the positive caption start"
            );
            assert!(
                frames[1].iter().any(|value| *value > 100),
                "captions survive encoding"
            );
            let mut settings = render::RenderSettings::from_project(&saved);
            settings.width = 360;
            settings.height = 640;
            let budget = engine::ByteBudget::new(128 << 20).unwrap();
            let hooks = engine::ByteBudgetHooks::new(&budget, None);
            let expected = render::Renderer::new(&saved, settings)
                .unwrap()
                .render_frame(
                    time::FrameRate::FPS_30.tick_of(1).unwrap(),
                    &mut render::FileSources::default(),
                    &hooks,
                )
                .unwrap()
                .video_rgba8(1, &hooks)
                .unwrap();
            let mse = frames[1]
                .iter()
                .zip(
                    expected
                        .bytes
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .flat_map(|p| p[..3].iter()),
                )
                .map(|(a, b)| (f64::from(*a) - f64::from(*b)).powi(2))
                .sum::<f64>()
                / frames[1].len() as f64;
            assert!(mse < 150.0, "caption-bank CLI encoded MSE {mse}");
        }
    }
}

fn cancel_bank_export(project: project::Project, destination: std::path::PathBuf) {
    use engine::cache::{ArtifactKind, CacheKey, KeyInputs};
    use engine::control::{Caller, ControlResult, Policy};
    use engine::jobs::{JobRunner, JobState, RunnerConfig, SystemClock};
    use std::sync::{Arc, Mutex, mpsc};
    use std::time::{Duration, Instant};

    struct CancelOnly;
    impl Policy for CancelOnly {
        fn authorize(
            &self,
            command: &engine::Command,
            _: &project::Project,
        ) -> engine::control::Result<()> {
            if matches!(command, engine::Command::JobCancel { .. }) {
                Ok(())
            } else {
                Err(engine::control::Error::Control(
                    engine::control::ControlError::Permission,
                ))
            }
        }
    }

    let saved = project.data().caption_presets.last().unwrap();
    assert_eq!(saved.name.as_str(), "box renamed");
    // The box look saves as a presentation value with an opaque background.
    assert!(saved.style.background.is_some());
    let caption = project
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| &track.items)
        .find_map(|item| match &item.content {
            project::ItemParams::Caption(params) => Some(params),
            _ => None,
        })
        .unwrap();
    assert!(!caption.cues.cues.is_empty());
    // The export renders the saved look through the live selection, not copied cues.
    assert_eq!(caption.preset, Some(saved.id));
    let mut session = engine::Session::new(project, 100).unwrap();
    let snapshot = session.project().clone();
    let revision = session.revision();
    let mut runner = JobRunner::new(
        RunnerConfig {
            concurrency: [1; 3],
            queue: 2,
            retained: 2,
            network_attempts: 1,
            backoff: Duration::from_millis(100),
            progress_interval: Duration::from_millis(10),
        },
        engine::ByteBudget::new(128 << 20).unwrap(),
        Arc::new(SystemClock::default()),
    )
    .unwrap();
    let key = CacheKey::new(KeyInputs {
        kind: ArtifactKind::Preview,
        source: [0; 32],
        provider: "render",
        model: "caption-bank",
        version: "1",
        settings: b"{}",
        transcript: [0; 32],
        cues: [0; 32],
        clip_range: [0; 32],
        translation: [0; 32],
    })
    .unwrap();
    let (entered, waiting) = mpsc::sync_channel(1);
    let (release, held) = mpsc::sync_channel(1);
    let held = Mutex::new(Some(held));
    // The real export worker stops at its existing source-factory seam before
    // render can publish. This gate orders cancellation independently of speed.
    let (job, _) = engine::submit_export_with_sources(
        &mut runner,
        key,
        snapshot.clone(),
        time::TimeRange::new(Tick::ZERO, snapshot.duration()).unwrap(),
        render::ExportSettings::from_project(&snapshot),
        destination.clone(),
        move || {
            let held = held.lock().unwrap().take().unwrap();
            entered.send(()).unwrap();
            held.recv_timeout(Duration::from_secs(30)).unwrap();
            Box::new(render::FileSources::default())
        },
    )
    .unwrap();
    runner.tick().unwrap();
    waiting.recv_timeout(Duration::from_secs(30)).unwrap();
    assert_eq!(runner.snapshot(job).unwrap().state, JobState::Running);
    assert!(!destination.exists());
    let request = engine::control::build_request(
        &session,
        serde_json::json!({"request_id":"cancel-bank-export","command":"job_cancel","params":{"job":job},"base_rev":revision,"idempotency_key":"cancel-bank-export"}),
    )
    .unwrap();
    let cancelled = session.control_typed(
        request,
        Caller {
            principal: "vibecut-cli",
            external: true,
            policy: &CancelOnly,
            fonts: None,
            sounds: None,
            music: None,
            media: None,
            chapters: None,
            captions: None,
            vision: None,
        },
        &mut runner,
    );
    release.send(()).unwrap();
    assert!(matches!(cancelled.unwrap(), ControlResult::CancelRequested { job: id } if id == job));
    let deadline = Instant::now() + Duration::from_secs(30);
    let state = loop {
        runner.tick().unwrap();
        let state = runner.snapshot(job).unwrap().state;
        if matches!(
            state,
            JobState::Completed | JobState::Failed | JobState::Cancelled
        ) {
            break state;
        }
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    };
    assert_eq!(state, JobState::Cancelled);
    assert!(runner.take_output(job).is_none());
    assert!(!destination.exists());
    assert!(Arc::ptr_eq(session.project(), &snapshot));
    assert_eq!(session.revision(), revision);
    assert_eq!(session.history_len(), (0, 0));
    runner.shutdown();
}

#[test]
fn captions_import_rejects_non_ass_input() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("ass-extension.vibecut");
    new_project(&project);
    let srt = dir.path().join("plain.srt");
    fs::write(&srt, "1\n00:00:00,000 --> 00:00:01,000\nHello\n").unwrap();
    let output = cli(&[
        os("captions"),
        os("import"),
        project.as_os_str(),
        srt.as_os_str(),
        os("--json"),
    ]);
    assert_eq!(code(&output), 2);
    assert_eq!(error_json(&output)["error"]["code"], "usage");
}

#[test]
fn cli_native_word_opt_in_survives_save_and_paints_the_encoded_source_word() {
    if !crate::media_fixture::tools() {
        return;
    }
    use std::sync::{Arc, Mutex, atomic::AtomicUsize};
    use time::TICKS_PER_SECOND;
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("synthetic-word-source.wav");
    crate::cli::fixtures::generate_audio(&source);
    let asset = media::SourceAsset::import(media::AssetId(1), source).unwrap();
    let language = engine::providers::parse_language("en").unwrap();
    // Reuse the genuine offline word-provider seam, not an imported cue promoted by count.
    let config = ai::ProviderConfig::new(
        "offline-asr".into(),
        "word-fixture".into(),
        "cli-v1".into(),
        ai::Location::Remote {
            backend: "fixture".into(),
            base_url: "https://fixture.invalid".into(),
            auth_ref: "fixture".into(),
        },
    )
    .unwrap();
    let providers = engine::providers::Providers::from_parts(
        engine::providers::TranscriptionProvider::ElevenLabs {
            model: "word-fixture".into(),
        },
        "transcribe".into(),
        Arc::new(crate::mcp::CaptionAsr {
            config,
            calls: Arc::new(AtomicUsize::new(0)),
        }),
        None,
        None,
        None,
    )
    .unwrap();
    let (cache, _) = engine::cache::ArtifactCache::open(
        &dir.path().join("cache"),
        engine::ByteBudget::new(256 << 20).unwrap(),
        16,
    )
    .unwrap();
    let cancel = engine::transcript::Cancellation::new();
    let acquired = engine::transcript::transcribe(
        &asset,
        Tick(TICKS_PER_SECOND),
        &language,
        &providers,
        None,
        &Mutex::new(cache),
        &engine::providers::AllowAll,
        &engine::transcript::SilentProgress,
        &cancel,
        &cancel,
    )
    .unwrap();
    let session = engine::transcript::build_session(
        &asset,
        &acquired.cues,
        Tick(TICKS_PER_SECOND),
        false,
        None,
    )
    .unwrap();
    let item = session
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| &track.items)
        .find(|item| matches!(item.content, project::ItemParams::Caption(_)))
        .unwrap()
        .id;
    let path = dir.path().join("native.vibecut");
    fs::write(&path, project::encode(session.project(), true).unwrap()).unwrap();
    let settings = serde_json::to_string(&render::OutputOptions {
        resolution: Some([360, 640]),
        ..Default::default()
    })
    .unwrap();
    // Keyword emphasis through the same exec path: a subset of the verified words, saved.
    let cue_id = session
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| &track.items)
        .find_map(|value| match &value.content {
            project::ItemParams::Caption(params) => {
                params.cues.cues.first().map(|cue| cue.id.clone())
            }
            _ => None,
        })
        .unwrap();
    let selected = cli(&[
        os("exec"),
        path.as_os_str(),
        os("captions.select_emphasis"),
        os(&serde_json::json!({"item":item,"cue_ids":[cue_id]}).to_string()),
        os("--json"),
    ]);
    assert_eq!(code(&selected), 0, "{:?}", selected);
    let saved = project::load(&path).unwrap().project;
    let project::ItemParams::Caption(saved_params) = &saved
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| &track.items)
        .find(|value| value.id == item)
        .unwrap()
        .content
    else {
        panic!("caption item")
    };
    let words = serde_json::to_value(&saved_params.cues.cues[0].word_timing).unwrap()["records"]
        .as_array()
        .map_or(0, Vec::len);
    assert!(words > 0, "the fixture carries verified word timing");
    assert!(
        saved_params
            .emphasis
            .values()
            .flatten()
            .all(|&id| (id as usize) < words)
            && saved_params.emphasis.values().all(|ids| ids.len() <= 3),
        "emphasis stays a subset of the verified words"
    );
    for enabled in [false, true] {
        let params = serde_json::json!({"item":item,"enabled":enabled}).to_string();
        let command = cli(&[
            os("exec"),
            path.as_os_str(),
            os("captions.set_word_highlight"),
            os(&params),
            os("--json"),
        ]);
        assert_eq!(code(&command), 0, "{:?}", command);
        let movie = dir.path().join(if enabled {
            "active.mp4"
        } else {
            "ordinary.mp4"
        });
        let output = cli(&[
            os("export"),
            path.as_os_str(),
            os("--out"),
            movie.as_os_str(),
            os("--settings"),
            os(&settings),
            os("--json"),
        ]);
        assert_eq!(code(&output), 0, "{:?}", output);
        assert_eq!(
            stdout_json(&output)["word_highlight_warnings"]["warnings"],
            serde_json::json!([])
        );
        let decoded = crate::cli::fixtures::decoded_rgb(&movie, 360, 640, &[1]);
        let gold = decoded[0]
            .as_chunks::<3>()
            .0
            .iter()
            .filter(|pixel| pixel[0] > 150 && pixel[1] > 70 && pixel[2] < 80)
            .count();
        if enabled {
            assert!(
                gold > 5,
                "verified source word survives the real movie encoder: {gold}"
            );
        } else {
            assert_eq!(gold, 0, "ordinary white caption remains unchanged");
        }
    }
}

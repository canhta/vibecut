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
            let mut refused_state = String::new();
            for n in 0..200 {
                let status=control.send(serde_json::json!({"request_id":format!("refusal-status-{n}"),"command":"job_status","params":{"job":refused_job}}));
                refused_state = status["value"]["value"]["snapshot"]["state"]
                    .as_str()
                    .unwrap()
                    .into();
                if refused_state == "failed" {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            assert_eq!(refused_state, "failed");
            assert_eq!(fs::read(&protected).unwrap(), b"existing destination");
            let mut last = serde_json::Value::Null;
            let mut destination = dir.path().join("cancel-0.mp4");
            for n in 0..8 {
                destination = dir.path().join(format!("cancel-{n}.mp4"));
                let out=control.send(serde_json::json!({"request_id":format!("export-{n}"),"command":"export","params":{"settings":settings,"path":destination,"overwrite":false},"base_rev":after["rev_after"],"idempotency_key":format!("export-{n}")}));
                assert_eq!(out["exit_code"], 0, "{out}");
                last = out["value"]["value"]["job"].clone();
            }
            let cancel=control.send(serde_json::json!({"request_id":"cancel-bank-export","command":"job_cancel","params":{"job":last},"base_rev":after["rev_after"],"idempotency_key":"cancel-bank-export"}));
            assert_eq!(cancel["exit_code"], 0, "{cancel}");
            let mut state = String::new();
            for n in 0..200 {
                let status=control.send(serde_json::json!({"request_id":format!("cancel-status-{n}"),"command":"job_status","params":{"job":last}}));
                state = status["value"]["value"]["snapshot"]["state"]
                    .as_str()
                    .unwrap()
                    .into();
                if state == "cancelled" {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            assert_eq!(state, "cancelled");
            assert!(!destination.exists());
            assert_eq!(fs::read(&project).unwrap(), bytes);
        }
        assert_eq!(control.finish().0, 0);
        assert_eq!(fs::read(&project).unwrap(), bytes);
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
                .zip(expected.bytes.chunks_exact(4).flat_map(|p| p[..3].iter()))
                .map(|(a, b)| (f64::from(*a) - f64::from(*b)).powi(2))
                .sum::<f64>()
                / frames[1].len() as f64;
            assert!(mse < 150.0, "caption-bank CLI encoded MSE {mse}");
        }
    }
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

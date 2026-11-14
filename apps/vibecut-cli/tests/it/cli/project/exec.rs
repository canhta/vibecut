//! Verifies native exec command behaviour.
use crate::cli::fixtures::{add_track, exec, info, new_project, overlay_project_color};
use crate::cli::fixtures::{decoded_mono, decoded_rgb, generate_audio, video_oetf};
use crate::cli::process::{cli, code, error_json, os, stdout_json};
use crate::media_fixture::{generate_av, media_project, tool_command, tools};
use serde_json::{Value, json};
use std::ffi::OsStr;
use std::fs;
use time::{Tick, TimeRange};

#[test]
fn lut_cli_imports_applies_reopens_and_refuses_without_project_write() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("look.vibecut");
    new_project(&path);
    exec(
        &path,
        "project.set_canvas",
        json!({"width":64,"height":64,"preset":null}),
    );
    let track = add_track(&path, "Overlay")["created"][0]["track"]
        .as_u64()
        .unwrap();
    let inserted = exec(
        &path,
        "timeline.insert",
        json!({"placement":{"mode":"track","track":track},"item":{"source_range":{"start":0,"end":254016000000_i64},"start":0,"animations":{},"content":{"kind":"overlay","params":{"color":[50,100,150,255]}}}}),
    );
    let item = inserted["placements"][0]["item"]
        .as_u64()
        .unwrap()
        .to_string();
    let source = dir.path().join("look.cube");
    fs::write(
        &source,
        "LUT_3D_SIZE 2\n0 0 0\n0 0 1\n1 0 0\n1 0 1\n0 1 0\n0 1 1\n1 1 0\n1 1 1\n",
    )
    .unwrap();
    let command = || {
        let mut command = std::process::Command::new(crate::cli::process::bin());
        command.args([
            os("color"),
            os("lut"),
            path.as_os_str(),
            os("--import"),
            source.as_os_str(),
            os("--item"),
            os(&item),
            os("--strength"),
            os("50"),
            os("--json"),
        ]);
        #[cfg(windows)]
        command.env("LOCALAPPDATA", dir.path());
        #[cfg(target_os = "macos")]
        command.env("HOME", dir.path());
        #[cfg(not(any(windows, target_os = "macos")))]
        command.env("XDG_CACHE_HOME", dir.path());
        command
    };
    let output = command().output().unwrap();
    assert_eq!(
        code(&output),
        0,
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let loaded = project::load(&path).unwrap().project;
    assert_eq!(loaded.data().luts.len(), 1);
    let record = &loaded.data().luts[0];
    assert!(record.directory.join("original").is_file());
    fs::remove_file(&source).unwrap();
    let budget = engine::ByteBudget::new(64 << 20).unwrap();
    let hooks = engine::ByteBudgetHooks::new(&budget, None);
    let effect = loaded
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|t| &t.items)
        .find(|i| i.id.0.to_string() == item)
        .unwrap()
        .effects
        .iter()
        .find(|e| e.kind == "color_grade")
        .unwrap();
    assert_eq!(
        effect.value("lut_strength"),
        Some(&project::EffectValue::Scalar(50.))
    );
    assert!(render::lut::LutResources::resolve(&loaded, std::iter::once(effect), &hooks).is_ok());
    let before = fs::read(&path).unwrap();
    let exported = dir.path().join("look.mp4");
    let output = cli(&[
        os("export"),
        path.as_os_str(),
        os("--out"),
        exported.as_os_str(),
        os("--json"),
    ]);
    assert_eq!(
        code(&output),
        0,
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(fs::read(&path).unwrap(), before);
    if tools() {
        let decode = |v: f64| {
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        // This table permutes encoded (r,g,b) to (g,b,r); mix in linear light.
        let expected = [50., 100., 150.]
            .into_iter()
            .zip([100., 150., 50.])
            .map(|(dry, wet)| {
                (255. * video_oetf((decode(dry / 255.) + decode(wet / 255.)) / 2.)).round() as i32
            })
            .collect::<Vec<_>>();
        for frame in decoded_rgb(&exported, 64, 64, &[0, 15, 29]) {
            let center = &frame[(32 * 64 + 32) * 3..(32 * 64 + 32) * 3 + 3];
            for (&actual, &expected) in center.iter().zip(&expected) {
                assert!(
                    (i32::from(actual) - expected).abs() <= 12,
                    "decoded LUT {actual} != {expected}"
                );
            }
        }
    }
    fs::write(&source, b"LUT_3D_SIZE 10000000\n").unwrap();
    let output = command().output().unwrap();
    assert_eq!(
        code(&output),
        6,
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(fs::read(&path).unwrap(), before);
    let error = error_json(&output);
    assert!(!error.to_string().contains(source.to_str().unwrap()));
}

#[test]
fn exec_moves_and_trims_timeline_groups() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("group.vibecut");
    new_project(&path);
    let track = add_track(&path, "Overlay")["created"][0]["track"]
        .as_u64()
        .unwrap();
    let inserted = exec(
        &path,
        "timeline.insert",
        json!({
            "placement":{"mode":"track","track":track},
            "item":{"source_range":{"start":100,"end":200},"start":20,
                "animations":{},"content":{"kind":"overlay","params":{"color":[1,2,3,255]}}}
        }),
    );
    let id = inserted["placements"][0]["item"].as_u64().unwrap();
    exec(
        &path,
        "timeline.move_items",
        json!({"items":[id],"delta":10}),
    );
    exec(
        &path,
        "timeline.trim",
        json!({"items":[id],"edge":"end","delta":-5}),
    );
    let loaded = project::load(&path).unwrap().project;
    let item = loaded
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|t| &t.items)
        .find(|i| i.id.0 == id)
        .unwrap();
    assert_eq!(item.start, Tick(30));
    assert_eq!(item.duration().unwrap(), Tick(95));
    assert_eq!(item.source_range.end(), Tick(195));
}

#[test]
fn exec_toggles_and_updates_bookmarks_through_save_reload() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bookmarks.vibecut");
    new_project(&path);
    let created = exec(&path, "bookmark.toggle", json!({"at":0}));
    let bookmark = created["created"][0]["bookmark"].as_u64().unwrap();
    let loaded = project::load(&path).unwrap().project;
    let stored = &loaded.data().scene().bookmarks[0];
    assert_eq!(stored.start, Tick::ZERO);
    assert_eq!(stored.duration, Tick::ZERO);
    assert_eq!(stored.note.as_str(), "");
    assert_eq!(stored.color, project::Color([0, 157, 255, 255]));

    exec(
        &path,
        "bookmark.update",
        json!({"bookmark":bookmark,"note":"Intro","color":[9,8,7,255],"duration":0}),
    );
    let loaded = project::load(&path).unwrap().project;
    let stored = &loaded.data().scene().bookmarks[0];
    assert_eq!(stored.note.as_str(), "Intro");
    assert_eq!(stored.color, project::Color([9, 8, 7, 255]));
}

#[test]
fn exec_mutates_through_engine_and_reports_created_ids() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("exec.vibecut");
    new_project(&project);
    let outcome = add_track(&project, "Video");
    assert_eq!(outcome["changed"], true);
    assert_eq!(outcome["created"][0]["track"], 1);
    let value = info(&project);
    assert_eq!(value["tracks"], 2);
    assert_eq!(value["items"], 0);
    let placed = exec(
        &project,
        "timeline.insert",
        json!({
            "placement":{"mode":"auto"},
            "item":{"source_asset":null,"source_range":{"start":0,"end":100},"start":50,
                "animations":{},"content":{"kind":"overlay","params":{"color":[10,20,30,255]}}}
        }),
    );
    assert_eq!(placed["placements"][0]["track"], 0);
    assert_eq!(placed["placements"][0]["item"], 2);
    let loaded = project::load(&project).unwrap().project;
    assert_eq!(
        loaded.data().scene().timeline.main.items[0].start,
        Tick::ZERO
    );
}

#[test]
fn exec_refuses_bad_input_without_writing() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("bad.vibecut");
    new_project(&project);
    let before = fs::read(&project).unwrap();

    let unknown = cli(&[
        os("exec"),
        project.as_os_str(),
        os("no_such_command"),
        os("{}"),
        os("--json"),
    ]);
    assert_eq!(code(&unknown), 2);
    assert_eq!(error_json(&unknown)["error"]["code"], "usage");

    let invalid = cli(&[
        os("exec"),
        project.as_os_str(),
        os("timeline.add_track"),
        os("{not json"),
        os("--json"),
    ]);
    assert_eq!(code(&invalid), 2);
    assert_eq!(error_json(&invalid)["error"]["code"], "usage");

    let params = cli(&[
        os("exec"),
        project.as_os_str(),
        os("timeline.add_track"),
        os("{}"),
        os("--json"),
    ]);
    assert_eq!(code(&params), 2);
    assert_eq!(error_json(&params)["error"]["code"], "usage");
    assert_eq!(fs::read(&project).unwrap(), before);
}

#[test]
fn cli_animates_opacity_and_exported_frames_follow_the_curve() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("animated.vibecut");
    let output_path = dir.path().join("animated.mp4");
    // The animation covers one second; the extra 0.2 s makes the 1.0 s key
    // land exactly on an exported frame.
    let duration = Tick(time::TICKS_PER_SECOND * 6 / 5);
    overlay_project_color(&project, 64, 64, duration, [255, 255, 255, 255]);
    let key = |time: i64, value: f64| {
        json!({
            "time": time,
            "value": {"kind": "scalar", "value": value},
            "segment": "bezier",
            "tangent": "flat",
            "incoming": {"dt":-84672000000i64,"dv":0.0},
            "outgoing": {"dt":84672000000i64,"dv":0.0},
        })
    };
    exec(
        &project,
        "keyframes.set_param",
        json!({
            "item": 0,
            "path": "opacity",
            "param": {
                "kind": "value",
                "channels": {
                    "value": {"kind": "scalar", "value": 0.0},
                    "keys": [key(0, 0.0)],
                    "before": "hold",
                    "after": "hold",
                },
            },
        }),
    );
    exec(
        &project,
        "clipboard.paste_keyframes",
        json!({"item":0,"at":0,"source":{"kind":"payload","payload":{"version":1,"origin":0,"params":[{"path":"opacity","param":{"kind":"value","channels":{"value":{"kind":"scalar","value":0.0},"keys":[key(0,0.0),key(time::TICKS_PER_SECOND,1.0)],"before":"hold","after":"hold"}}}]}}}),
    );
    let saved = project::load(&project).unwrap().project;
    let curve = saved.data().scene().timeline.overlays[0].items[0]
        .animations
        .get(&project::BuiltinPath::Opacity.into())
        .unwrap();
    for (n, v) in [(0, 0.0), (9, 0.216), (15, 0.5), (21, 0.784), (30, 1.0)] {
        let project::ParamValue::Scalar(actual) = curve
            .value_at(time::FrameRate::FPS_30.tick_of(n).unwrap())
            .unwrap()
        else {
            panic!("opacity")
        };
        assert!((actual - v).abs() < 1e-8);
    }
    let output = cli(&[
        os("export"),
        project.as_os_str(),
        os("--out"),
        output_path.as_os_str(),
        os("--json"),
    ]);
    assert_eq!(
        code(&output),
        0,
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(stdout_json(&output)["status"], "completed");
    assert!(tools(), "required ffmpeg/ffprobe oracle setup");
    let proof = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/p1-20-clipboard-bezier.mp4");
    fs::create_dir_all(proof.parent().unwrap()).unwrap();
    fs::copy(&output_path, proof).unwrap();
    let frames = decoded_rgb(&output_path, 64, 64, &[0, 9, 15, 21, 30]);
    for (frame, opacity) in frames.iter().zip([0.0, 0.216, 0.5, 0.784, 1.0]) {
        let expected = (video_oetf(opacity) * 255.0).round() as i32;
        for channel in &frame[..3] {
            assert!(
                (i32::from(*channel) - expected).abs() <= 12,
                "decoded {channel} != expected {expected} for opacity {opacity}"
            );
        }
    }
}

#[test]
fn cli_animates_a_mask_and_exported_frames_follow_its_coverage() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("masked.vibecut");
    let output_path = dir.path().join("masked.mp4");
    let duration = Tick(time::TICKS_PER_SECOND * 6 / 5);
    overlay_project_color(&project, 64, 64, duration, [255, 255, 255, 255]);
    exec(
        &project,
        "masks.add",
        json!({"item":0,"mask":project::Mask::new("m", project::MaskShape::Rectangle)}),
    );
    let key = |time: i64, value: f64| {
        json!({
            "time": time,
            "value": {"kind": "scalar", "value": value},
            "segment": "linear",
            "tangent": "broken",
            "incoming": null,
            "outgoing": null,
        })
    };
    // The centre starts left of the canvas middle and reaches it at one second.
    exec(
        &project,
        "keyframes.set_param",
        json!({
            "item": 0,
            "path": "masks.m.params.center_x",
            "param": {
                "kind": "value",
                "channels": {
                    "value": {"kind": "scalar", "value": -0.4},
                    "keys": [key(0, -0.4), key(time::TICKS_PER_SECOND, 0.0)],
                    "before": "hold",
                    "after": "hold",
                },
            },
        }),
    );
    let output = cli(&[
        os("export"),
        project.as_os_str(),
        os("--out"),
        output_path.as_os_str(),
        os("--json"),
    ]);
    assert_eq!(
        code(&output),
        0,
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(stdout_json(&output)["status"], "completed");
    assert!(tools(), "required ffmpeg/ffprobe oracle setup");
    let frames = decoded_rgb(&output_path, 64, 64, &[0, 30]);
    // Red channel at (x, y) of a 64x64 rgb24 frame.
    let red = |frame: &[u8], x: usize, y: usize| frame[(y * 64 + x) * 3];
    // Frame 0 (centre x 6.4, half width 19.2) covers x 4 and not x 46; frame 30
    // (centre x 32) covers x 46 and not x 4.
    assert!(red(&frames[0], 4, 32) > 128 && red(&frames[0], 46, 32) < 128);
    assert!(red(&frames[1], 4, 32) < 128 && red(&frames[1], 46, 32) > 128);
}

#[test]
fn cli_clipboard_item_payload_pastes_with_relative_track_overwrite() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("clipboard-items.vibecut");
    overlay_project_color(
        &project,
        64,
        64,
        Tick(time::TICKS_PER_SECOND * 3),
        [255, 255, 255, 255],
    );
    let pasted = testkit::item::item(
        project::ItemId(0),
        project::ItemParams::Overlay(project::OverlayParams {
            color: project::Color([0, 0, 255, 255]),
        }),
    )
    .source(TimeRange::new(Tick::ZERO, Tick(time::TICKS_PER_SECOND)).unwrap())
    .build();
    let payload = json!({
        "version": 1,
        "span": time::TICKS_PER_SECOND,
        "items": [{
            "trackKind": "video",
            "trackIndex": 0,
            "item": pasted,
        }],
    });
    let out = exec(
        &project,
        "clipboard.paste",
        json!({
            "at": time::TICKS_PER_SECOND,
            "source": {"kind": "payload", "payload": payload},
        }),
    );
    assert_eq!(out["created"].as_array().unwrap().len(), 1);
    let saved = project::load(&project).unwrap().project;
    let track = &saved.data().scene().timeline.overlays[0];
    let starts: Vec<i64> = track.items.iter().map(|item| item.start.0).collect();
    assert_eq!(
        starts,
        [0, time::TICKS_PER_SECOND, time::TICKS_PER_SECOND * 2]
    );
    let inserted = &track.items[1];
    assert_eq!(inserted.start, Tick(time::TICKS_PER_SECOND));
    assert_eq!(
        inserted.content,
        project::ItemParams::Overlay(project::OverlayParams {
            color: project::Color([0, 0, 255, 255]),
        })
    );
}

#[test]
fn cli_text_presets_and_animations_execute_reopen_export_and_control_replay() {
    assert!(tools());
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("text.vibecut");
    let output = dir.path().join("text.mp4");
    let mut data = project::Project::empty(project::ProjectId(388), "Fades".into())
        .unwrap()
        .into_data();
    data.canvas.width = 320;
    data.canvas.height = 180;
    data.canvas.fps = time::FrameRate::FPS_30;
    let mut session = engine::Session::new(project::Project::new(data).unwrap(), 10).unwrap();
    let content = project::ItemParams::Text(project::TextParams {
        content: "Tiếng Việt\nEnglish".into(),
        font_size: 24.0,
        background: Some(Default::default()),
        ..Default::default()
    });
    let placed = session
        .apply(engine::Command::InsertItem {
            placement: engine::Placement::Auto,
            item: engine::NewItem {
                source_asset: None,
                start: Tick::ZERO,
                source_range: TimeRange::new(Tick::ZERO, Tick(time::TICKS_PER_SECOND)).unwrap(),
                content,
                animations: Default::default(),
                retime: Default::default(),
            },
        })
        .unwrap();
    let engine::CreatedId::Item(item) = *placed.created.last().unwrap() else {
        panic!("text")
    };
    project::WriterLease::acquire(&file)
        .unwrap()
        .save(session.project())
        .unwrap();
    let saved = exec(
        &file,
        "text.save_preset",
        json!({"item":item.0,"name":"Style A"}),
    );
    let preset = saved["created"][0]["text_preset"].as_u64().unwrap();
    exec(
        &file,
        "timeline.set_item_param",
        json!({"item":item.0,"patch":{"field":"text_size","value":48.0}}),
    );
    let applied = exec(
        &file,
        "text.apply_preset",
        json!({"items":[item.0],"preset":{"kind":"saved","preset":preset}}),
    );
    assert_eq!(applied["changed"], true);
    let reopened = project::load(&file).unwrap().project;
    let titled = reopened
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| &track.items)
        .find(|candidate| candidate.id == item)
        .unwrap();
    let project::ItemParams::Text(style) = &titled.content else {
        panic!("text")
    };
    assert_eq!(style.font_size, 24.0);
    assert_eq!(reopened.data().text_presets[0].style.font, style.font);
    assert_eq!(reopened.data().text_presets[0].name.as_str(), "Style A");
    exec(
        &file,
        "clip.set_animation",
        json!({"item":item.0,"slot":"out","preset":"slide_left"}),
    );
    let reopened = project::load(&file).unwrap().project;
    let titled = reopened
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| &track.items)
        .find(|candidate| candidate.id == item)
        .unwrap();
    let slide = titled
        .animations
        .get(&project::BuiltinPath::TransformPositionX.into())
        .unwrap()
        .channel(None)
        .unwrap();
    assert_eq!(slide.keys().len(), 2);
    for time in slide.keys().iter().map(|key| key.time) {
        exec(
            &file,
            "keyframes.remove",
            json!({"item":item.0,"path":"transform.positionX","component":null,"time":time.0}),
        );
    }
    for slot in ["in", "out"] {
        exec(
            &file,
            "clip.set_animation",
            json!({"item":item.0,"slot":slot,"preset":"fade","duration":76_204_800_000_i64}),
        );
    }
    let saved = project::load(&file).unwrap().project;
    let title = saved
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|t| &t.items)
        .find(|i| i.id == item)
        .unwrap();
    assert_eq!(
        title
            .animations
            .get(&project::BuiltinPath::Opacity.into())
            .unwrap()
            .channel(None)
            .unwrap()
            .keys()
            .len(),
        4
    );
    let before = fs::read(&file).unwrap();
    let refused = cli(&[
        os("exec"),
        file.as_os_str(),
        os("clip.set_animation"),
        os(&json!({"item":item.0,"slot":"combo","preset":"fade"}).to_string()),
        os("--json"),
    ]);
    assert_eq!(code(&refused), 5);
    assert_eq!(fs::read(&file).unwrap(), before);
    assert_eq!(
        code(&cli(&[
            os("export"),
            file.as_os_str(),
            os("--out"),
            output.as_os_str(),
            os("--json")
        ])),
        0
    );
    let frames = decoded_rgb(&output, 320, 180, &[0, 4, 15, 26]);
    let budget = engine::ByteBudget::new(64 << 20).unwrap();
    let hooks = engine::ByteBudgetHooks::new(&budget, None);
    let mut renderer =
        render::Renderer::new(&saved, render::RenderSettings::from_project(&saved)).unwrap();
    for (frame, n) in frames.iter().zip([0, 4, 15, 26]) {
        let expected = renderer
            .render_frame(
                time::FrameRate::FPS_30.tick_of(n).unwrap(),
                &mut render::FileSources::default(),
                &hooks,
            )
            .unwrap()
            .video_rgba8(1, &hooks)
            .unwrap();
        let mse = frame
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
            / frame.len() as f64;
        assert!(mse < 150.0, "CLI title frame {n} MSE {mse}");
    }
    let mut control = super::control::ControlSession::start(&file, &[os("--no-save")]);
    let initial = control.send(json!({"request_id":"inspect","command":"inspect","params":{}}));
    let request = json!({"request_id":"fade","command":"clip.set_animation","params":{"item":item.0,"slot":"in","preset":"fade","duration":76_204_800_000_i64},"base_rev":initial["rev_after"],"idempotency_key":"fade"});
    let applied = control.send(request.clone());
    assert_eq!(applied["exit_code"], 0);
    let undo=control.send(json!({"request_id":"undo","command":"undo","params":{},"base_rev":applied["rev_after"],"idempotency_key":"undo"}));
    assert_eq!(
        undo["exit_code"], 5,
        "reopened session has no earlier history"
    );
    let edit=control.send(json!({"request_id":"key","command":"keyframes.remove","params":{"item":item.0,"path":"opacity","component":null,"time":0},"base_rev":applied["rev_after"],"idempotency_key":"key"}));
    assert_eq!(edit["exit_code"], 0);
    assert_eq!(control.send(request.clone()), applied);
    let mut stale = request;
    stale["idempotency_key"] = json!("stale");
    assert_eq!(control.send(stale)["exit_code"], 3);
    let undo=control.send(json!({"request_id":"undo-key","command":"undo","params":{},"base_rev":edit["rev_after"],"idempotency_key":"undo-key"}));
    assert_eq!(undo["exit_code"], 0);
    assert_eq!(control.finish().0, 0);
}

#[test]
fn cli_pixel_modifiers_execute_and_export_instead_of_copying() {
    if !tools() {
        return;
    }
    for case in ["blur", "mask", "grade", "key"] {
        let masked = case == "mask";
        let dir = tempfile::tempdir().unwrap();
        let video = dir.path().join("input.mp4");
        let project = dir.path().join("blur.vibecut");
        let output_path = dir.path().join("blur.mp4");
        generate_av(&video, 1);
        media_project(&project, &video, None);
        match case {
            "mask" => {
                exec(
                    &project,
                    "masks.add",
                    json!({"item":0,"mask":project::Mask::new("rectangle",project::MaskShape::Rectangle)}),
                );
            }
            "grade" => {
                // Added neutral, then graded through update_param, as the Adjust tab does.
                exec(
                    &project,
                    "effects.add",
                    json!({"item":0,"effect":project::effect_definition("color_grade").unwrap().instance("color_grade")}),
                );
                for (key, value) in [("brightness", 50.0), ("saturation", -50.0)] {
                    exec(
                        &project,
                        "effects.update_param",
                        json!({"item":0,"effect":"color_grade","key":key,"value":value}),
                    );
                }
            }
            "key" => {
                // Picked a green key colour, then raised intensity, as the panel does.
                exec(
                    &project,
                    "effects.add",
                    json!({"item":0,"effect":{"id":"chroma_key","kind":"chroma_key","enabled":true,"params":{"key_color":{"kind":"color","value":[0,255,0,255]},"intensity":{"kind":"scalar","value":20.0},"shadow":{"kind":"scalar","value":0.0},"feather_edge":{"kind":"scalar","value":0.0},"clean_up_edge":{"kind":"scalar","value":0.0}}}}),
                );
                exec(
                    &project,
                    "effects.update_param",
                    json!({"item":0,"effect":"chroma_key","key":"intensity","value":100.0}),
                );
            }
            _ => {
                exec(
                    &project,
                    "effects.add",
                    json!({"item":0,"effect":{"id":"blur","kind":"blur","enabled":true,"params":{"intensity":{"kind":"scalar","value":100.0}}}}),
                );
            }
        }
        let output = cli(&[
            os("export"),
            project.as_os_str(),
            os("--out"),
            output_path.as_os_str(),
            os("--json"),
        ]);
        assert_eq!(
            code(&output),
            0,
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(stdout_json(&output)["status"], "completed");
        let source = decoded_rgb(&video, 128, 96, &[0]);
        let actual = decoded_rgb(&output_path, 128, 96, &[0]);
        let difference: u64 = source[0]
            .iter()
            .zip(&actual[0])
            .map(|(a, b)| u64::from(a.abs_diff(*b)))
            .sum();
        assert!(
            difference > 10000,
            "pixel modifier was bypassed: {difference}"
        );
        if masked {
            for (x, y) in [(8usize, 8usize), (64, 8), (120, 88)] {
                let offset = (y * 128 + x) * 3;
                assert!(
                    actual[0][offset..offset + 3].iter().all(|c| *c < 20),
                    "rectangle did not clip outside"
                );
            }
        }
    }
}

#[test]
fn exec_shared_field_id_list_persists_only_compatible_item_fields() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("Lựa chọn.json");
    let mut data = project::Project::empty(project::ProjectId(0), "Batch".into())
        .unwrap()
        .into_data();
    data.scene_mut().timeline.insert_track(project::Track {
        id: project::TrackId(1),
        name: "Text".into(),
        kind: project::TrackKind::Text,
        muted: false,
        hidden: false,
        locked: false,
        items: (0..2)
            .map(|i| {
                testkit::item::item(
                    project::ItemId(i + 10),
                    project::ItemParams::Text(project::TextParams {
                        content: format!("Text {i}").into(),
                        font_size: 24.0 + i as f32,
                        color: project::Color([255; 4]),
                        ..Default::default()
                    }),
                )
                .source(TimeRange::new(Tick::ZERO, Tick(100)).unwrap())
                .start(Tick(i as i64 * 100))
                .build()
            })
            .collect(),
    });
    let original = project::Project::new(data).unwrap();
    fs::write(&path, project::encode(&original, true).unwrap()).unwrap();
    let result = exec(
        &path,
        "timeline.set_item_param_many",
        json!({"items":[10,11],"patch":{"field":"text_size","value":60}}),
    );
    assert_eq!(result["changed"], true);
    let saved = project::decode(&fs::read(&path).unwrap()).unwrap().project;
    for (before, after) in original
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|t| &t.items)
        .zip(saved.data().scene().timeline.iter().flat_map(|t| &t.items))
    {
        let project::ItemParams::Text(p) = &after.content else {
            panic!("text")
        };
        let project::ItemParams::Text(old) = &before.content else {
            panic!("text")
        };
        assert_eq!(p.font_size, 60.0);
        assert_eq!(p.content, old.content);
        assert_eq!(p.color, old.color);
    }
    let before = fs::read(&path).unwrap();
    let result = exec(
        &path,
        "timeline.set_item_param_many",
        json!({"items":[10,11],"patch":{"field":"volume","value":-6}}),
    );
    assert_eq!(result["changed"], false);
    assert_eq!(fs::read(&path).unwrap(), before);
}

#[test]
fn exec_scrub_equivalent_params_use_the_single_item_command() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("scrub.vibecut");
    let mut data = project::Project::empty(project::ProjectId(0), "Scrub".into())
        .unwrap()
        .into_data();
    data.scene_mut().timeline.insert_track(project::Track {
        id: project::TrackId(1),
        name: "Text".into(),
        kind: project::TrackKind::Text,
        muted: false,
        hidden: false,
        locked: false,
        items: vec![
            testkit::item::item(
                project::ItemId(10),
                project::ItemParams::Text(project::TextParams {
                    content: "Scrub".into(),
                    font_size: 32.0,
                    color: project::Color([255; 4]),
                    ..Default::default()
                }),
            )
            .source(TimeRange::new(Tick::ZERO, Tick(100)).unwrap())
            .build(),
        ],
    });
    let original = project::Project::new(data).unwrap();
    fs::write(&path, project::encode(&original, true).unwrap()).unwrap();

    let changed = exec(
        &path,
        "timeline.set_item_param",
        json!({"item":10,"patch":{"field":"text_size","value":64}}),
    );
    assert_eq!(changed["changed"], true);
    let changed = exec(
        &path,
        "timeline.set_item_param",
        json!({"item":10,"patch":{"field":"opacity","value":0.37}}),
    );
    assert_eq!(changed["changed"], true);
    let saved = project::decode(&fs::read(&path).unwrap()).unwrap().project;
    let item = saved
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| &track.items)
        .find(|item| item.id == project::ItemId(10))
        .unwrap();
    let project::ItemParams::Text(params) = &item.content else {
        panic!("text")
    };
    assert_eq!(params.font_size, 64.0);
    let opacity = item
        .animations
        .value_at(&project::AnimationPath::new("opacity").unwrap(), Tick::ZERO)
        .unwrap();
    assert!(matches!(
        opacity,
        Some(project::ParamValue::Scalar(value)) if (value - 0.37).abs() < 1e-9
    ));

    // Hostile clamp inputs refuse before writing the project.
    let before = fs::read(&path).unwrap();
    let params = json!({"item":10,"patch":{"field":"text_size","value":0}}).to_string();
    let output = cli(&[
        os("exec"),
        path.as_os_str(),
        OsStr::new("timeline.set_item_param"),
        OsStr::new(&params),
        os("--json"),
    ]);
    assert_ne!(code(&output), 0);
    assert_eq!(fs::read(&path).unwrap(), before);
}

#[test]
fn cli_sets_blend_and_opacity_and_renders_a_blended_frame() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("blended.vibecut");
    let output_path = dir.path().join("blended.mp4");
    overlay_project_color(
        &project,
        64,
        64,
        Tick(time::TICKS_PER_SECOND),
        [128, 64, 192, 128],
    );
    let mut data = project::load(&project).unwrap().project.into_data();
    data.canvas.background = project::Color([64, 128, 32, 255]);
    fs::write(
        &project,
        project::encode(&project::Project::new(data).unwrap(), false).unwrap(),
    )
    .unwrap();
    exec(
        &project,
        "timeline.set_item_param",
        json!({"item":0,"patch":{"field":"blendMode","value":"screen"}}),
    );
    exec(
        &project,
        "timeline.set_item_param",
        json!({"item":0,"patch":{"field":"opacity","value":0.5}}),
    );
    let loaded = project::load(&project).unwrap().project;
    let item = loaded
        .data()
        .scene()
        .timeline
        .overlays
        .first()
        .unwrap()
        .items
        .first()
        .unwrap();
    assert_eq!(item.blend_mode, project::BlendMode::Screen);
    let output = cli(&[
        os("export"),
        project.as_os_str(),
        os("--out"),
        output_path.as_os_str(),
        os("--json"),
    ]);
    assert_eq!(
        code(&output),
        0,
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    if !tools() {
        return;
    }
    let frames = decoded_rgb(&output_path, 64, 64, &[0]);
    let expected = [75, 118, 92];
    for (actual, expected) in frames[0][..3].iter().zip(expected) {
        assert!(
            (i32::from(*actual) - expected).abs() <= 12,
            "decoded blend {actual} != {expected}"
        );
    }
}

#[test]
fn cli_sets_rotate_and_exports_the_transformed_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rotated.vibecut");
    let output_path = dir.path().join("rotated.mp4");
    overlay_project_color(
        &path,
        64,
        64,
        Tick(time::TICKS_PER_SECOND),
        [255, 0, 0, 255],
    );
    for (field, value) in [
        ("transform.scaleX", 0.5),
        ("transform.scaleY", 0.25),
        ("transform.rotate", 90.0),
    ] {
        exec(
            &path,
            "timeline.set_item_param",
            json!({"item":0,"patch":{"field":field,"value":value}}),
        );
    }
    let loaded = project::load(&path).unwrap().project;
    let item = loaded
        .data()
        .scene()
        .timeline
        .overlays
        .first()
        .unwrap()
        .items
        .first()
        .unwrap();
    assert_eq!(
        project::TransformField::Rotate
            .value_at(&item.animations, Tick::ZERO)
            .unwrap(),
        90.0
    );
    let output = cli(&[
        os("export"),
        path.as_os_str(),
        os("--out"),
        output_path.as_os_str(),
        os("--json"),
    ]);
    assert_eq!(
        code(&output),
        0,
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    if !tools() {
        return;
    }
    let frames = decoded_rgb(&output_path, 64, 64, &[0]);
    let frame = &frames[0];
    for (x, y, red) in [
        (32, 32, true),
        (32, 19, true),
        (19, 32, false),
        (2, 2, false),
    ] {
        let rgb = &frame[(y * 64 + x) * 3..(y * 64 + x) * 3 + 3];
        if red {
            assert!(
                rgb[0] > 180 && rgb[1] < 40 && rgb[2] < 40,
                "{x},{y}: {rgb:?}"
            );
        } else {
            assert!(rgb.iter().all(|c| *c < 40), "{x},{y}: {rgb:?}");
        }
    }
}

#[test]
fn exec_import_and_insert_share_persisted_media_policy() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("policy.vibecut");
    project::atomic_write(
        &path,
        &project::encode(
            &project::Project::empty(project::ProjectId(1), "policy".into()).unwrap(),
            false,
        )
        .unwrap(),
    )
    .unwrap();
    let asset = serde_json::json!({
        "ephemeral":true,"id":1,"path":dir.path().join("source.mp4"),"content_hash":vec![0;32],
        "probe":{"format":"mp4","duration":1000,"streams":[{
            "index":0,"kind":"Video","codec":"synthetic","time_base":[1,1000],
            "duration":1000,"start":null,"frame_rate":{"num":60000,"den":1001},
            "frame_count":null,"variable_frame_rate":null,
            "video":{"width":640,"height":360,"color":media::ColorInfo::default(),"bit_depth":8},"audio":null
        }]}
    });
    exec(&path, "import.add_asset", json!({"asset":asset}));
    let imported = project::load(&path).unwrap().project;
    assert_eq!(imported.data().canvas.fps, time::FrameRate::FPS_59_94);
    assert_eq!(imported.data().original_canvas_size, None);
    exec(
        &path,
        "timeline.insert",
        json!({"item":{
            "animations":{},"source_asset":1,"source_range":{"start":0,"end":1000},"start":0,
            "content":{"kind":"video","params":{"source_audio_enabled":true,"volume":0.0,"muted":false}}
        }}),
    );
    let inserted = project::load(&path).unwrap().project;
    assert_eq!(inserted.data().canvas.fps, time::FrameRate::FPS_59_94);
    assert_eq!(
        (inserted.data().canvas.width, inserted.data().canvas.height),
        (640, 360)
    );
    assert_eq!(
        inserted.data().original_canvas_size,
        Some(project::CanvasSize {
            width: 640,
            height: 360
        })
    );
    assert!(inserted.data().assets[0].ephemeral());
    assert_eq!(inserted.data().scene().timeline.main.items.len(), 1);
}

/// The drop path's typed placement (a track and a start) inserts a generated clip on that
/// track at that start; the saved project reads back the same item.
#[test]
fn timeline_insert_of_a_generated_clip_on_a_track_at_a_start_lands_there() {
    if !tools() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let video = dir.path().join("clip.mp4");
    let path = dir.path().join("insert.vibecut");
    generate_av(&video, 1);
    media_project(&path, &video, None);
    let duration = media::SourceAsset::import(media::AssetId(1), video.clone())
        .unwrap()
        .probe()
        .duration
        .unwrap();
    let track = add_track(&path, "Overlay")["created"][0]["track"]
        .as_u64()
        .unwrap();
    let start = Tick(time::TICKS_PER_SECOND / 2);
    let content = serde_json::to_value(project::ItemParams::Video(project::VideoParams {
        source_audio_enabled: true,
        ..project::VideoParams::plain()
    }))
    .unwrap();
    let inserted = exec(
        &path,
        "timeline.insert",
        json!({
            "placement": {"mode": "track", "track": track},
            "item": {
                "source_asset": 1,
                "source_range": {"start": 0, "end": duration},
                "start": start,
                "animations": {},
                "content": content,
            }
        }),
    );
    let placed = &inserted["placements"][0];
    assert_eq!(placed["track"].as_u64(), Some(track));
    let item_id = placed["item"].as_u64().unwrap();

    let loaded = project::load(&path).unwrap().project;
    let lane = loaded
        .data()
        .scene()
        .timeline
        .iter()
        .find(|lane| lane.id.0 == track)
        .unwrap();
    let item = lane.items.iter().find(|item| item.id.0 == item_id).unwrap();
    assert_eq!(item.source_asset, Some(media::AssetId(1)));
    assert_eq!(item.start, start);
}

#[test]
fn cli_sets_canvas_sizes_through_the_registry_and_exports_them() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("portrait.vibecut");
    let output_path = dir.path().join("portrait.mp4");
    overlay_project_color(
        &project,
        64,
        64,
        Tick(time::TICKS_PER_SECOND / 30),
        [255, 255, 255, 255],
    );
    exec(
        &project,
        "project.set_canvas",
        json!({"width":1080,"height":1920,"preset":"9:16"}),
    );
    let loaded = project::load(&project).unwrap().project;
    assert_eq!(
        (loaded.data().canvas.width, loaded.data().canvas.height),
        (1080, 1920)
    );
    assert_eq!(
        loaded.data().canvas.preset,
        Some(project::AspectPreset::Portrait)
    );
    let output = cli(&[
        os("export"),
        project.as_os_str(),
        os("--out"),
        output_path.as_os_str(),
        os("--json"),
    ]);
    assert_eq!(
        code(&output),
        0,
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    if tools() {
        let probe = tool_command("ffprobe")
            .args([
                os("-v"),
                os("error"),
                os("-select_streams"),
                os("v:0"),
                os("-show_entries"),
                os("stream=width,height"),
                os("-of"),
                os("csv=p=0"),
            ])
            .arg(&output_path)
            .output()
            .unwrap();
        assert!(probe.status.success());
        assert_eq!(String::from_utf8_lossy(&probe.stdout).trim(), "1080,1920");
    }

    // An odd custom size commits its nearest even value, so the same project
    // still exports instead of failing on an odd render size.
    exec(
        &project,
        "project.set_canvas",
        json!({"width":1081,"height":721}),
    );
    let rounded = project::load(&project).unwrap().project;
    assert_eq!(
        (rounded.data().canvas.width, rounded.data().canvas.height),
        (1082, 722)
    );
    assert_eq!(rounded.data().canvas.preset, None);
    assert_eq!(
        rounded.data().canvas.last_custom_size,
        project::CanvasSize {
            width: 1082,
            height: 722
        }
    );
    let odd_path = dir.path().join("odd.mp4");
    let output = cli(&[
        os("export"),
        project.as_os_str(),
        os("--out"),
        odd_path.as_os_str(),
        os("--json"),
    ]);
    assert_eq!(
        code(&output),
        0,
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    if tools() {
        let probe = tool_command("ffprobe")
            .args([
                os("-v"),
                os("error"),
                os("-select_streams"),
                os("v:0"),
                os("-show_entries"),
                os("stream=width,height"),
                os("-of"),
                os("csv=p=0"),
            ])
            .arg(&odd_path)
            .output()
            .unwrap();
        assert!(probe.status.success());
        assert_eq!(String::from_utf8_lossy(&probe.stdout).trim(), "1082,722");
    }
}

#[test]
fn audio_set_fade_shapes_the_exported_tone_envelope() {
    if !tools() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let video = dir.path().join("video.mp4");
    let audio = dir.path().join("tone.wav");
    let project = dir.path().join("fade.vibecut");
    let out = dir.path().join("fade.mp4");
    generate_av(&video, 1);
    generate_audio(&audio);
    media_project(&project, &video, Some(&audio));
    // 250 ms edges: equal-power in, linear out.
    let quarter = time::TICKS_PER_SECOND / 4;
    exec(
        &project,
        "audio.set_fade",
        json!({"item":1,"edge":"in","duration":quarter,"curve":"equal_power"}),
    );
    exec(
        &project,
        "audio.set_fade",
        json!({"item":1,"edge":"out","duration":quarter,"curve":"linear"}),
    );
    // Hostile input is refused without touching the saved project.
    let before = fs::read(&project).unwrap();
    let refused = cli(&[
        os("exec"),
        project.as_os_str(),
        os("audio.set_fade"),
        os(r#"{"item":1,"edge":"in","duration":-1,"curve":"linear"}"#),
        os("--json"),
    ]);
    assert_ne!(code(&refused), 0);
    assert_eq!(fs::read(&project).unwrap(), before);

    let exported = cli(&[
        os("export"),
        project.as_os_str(),
        os("--out"),
        out.as_os_str(),
        os("--json"),
    ]);
    assert_eq!(
        code(&exported),
        0,
        "{}",
        String::from_utf8_lossy(&exported.stderr)
    );
    let pcm = decoded_mono(&out);
    assert!(pcm.len() >= 47_000, "{}", pcm.len());
    // RMS of a 20 ms window centred on `second`.
    let rms = |second: f64| {
        let centre = (second * 48_000.0) as usize;
        let window = &pcm[centre - 480..centre + 480];
        (window.iter().map(|x| x * x).sum::<f32>() / window.len() as f32).sqrt()
    };
    let body = rms(0.5);
    assert!(body > 0.05, "tone body {body}");
    // Edges are silent (AAC adds only a faint pre-echo), midpoints follow each curve.
    assert!(pcm[..48].iter().all(|x| x.abs() < body * 0.1));
    assert!(rms(0.01) < body * 0.15, "fade-in start {}", rms(0.01));
    assert!(rms(0.99) < body * 0.15, "fade-out end {}", rms(0.99));
    let mid_in = rms(0.125) / body;
    let mid_out = rms(0.875) / body;
    assert!(
        (mid_in - 0.707).abs() < 0.12,
        "equal-power midpoint {mid_in}"
    );
    assert!((mid_out - 0.5).abs() < 0.12, "linear midpoint {mid_out}");
}

#[test]
fn audio_cleanup_effects_process_the_exported_tone_and_refuse_hostile_input() {
    if !tools() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let video = dir.path().join("video.mp4");
    let audio = dir.path().join("tone.wav");
    let project = dir.path().join("cleanup.vibecut");
    generate_av(&video, 1);
    generate_audio(&audio);
    media_project(&project, &video, Some(&audio));
    let export = |name: &str| {
        let out = dir.path().join(name);
        let exported = cli(&[
            os("export"),
            project.as_os_str(),
            os("--out"),
            out.as_os_str(),
            os("--json"),
        ]);
        assert_eq!(
            code(&exported),
            0,
            "{}",
            String::from_utf8_lossy(&exported.stderr)
        );
        decoded_mono(&out)
    };
    let plain = export("plain.mp4");

    // Out-of-range parameters are refused without touching the saved project.
    let before = fs::read(&project).unwrap();
    for kind in [
        json!({"kind":"noise_reduction","reduction_db":99.0,"threshold_db":6.0,"smoothing_percent":50.0}),
        json!({"kind":"declick","threshold":30.0,"complexity":0.0}),
    ] {
        let params =
            json!({"item":1,"effect":{"id":1,"enabled":true,"wet_percent":100.0,"kind":kind}})
                .to_string();
        let refused = cli(&[
            os("exec"),
            project.as_os_str(),
            os("audio.add_cleanup_effect"),
            OsStr::new(&params),
            os("--json"),
        ]);
        assert_ne!(code(&refused), 0);
        assert_eq!(fs::read(&project).unwrap(), before);
    }

    exec(
        &project,
        "audio.add_cleanup_effect",
        json!({"item":1,"effect":{"id":1,"enabled":true,"wet_percent":100.0,
            "kind":{"kind":"declick","threshold":30.0,"complexity":16.0}}}),
    );
    exec(
        &project,
        "audio.add_cleanup_effect",
        json!({"item":1,"effect":{"id":2,"enabled":true,"wet_percent":100.0,
            "kind":{"kind":"noise_reduction","reduction_db":12.0,"threshold_db":6.0,"smoothing_percent":50.0}}}),
    );
    let loaded = project::load(&project).unwrap().project;
    let item = loaded
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| &track.items)
        .find(|item| item.id == project::ItemId(1))
        .unwrap();
    let project::ItemParams::Audio(params) = &item.content else {
        panic!("audio item")
    };
    assert_eq!(params.cleanup.effects.len(), 2);

    // The export keeps its length: the chain's delay is compensated, not added.
    let cleaned = export("cleaned.mp4");
    assert!(
        cleaned.len().abs_diff(plain.len()) < 2048,
        "{} vs {}",
        cleaned.len(),
        plain.len()
    );
    // Noise reduction leaves the gate open for the first sub-window, then treats the steady tone
    // as the noise floor and attenuates it: the head is unchanged and the body is much quieter.
    let rms = |pcm: &[f32], from: f64, to: f64| {
        let window = &pcm[(from * 48_000.0) as usize..(to * 48_000.0) as usize];
        (window.iter().map(|x| x * x).sum::<f32>() / window.len() as f32).sqrt()
    };
    let head = rms(&cleaned, 0.05, 0.15) / rms(&plain, 0.05, 0.15);
    assert!((0.7..1.3).contains(&head), "head ratio {head}");
    let body = rms(&cleaned, 0.6, 0.9) / rms(&plain, 0.6, 0.9);
    assert!(body < 0.6, "body ratio {body}");
}

#[test]
fn cli_retimes_linked_clips_and_the_export_follows_speed_and_pitch() {
    if !tools() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let video = dir.path().join("video.mp4");
    let audio = dir.path().join("audio.wav");
    let project = dir.path().join("retime.vibecut");
    let output_path = dir.path().join("retime.mp4");
    generate_av(&video, 1);
    generate_audio(&audio);
    media_project(&project, &video, Some(&audio));
    // Video item 0 and audio item 1 are retimed together, keeping 1 s of source at 2x.
    exec(
        &project,
        "timeline.set_speed",
        json!({"items":[0,1],"speed":2.0,"maintain_pitch":true}),
    );
    assert_eq!(
        info(&project)["duration"]["seconds"],
        0.5,
        "1 s of source at 2x is 0.5 s"
    );
    let output = cli(&[
        os("export"),
        project.as_os_str(),
        os("--out"),
        output_path.as_os_str(),
        os("--json"),
    ]);
    assert_eq!(
        code(&output),
        0,
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let probe = tool_command("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "stream=codec_type,nb_frames,duration",
            "-of",
            "json",
        ])
        .arg(&output_path)
        .output()
        .unwrap();
    let value: Value = serde_json::from_slice(&probe.stdout).unwrap();
    let streams = value["streams"].as_array().unwrap();
    let video_stream = streams.iter().find(|s| s["codec_type"] == "video").unwrap();
    assert_eq!(
        video_stream["nb_frames"], "15",
        "1 s of 30 fps source at 2x is 15 frames"
    );
    let audio_stream = streams.iter().find(|s| s["codec_type"] == "audio").unwrap();
    let seconds: f64 = audio_stream["duration"].as_str().unwrap().parse().unwrap();
    assert!((seconds - 0.5).abs() < 0.06, "audio lasts {seconds} s");
    // Pitch is preserved: the 440 Hz source tone still measures 440 Hz at 2x.
    let decoded = tool_command("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(&output_path)
        .args(["-vn", "-ac", "1", "-ar", "48000", "-f", "f32le", "-"])
        .output()
        .unwrap();
    let samples: Vec<f32> = decoded
        .stdout
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| f32::from_le_bytes(*b))
        .collect();
    let (from, to) = (4800, samples.len().min(21600));
    let crossings = samples[from..to]
        .windows(2)
        .filter(|w| w[0] <= 0.0 && w[1] > 0.0)
        .count();
    let hertz = crossings as f64 / ((to - from) as f64 / 48000.0);
    assert!((hertz - 440.0).abs() < 15.0, "measured {hertz} Hz");

    // Reverse then export: the same 0.5 s at 2x plays backwards with the same frame count.
    exec(
        &project,
        "timeline.set_reverse",
        json!({"items":[0,1],"reverse":true}),
    );
    assert_eq!(
        info(&project)["duration"]["seconds"],
        0.5,
        "reverse keeps the span"
    );
    let reversed_path = dir.path().join("retime-reversed.mp4");
    let output = cli(&[
        os("export"),
        project.as_os_str(),
        os("--out"),
        reversed_path.as_os_str(),
        os("--json"),
    ]);
    assert_eq!(
        code(&output),
        0,
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let probe = tool_command("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "stream=codec_type,nb_frames,duration",
            "-of",
            "json",
        ])
        .arg(&reversed_path)
        .output()
        .unwrap();
    let value: Value = serde_json::from_slice(&probe.stdout).unwrap();
    let streams = value["streams"].as_array().unwrap();
    let video_stream = streams.iter().find(|s| s["codec_type"] == "video").unwrap();
    assert_eq!(video_stream["nb_frames"], "15", "reversed frame count");
}

#[test]
fn cli_clipboard_color_and_opacity_match_analytic_source_over_preview_and_encode() {
    assert!(tools(), "required ffmpeg/ffprobe setup");
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("clipboard-color.vibecut");
    let encoded = dir.path().join("clipboard-color.mp4");
    overlay_project_color(&file, 64, 64, Tick(time::TICKS_PER_SECOND / 10), [0; 4]);
    let mut data = project::load(&file).unwrap().project.into_data();
    data.canvas.background = project::Color([89, 124, 149, 255]);
    let target = &mut data.scene_mut().timeline.overlays[0].items[0];
    target
        .animations
        .set(
            project::BuiltinPath::Opacity.into(),
            project::Param::new(project::ParamValue::Scalar(1.0)).unwrap(),
        )
        .unwrap();
    target
        .animations
        .set(
            project::AnimationPath::new("params.strokeColor").unwrap(),
            project::Param::new(project::ParamValue::Color([0.0; 4])).unwrap(),
        )
        .unwrap();
    fs::write(
        &file,
        project::encode(&project::Project::new(data).unwrap(), false).unwrap(),
    )
    .unwrap();
    let scalar = |v: f64| json!({"value":{"kind":"scalar","value":v},"keys":[{"time":0,"value":{"kind":"scalar","value":v},"segment":"linear","tangent":"flat","incoming":null,"outgoing":null}],"before":"hold","after":"hold"});
    let paste = json!({"item":0,"at":0,"source":{"kind":"payload","payload":{"version":1,"origin":0,"params":[{"path":"opacity","param":{"kind":"value","channels":scalar(0.5)}},{"path":"params.strokeColor","param":{"kind":"color","channels":[scalar(0.8),scalar(0.2),scalar(0.1),scalar(0.25)]}}]}}});
    exec(&file, "clipboard.paste_keyframes", paste.clone());
    let saved = project::load(&file).unwrap().project;
    let budget = engine::ByteBudget::new(64 << 20).unwrap();
    let hooks = engine::ByteBudgetHooks::new(&budget, None);
    let mut renderer =
        render::Renderer::new(&saved, render::RenderSettings::from_project(&saved)).unwrap();
    let preview = renderer
        .preview_frame(Tick::ZERO, &mut render::FileSources::default(), &hooks)
        .unwrap();
    let frame = renderer
        .render_frame(Tick::ZERO, &mut render::FileSources::default(), &hooks)
        .unwrap();
    assert_eq!(
        preview.video_rgba8(1, &hooks).unwrap().bytes,
        frame.video_rgba8(1, &hooks).unwrap().bytes
    );
    // The current consumed stroke-color boundary quantizes straight sRGB/alpha
    // to [231,124,89,64] before working-space source-over. Author the conversion
    // independently, then use a 3e-6 working-linear tolerance.
    let linear = |v: f64| {
        let x = v / 255.0;
        if x <= 0.04045 {
            x / 12.92
        } else {
            ((x + 0.055) / 1.055).powf(2.4)
        }
    };
    let mut expected = [0.0; 3];
    for (c, (s, b)) in expected
        .iter_mut()
        .zip([(231.0, 89.0), (124.0, 124.0), (89.0, 149.0)])
    {
        *c = linear(b) + (linear(s) - linear(b)) * (32.0 / 255.0);
    }
    for (actual, wanted) in frame.pixels()[0][..3].iter().zip(expected) {
        assert!(
            (f64::from(*actual) - wanted).abs() < 3e-6,
            "{actual} vs {wanted}"
        );
    }
    let output = cli(&[
        os("export"),
        file.as_os_str(),
        os("--out"),
        encoded.as_os_str(),
        os("--json"),
    ]);
    assert_eq!(
        code(&output),
        0,
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let proof = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/p1-20-clipboard-color.mp4");
    fs::create_dir_all(proof.parent().unwrap()).unwrap();
    fs::copy(&encoded, proof).unwrap();
    let decoded = decoded_rgb(&encoded, 64, 64, &[0]);
    for (actual, wanted) in decoded[0][..3].iter().zip(expected) {
        let wanted = (video_oetf(wanted) * 255.0).round() as i32;
        assert!(
            (i32::from(*actual) - wanted).abs() <= 12,
            "encoded {actual} vs {wanted}"
        );
    }
    let before = fs::read(&file).unwrap();
    let mut invalid = paste.clone();
    invalid["source"]["payload"]["version"] = json!(2);
    let invalid_json = invalid.to_string();
    let refused = cli(&[
        os("exec"),
        file.as_os_str(),
        os("clipboard.paste_keyframes"),
        os(&invalid_json),
        os("--json"),
    ]);
    assert_eq!(code(&refused), 2);
    assert_eq!(fs::read(&file).unwrap(), before);
    exec(&file, "track.set_locked", json!({"track":1,"locked":true}));
    let before = fs::read(&file).unwrap();
    let mut locked = paste;
    locked["at"] = json!(40);
    let locked_json = locked.to_string();
    let refused = cli(&[
        os("exec"),
        file.as_os_str(),
        os("clipboard.paste_keyframes"),
        os(&locked_json),
        os("--json"),
    ]);
    assert_eq!(code(&refused), 5);
    assert_eq!(fs::read(&file).unwrap(), before);
}

#[test]
fn cli_graphics_insert_key_save_snapshot_encode_and_reopen_real_processes() {
    assert!(tools());
    for graphic in [
        project::GraphicParams::rectangle(),
        project::GraphicParams::ellipse(),
        project::GraphicParams::polygon(),
        project::GraphicParams::star(),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("Hình.vibecut");
        let mut data = project::Project::empty(project::ProjectId(397), "Graphics".into())
            .unwrap()
            .into_data();
        data.canvas.width = 320;
        data.canvas.height = 180;
        project::WriterLease::acquire(&file)
            .unwrap()
            .save(&project::Project::new(data).unwrap())
            .unwrap();
        let inserted = exec(
            &file,
            "timeline.insert",
            json!({"placement":{"mode":"auto"},"item":{"source_asset":null,"source_range":{"start":0,"end":time::TICKS_PER_SECOND},"start":0,"animations":{},"content":project::ItemParams::Graphic(graphic)}}),
        );
        let item = inserted["placements"][0]["item"].clone();
        assert!(item.is_u64());
        exec(
            &file,
            "timeline.set_item_param",
            json!({"item":item,"patch":{"field":"graphic_stroke_width","value":8.}}),
        );
        let color = project::Param::new(project::ParamValue::Color([1., 0., 0., 0.5])).unwrap();
        exec(
            &file,
            "keyframes.set_param",
            json!({"item":item,"path":"params.fill","param":color}),
        );
        exec(
            &file,
            "keyframes.add",
            json!({"item":item,"path":"params.fill","component":"a","key":project::Keyframe::new(Tick(0),project::ParamValue::Scalar(0.5))}),
        );
        let png = dir.path().join("graphic.png");
        let before = fs::read(&file).unwrap();
        let params = json!({"time":0,"path":png}).to_string();
        let refused = cli(&[
            os("exec"),
            file.as_os_str(),
            os("preview.snapshot"),
            os(&params),
            os("--json"),
        ]);
        assert_eq!(code(&refused), 5);
        assert_eq!(fs::read(&file).unwrap(), before);
        assert!(!png.exists());
        let mut snapshot = super::control::ControlSession::start(&file, &[]);
        let initial =
            snapshot.send(json!({"request_id":"inspect","command":"inspect","params":{}}));
        let response=snapshot.send(json!({"request_id":"png","command":"preview.snapshot","params":{"time":0,"path":png},"base_rev":initial["rev_after"],"idempotency_key":"png"}));
        assert_eq!(response["exit_code"], 0, "{response}");
        assert_eq!(snapshot.finish().0, 0);
        assert_eq!(fs::read(&file).unwrap(), before);
        let mut snapshot = super::control::ControlSession::start(&file, &[]);
        let initial =
            snapshot.send(json!({"request_id":"inspect","command":"inspect","params":{}}));
        let alias=snapshot.send(json!({"request_id":"alias","command":"preview.snapshot","params":{"time":0,"path":file},"base_rev":initial["rev_after"],"idempotency_key":"alias"}));
        assert_eq!(alias["exit_code"], 5);
        assert_eq!(snapshot.finish().0, 5);
        assert_eq!(fs::read(&file).unwrap(), before);
        let pixels = testkit::oracle::rgba(&png);
        let center = &pixels[(90 * 320 + 160) * 4..(90 * 320 + 160) * 4 + 4];
        assert!((i16::from(center[0]) - 188).abs() <= 1);
        assert_eq!(&center[1..], &[0, 0, 255]);
        let saved = project::load(&file).unwrap().project;
        assert!(matches!(
            saved.data().scene().timeline.main.items[0].content,
            project::ItemParams::Graphic(_)
        ));
        let output = dir.path().join("graphic.mp4");
        let encoded = cli(&[
            os("export"),
            file.as_os_str(),
            os("--out"),
            output.as_os_str(),
            os("--json"),
        ]);
        assert_eq!(
            code(&encoded),
            0,
            "{}",
            String::from_utf8_lossy(&encoded.stderr)
        );
        let probe = testkit::oracle::probe(&output);
        assert_eq!(probe["streams"][0]["width"], 320);
        assert_eq!(probe["streams"][0]["height"], 180);
        let frames = decoded_rgb(&output, 320, 180, &[0, 15, 29]);
        let budget = engine::ByteBudget::new(64 << 20).unwrap();
        let hooks = engine::ByteBudgetHooks::new(&budget, None);
        let mut renderer =
            render::Renderer::new(&saved, render::RenderSettings::from_project(&saved)).unwrap();
        for (frame, n) in frames.iter().zip([0, 15, 29]) {
            let expected = renderer
                .render_frame(
                    time::FrameRate::FPS_30.tick_of(n).unwrap(),
                    &mut render::FileSources::default(),
                    &hooks,
                )
                .unwrap()
                .video_rgba8(1, &hooks)
                .unwrap();
            let mse = frame
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
                / frame.len() as f64;
            assert!(mse < 150., "graphic frame{n} MSE{mse}");
        }
        let mut control = super::control::ControlSession::start(&file, &[os("--no-save")]);
        let initial = control.send(json!({"request_id":"inspect","command":"inspect","params":{}}));
        let request = json!({"request_id":"graphic","command":"timeline.set_item_param","params":{"item":item,"patch":{"field":"graphic_stroke_width","value":4.}},"base_rev":initial["rev_after"],"idempotency_key":"graphic"});
        let applied = control.send(request.clone());
        assert_eq!(applied["exit_code"], 0);
        assert_eq!(control.send(request.clone()), applied);
        let mut stale = request;
        stale["idempotency_key"] = json!("stale");
        assert_eq!(control.send(stale)["exit_code"], 3);
        assert_eq!(control.send(json!({"request_id":"undo","command":"undo","params":{},"base_rev":applied["rev_after"],"idempotency_key":"undo"}))["exit_code"],0);
        assert_eq!(control.finish().0, 0);
    }
}

#[test]
fn exec_freeform_mask_path_round_trips_through_save_and_reload() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("freeform.vibecut");
    new_project(&path);
    let track = add_track(&path, "Overlay")["created"][0]["track"]
        .as_u64()
        .unwrap();
    let inserted = exec(
        &path,
        "timeline.insert",
        json!({"placement":{"mode":"track","track":track},"item":{"source_range":{"start":0,"end":1000},"start":0,"animations":{},"content":{"kind":"overlay","params":{"color":[40,80,120,255]}}}}),
    );
    let item = inserted["placements"][0]["item"].as_u64().unwrap();
    exec(
        &path,
        "masks.add",
        json!({"item":item,"mask":project::Mask::new("p",project::MaskShape::Path{path:project::MaskPath::ellipse()})}),
    );
    let mut dragged = project::MaskPath::ellipse();
    dragged.vertices[1].t_out = [0.0, 0.5];
    let expected = dragged.split(1, 0.5).unwrap();
    exec(
        &path,
        "masks.set_path",
        json!({"item":item,"mask":"p","path":dragged}),
    );
    exec(
        &path,
        "masks.add_point",
        json!({"item":item,"mask":"p","segment":1,"t":0.5}),
    );
    let added = project::load(&path).unwrap().project;
    let stored = added
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| track.items.iter())
        .find(|candidate| candidate.id == project::ItemId(item))
        .and_then(|item| item.masks.first())
        .unwrap();
    assert!(matches!(&stored.shape, project::MaskShape::Path { .. }));
    let project::MaskShape::Path { path: stored_path } = &stored.shape else {
        return;
    };
    assert_eq!(stored_path, &expected);
    assert!(stored_path.closed);
    exec(
        &path,
        "masks.remove_point",
        json!({"item":item,"mask":"p","points":[2]}),
    );
    let reloaded = project::load(&path).unwrap().project;
    let stored = reloaded
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|track| track.items.iter())
        .find(|candidate| candidate.id == project::ItemId(item))
        .and_then(|item| item.masks.first())
        .unwrap();
    assert!(matches!(&stored.shape, project::MaskShape::Path { .. }));
    let project::MaskShape::Path { path: stored_path } = &stored.shape else {
        return;
    };
    assert_eq!(stored_path.vertices.len(), 4);
    // Removal keeps the survivors' tangents exactly as the split left them.
    assert_eq!(stored_path, &expected.without(&[2]).unwrap());
}

#[test]
fn exec_runs_the_sticker_commands_with_the_bundled_catalog_lent() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("stickers.vibecut");
    new_project(&path);
    let found = exec(
        &path,
        "stickers.search",
        json!({"query":"","category":"flags","limit":5}),
    );
    assert_eq!(found["kind"], json!("sticker_search"));
    assert!(found["page"]["total"].as_u64().unwrap() >= 1, "{found}");
    let inserted = exec(
        &path,
        "stickers.insert",
        json!({"placement":{"mode":"auto"},"sticker":"flags:AF","start":0,"duration":127008000000_i64}),
    );
    assert_eq!(inserted["changed"], json!(true), "{inserted}");
    assert_eq!(inserted["placements"].as_array().map(Vec::len), Some(1));
}

#[test]
fn cli_applies_a_speed_ramp_to_linked_clips_and_the_export_follows_the_curve() {
    if !tools() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let video = dir.path().join("video.mp4");
    let audio = dir.path().join("audio.wav");
    let project = dir.path().join("ramp.vibecut");
    let output_path = dir.path().join("ramp.mp4");
    generate_av(&video, 1);
    generate_audio(&audio);
    media_project(&project, &video, Some(&audio));
    // 1 s of source in four 0.25 s stretches at 0.5x, 0.8x, 1.25x and 2x plays for
    // 0.5 + 0.3125 + 0.2 + 0.125 = 1.1375 s.
    exec(
        &project,
        "timeline.apply_speed_preset",
        json!({"items":[0,1],"preset":"slow_in_fast_out","maintain_pitch":true}),
    );
    let seconds = info(&project)["duration"]["seconds"].as_f64().unwrap();
    assert!((seconds - 1.1375).abs() < 1e-6, "ramp duration {seconds}");
    // An edited curve replaces it: half the source at 1x, half at 4x is 0.5 + 0.125 s.
    exec(
        &project,
        "timeline.set_speed_curve",
        json!({"items":[0,1],"segments":[{"span":500,"speed":1.0},{"span":500,"speed":4.0}],"maintain_pitch":true}),
    );
    let seconds = info(&project)["duration"]["seconds"].as_f64().unwrap();
    assert!(
        (seconds - 0.625).abs() < 1e-6,
        "edited ramp duration {seconds}"
    );
    exec(
        &project,
        "timeline.apply_speed_preset",
        json!({"items":[0,1],"preset":"slow_in_fast_out","maintain_pitch":true}),
    );
    let output = cli(&[
        os("export"),
        project.as_os_str(),
        os("--out"),
        output_path.as_os_str(),
        os("--json"),
    ]);
    assert_eq!(
        code(&output),
        0,
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let probe = tool_command("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "stream=codec_type,nb_frames,duration",
            "-of",
            "json",
        ])
        .arg(&output_path)
        .output()
        .unwrap();
    let value: Value = serde_json::from_slice(&probe.stdout).unwrap();
    let streams = value["streams"].as_array().unwrap();
    let video_stream = streams.iter().find(|s| s["codec_type"] == "video").unwrap();
    let frames: i64 = video_stream["nb_frames"].as_str().unwrap().parse().unwrap();
    assert!(
        (34..=35).contains(&frames),
        "1.1375 s at 30 fps is about 34 frames, got {frames}"
    );
    let audio_stream = streams.iter().find(|s| s["codec_type"] == "audio").unwrap();
    let length: f64 = audio_stream["duration"].as_str().unwrap().parse().unwrap();
    assert!((length - 1.1375).abs() < 0.06, "audio lasts {length} s");
}

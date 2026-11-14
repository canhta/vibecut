//! Verifies native exec command behaviour.
use crate::cli::fixtures::{add_track, exec, info, new_project, overlay_project_color};
use crate::cli::fixtures::{decoded_rgb, video_oetf};
use crate::cli::process::{cli, code, error_json, os};
use crate::media_fixture::{generate_av, media_project, tools};
use serde_json::json;
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
fn cli_graphics_insert_key_save_snapshot_encode_and_reopen_real_processes() {
    // The shape kind is render's geometry; one kind carries the process round trip.
    assert!(tools());
    let graphic = project::GraphicParams::rectangle();
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


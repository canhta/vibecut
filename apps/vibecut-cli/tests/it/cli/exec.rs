use super::media_fixture::{decoded_mono, decoded_rgb, generate_audio, video_oetf};
use super::process::{cli, code, error_json, os, stdout_json};
use super::project_fixture::{add_track, exec, info, new_project, overlay_project_color};
use crate::common::{generate_av, media_project, tool_command, tools};
use serde_json::{Value, json};
use std::ffi::OsStr;
use std::fs;
use time::{Tick, TimeRange};

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
            "segment": "linear",
            "tangent": "flat",
            "incoming": null,
            "outgoing": null,
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
        "keyframes.add",
        json!({
            "item": 0,
            "path": "opacity",
            "component": null,
            "key": key(time::TICKS_PER_SECOND, 1.0),
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
    if !tools() {
        return;
    }
    let frames = decoded_rgb(&output_path, 64, 64, &[0, 15, 30]);
    for (frame, opacity) in frames.iter().zip([0.0, 0.5, 1.0]) {
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
fn cli_pixel_modifiers_execute_and_export_instead_of_copying() {
    if !tools() {
        return;
    }
    for masked in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let video = dir.path().join("input.mp4");
        let project = dir.path().join("blur.vibecut");
        let output_path = dir.path().join("blur.mp4");
        generate_av(&video);
        media_project(&project, &video, None);
        if masked {
            exec(
                &project,
                "masks.add",
                json!({"item":0,"mask":project::Mask::new("rectangle",project::MaskShape::Rectangle)}),
            );
        } else {
            exec(
                &project,
                "effects.add",
                json!({"item":0,"effect":{"id":"blur","enabled":true,"params":{"kind":"blur","intensity":100.0}}}),
            );
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
            .map(|i| project::Item {
                retime: Default::default(),
                blend_mode: Default::default(),
                masks: Vec::new(),
                effects: vec![],
                animations: Default::default(),
                id: project::ItemId(i + 10),
                source_asset: None,
                source_range: TimeRange::new(Tick::ZERO, Tick(100)).unwrap(),
                start: Tick(i as i64 * 100),
                content: project::ItemParams::Text(project::TextParams {
                    content: format!("Text {i}").into(),
                    font_size: 24.0 + i as f32,
                    color: project::Color([255; 4]),
                    ..Default::default()
                }),
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
        items: vec![project::Item {
            retime: Default::default(),
            blend_mode: Default::default(),
            masks: Vec::new(),
            effects: vec![],
            animations: Default::default(),
            id: project::ItemId(10),
            source_asset: None,
            source_range: TimeRange::new(Tick::ZERO, Tick(100)).unwrap(),
            start: Tick::ZERO,
            content: project::ItemParams::Text(project::TextParams {
                content: "Scrub".into(),
                font_size: 32.0,
                color: project::Color([255; 4]),
                ..Default::default()
            }),
        }],
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
    generate_av(&video);
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
fn cli_retimes_linked_clips_and_the_export_follows_speed_and_pitch() {
    if !tools() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let video = dir.path().join("video.mp4");
    let audio = dir.path().join("audio.wav");
    let project = dir.path().join("retime.vibecut");
    let output_path = dir.path().join("retime.mp4");
    generate_av(&video);
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
}

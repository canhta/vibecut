//! Verifies native export command behaviour.
use crate::cli::fixtures::{decoded_rgb, exec, generate_audio};
use crate::cli::fixtures::{overlay_project, overlay_project_color};
use crate::cli::process::{cli, code, error_json, os, stdout_json};
use crate::media_fixture::{ffmpeg, generate_av, media_project, tool_command, tools};
use serde_json::{Value, json};
use std::fs;
use time::{Tick, TimeRange};

#[test]
fn typed_export_options_cover_formats_ranges_profiles_and_scene_scope_with_probe() {
    if !tools() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("options.vibecut");
    overlay_project(&path, 64, 64, Tick(time::TICKS_PER_SECOND));
    let before = fs::read(&path).unwrap();
    for (extension, codec, profile) in [
        ("MP4", render::VideoCodec::H264, media::H264Profile::Main),
        ("mov", render::VideoCodec::H264, media::H264Profile::High),
        (
            "webm",
            render::VideoCodec::Vp9,
            media::H264Profile::Automatic,
        ),
    ] {
        for range_kind in [0, 1, 2] {
            let output_path = dir.path().join(format!("{range_kind}.{extension}"));
            let start = Tick(time::TICKS_PER_SECOND / 4);
            let end = Tick(time::TICKS_PER_SECOND / 2);
            let options = render::OutputOptions {
                resolution: Some([64, 96]),
                fps: Some(time::FrameRate::FPS_24),
                bit_rate: Some(200_000),
                codec,
                profile,
                range: match range_kind {
                    0 => render::RangeChoice::Full,
                    1 => render::RangeChoice::InOut { start, end },
                    _ => render::RangeChoice::Selected { start, end },
                },
                ..Default::default()
            };
            let settings = serde_json::to_string(&options).unwrap();
            let output = cli(&[
                os("export"),
                path.as_os_str(),
                os("--out"),
                output_path.as_os_str(),
                os("--settings"),
                os(&settings),
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
                    "-count_frames",
                    "-show_streams",
                    "-of",
                    "json",
                ])
                .arg(&output_path)
                .output()
                .unwrap();
            assert!(probe.status.success());
            let value: Value = serde_json::from_slice(&probe.stdout).unwrap();
            let video = value["streams"]
                .as_array()
                .unwrap()
                .iter()
                .find(|stream| stream["codec_type"] == "video")
                .unwrap();
            assert_eq!(
                (video["width"].as_u64(), video["height"].as_u64()),
                (Some(64), Some(96))
            );
            assert_eq!(video["r_frame_rate"], "24/1");
            // Timeline markers snap on the project frame grid before resampling.
            let project = project::load(&path).unwrap().project;
            let duration = options.ranges(&project).unwrap()[0].1.duration();
            let frames = (duration.0 + time::FrameRate::FPS_24.frame_duration().unwrap().0 - 1)
                / time::FrameRate::FPS_24.frame_duration().unwrap().0;
            assert_eq!(video["nb_read_frames"], frames.to_string());
            assert_eq!(
                video["codec_name"],
                if extension == "webm" { "vp9" } else { "h264" }
            );
            if profile != media::H264Profile::Automatic {
                assert_eq!(
                    video["profile"],
                    if profile == media::H264Profile::Main {
                        "Main"
                    } else {
                        "High"
                    }
                );
            }
            assert_eq!(fs::read(&path).unwrap(), before);
        }
    }
    let invalid = serde_json::to_string(&render::OutputOptions {
        range: render::RangeChoice::InOut {
            start: Tick::ZERO,
            end: Tick::ZERO,
        },
        ..Default::default()
    })
    .unwrap();
    let destination = dir.path().join("refused.mp4");
    let output = cli(&[
        os("export"),
        path.as_os_str(),
        os("--out"),
        destination.as_os_str(),
        os("--settings"),
        os(&invalid),
    ]);
    assert_ne!(code(&output), 0);
    assert!(!destination.exists());
}

#[test]
fn export_pattern_writes_a_png_sequence_directory_and_refuses_hostile_choices() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("overlay.vibecut");
    overlay_project(&project, 64, 64, Tick(time::TICKS_PER_SECOND));
    let pattern = dir.path().join("shot_%05d.png");
    let output = cli(&[
        os("export"),
        project.as_os_str(),
        os("--out"),
        pattern.as_os_str(),
        os("--json"),
    ]);
    assert_eq!(
        code(&output),
        0,
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let directory = dir.path().join("shot");
    assert_eq!(
        stdout_json(&output)["path"],
        directory.to_string_lossy().as_ref()
    );
    let mut frames: Vec<_> = fs::read_dir(&directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    frames.sort();
    assert_eq!(frames.len(), 30);
    assert_eq!(frames.first().unwrap(), "shot_00001.png");
    assert_eq!(frames.last().unwrap(), "shot_00030.png");
    // A second run refuses the directory, keeps it and stages nothing beside it.
    let again = cli(&[
        os("export"),
        project.as_os_str(),
        os("--out"),
        pattern.as_os_str(),
    ]);
    assert_ne!(code(&again), 0);
    assert_eq!(fs::read_dir(&directory).unwrap().count(), 30);
    // Video-only and all-scene choices refuse before any directory exists.
    for options in [
        render::OutputOptions {
            bit_rate: Some(200_000),
            ..Default::default()
        },
        render::OutputOptions {
            scene: render::SceneChoice::All,
            ..Default::default()
        },
    ] {
        let hostile = dir.path().join("hostile_%05d.png");
        let settings = serde_json::to_string(&options).unwrap();
        let refused = cli(&[
            os("export"),
            project.as_os_str(),
            os("--out"),
            hostile.as_os_str(),
            os("--settings"),
            os(&settings),
        ]);
        assert_ne!(code(&refused), 0);
        assert!(!dir.path().join("hostile").exists());
    }
    assert!(
        fs::read_dir(dir.path()).unwrap().all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".vibecut-export")),
        "no staging directory is left behind"
    );
}

#[test]
fn export_writes_a_completed_file_and_keeps_progress_on_stderr() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("overlay.vibecut");
    let output_path = dir.path().join("export.mp4");
    overlay_project(&project, 64, 64, Tick(time::TICKS_PER_SECOND));
    // A longer red main scene must not leak into the selected green scene's export.
    let mut data = project::load(&project).unwrap().project.into_data();
    let mut secondary = data.scene().clone();
    secondary.id = project::SceneId(2);
    secondary.name = "Export scene".into();
    secondary.is_main = false;
    secondary.timeline.main.id = project::TrackId(3);
    secondary
        .timeline
        .overlays
        .first_mut()
        .unwrap()
        .items
        .first_mut()
        .unwrap()
        .content = project::ItemParams::Overlay(project::OverlayParams {
        color: project::Color([0, 255, 0, 255]),
    });
    secondary.timeline.overlays.first_mut().unwrap().id = project::TrackId(4);
    secondary
        .timeline
        .overlays
        .first_mut()
        .unwrap()
        .items
        .first_mut()
        .unwrap()
        .id = project::ItemId(5);
    let item = data
        .scene_mut()
        .timeline
        .overlays
        .first_mut()
        .unwrap()
        .items
        .first_mut()
        .unwrap();
    item.source_range = TimeRange::new(Tick::ZERO, Tick(2 * time::TICKS_PER_SECOND)).unwrap();
    item.content = project::ItemParams::Overlay(project::OverlayParams {
        color: project::Color([255, 0, 0, 255]),
    });
    data.scenes.insert(secondary).unwrap();
    data.current_scene_id = project::SceneId(2);
    fs::write(
        &project,
        project::encode(&project::Project::new(data).unwrap(), false).unwrap(),
    )
    .unwrap();
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
    assert_eq!(
        stdout_json(&output)["path"],
        output_path.to_string_lossy().as_ref()
    );
    assert!(fs::metadata(&output_path).unwrap().len() > 0);
    assert!(String::from_utf8_lossy(&output.stderr).contains("progress"));
    if tools() {
        let frame = decoded_rgb(&output_path, 64, 64, &[0]);
        let rgb = &frame.first().unwrap()[..3];
        for (actual, expected) in rgb.iter().zip([0, 255, 0]) {
            assert!(
                (i32::from(*actual) - expected).abs() <= 12,
                "decoded {rgb:?} expected green"
            );
        }
        let probe = tool_command("ffprobe")
            .args([
                os("-v"),
                os("error"),
                os("-show_entries"),
                os("stream=nb_frames"),
                os("-of"),
                os("json"),
                output_path.as_os_str(),
            ])
            .output()
            .unwrap();
        let value: Value = serde_json::from_slice(&probe.stdout).unwrap();
        assert_eq!(value["streams"][0]["nb_frames"], "30");
        let before = fs::read(&project).unwrap();
        for (scope, stem, count) in [
            (render::SceneChoice::All, "all", 2),
            (
                render::SceneChoice::Selected {
                    scene: project::SceneId(0),
                },
                "main",
                1,
            ),
        ] {
            let destination = dir.path().join(format!("{stem}.mp4"));
            let settings = serde_json::to_string(&render::OutputOptions {
                scene: scope,
                resolution: Some([64, 64]),
                ..Default::default()
            })
            .unwrap();
            let output = cli(&[
                os("export"),
                project.as_os_str(),
                os("--out"),
                destination.as_os_str(),
                os("--settings"),
                os(&settings),
                os("--json"),
            ]);
            assert_eq!(
                code(&output),
                0,
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let paths = stdout_json(&output)["paths"].as_array().unwrap().clone();
            assert_eq!(paths.len(), count);
            for path in paths {
                let name = path.as_str().unwrap();
                let path = std::path::Path::new(name);
                let red = scope != render::SceneChoice::All || name.contains("-scene-0.");
                let rgb = decoded_rgb(path, 64, 64, &[0]);
                let pixel = &rgb.first().unwrap()[..3];
                for (actual, expected) in
                    pixel
                        .iter()
                        .zip(if red { [255, 0, 0] } else { [0, 255, 0] })
                {
                    assert!(
                        (i32::from(*actual) - expected).abs() <= 12,
                        "{name}: {pixel:?}"
                    );
                }
                let probe = tool_command("ffprobe")
                    .args([
                        "-v",
                        "error",
                        "-show_entries",
                        "stream=nb_frames",
                        "-of",
                        "json",
                    ])
                    .arg(path)
                    .output()
                    .unwrap();
                assert!(probe.status.success());
                let value: Value = serde_json::from_slice(&probe.stdout).unwrap();
                assert_eq!(
                    value["streams"][0]["nb_frames"],
                    if red { "60" } else { "30" }
                );
            }
            assert_eq!(fs::read(&project).unwrap(), before);
        }
    }
}

#[test]
fn export_aspect_selects_portrait_geometry() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("aspect.vibecut");
    let output_path = dir.path().join("portrait.mp4");
    overlay_project(&project, 64, 64, Tick(time::TICKS_PER_SECOND));
    let output = cli(&[
        os("export"),
        project.as_os_str(),
        os("--out"),
        output_path.as_os_str(),
        os("--aspect"),
        os("9:16"),
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
                output_path.as_os_str(),
            ])
            .output()
            .unwrap();
        assert!(probe.status.success());
        assert_eq!(String::from_utf8_lossy(&probe.stdout).trim(), "1080,1920");
    }
}

#[test]
fn export_refuses_missing_changed_and_missing_audio_sources() {
    if !tools() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let video = dir.path().join("video.mp4");
    let audio = dir.path().join("audio.wav");
    let project = dir.path().join("media.vibecut");
    let output_path = dir.path().join("media.mp4");
    generate_av(&video, 1);
    generate_audio(&audio);
    media_project(&project, &video, Some(&audio));
    let video_bytes = fs::read(&video).unwrap();

    fs::remove_file(&video).unwrap();
    let missing = cli(&[
        os("export"),
        project.as_os_str(),
        os("--out"),
        output_path.as_os_str(),
        os("--json"),
    ]);
    assert_eq!(code(&missing), 4);
    let error = error_json(&missing);
    assert_eq!(error["error"]["code"], "unavailable");
    let unavailable = error["error"]["unavailable"].as_array().unwrap();
    assert_eq!(unavailable[0]["asset"], 1);
    assert_eq!(unavailable[0]["item"], 0);
    assert_eq!(unavailable[0]["reason"], "missing");
    assert!(!output_path.exists());

    fs::write(&video, &video_bytes).unwrap();
    let mut changed_bytes = video_bytes.clone();
    changed_bytes.extend_from_slice(b"changed content");
    fs::write(&video, changed_bytes).unwrap();
    let changed = cli(&[
        os("export"),
        project.as_os_str(),
        os("--out"),
        output_path.as_os_str(),
        os("--json"),
    ]);
    assert_eq!(code(&changed), 4);
    let error = error_json(&changed);
    assert!(
        error["error"]["unavailable"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["reason"] == "changed")
    );
    assert!(!output_path.exists());

    fs::write(&video, &video_bytes).unwrap();
    fs::remove_file(&audio).unwrap();
    let missing_audio = cli(&[
        os("export"),
        project.as_os_str(),
        os("--out"),
        output_path.as_os_str(),
        os("--json"),
    ]);
    assert_eq!(code(&missing_audio), 4);
    let error = error_json(&missing_audio);
    let unavailable = error["error"]["unavailable"].as_array().unwrap();
    assert!(unavailable.iter().all(|entry| entry["asset"] == 2));
    assert!(!output_path.exists());
}

#[test]
fn export_allow_degraded_is_explicit_consent() {
    if !tools() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let video = dir.path().join("degraded.mp4");
    let project = dir.path().join("degraded.vibecut");
    let output_path = dir.path().join("degraded-out.mp4");
    generate_av(&video, 1);
    media_project(&project, &video, None);
    fs::remove_file(&video).unwrap();
    let output = cli(&[
        os("export"),
        project.as_os_str(),
        os("--out"),
        output_path.as_os_str(),
        os("--allow-degraded"),
    ]);
    assert_eq!(
        code(&output),
        0,
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(fs::metadata(&output_path).unwrap().len() > 0);
}

#[test]
fn export_clip_writes_each_mode_over_the_clip_span_with_probe() {
    if !tools() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.mp4");
    let project_path = dir.path().join("clip.vibecut");
    generate_av(&source, 1);
    media_project(&project_path, &source, None);
    for mode in ["clip", "duration"] {
        let out = dir.path().join(format!("{mode}.mp4"));
        let output = cli(&[
            os("media"),
            os("export-clip"),
            project_path.as_os_str(),
            os("--clip"),
            os("0"),
            os("--mode"),
            os(mode),
            os("--out"),
            out.as_os_str(),
        ]);
        assert_eq!(
            code(&output),
            0,
            "{mode}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        // The clip spans 0..1 s: each mode writes a file of that duration.
        let probe = tool_command("ffprobe")
            .args([
                "-v",
                "error",
                "-show_entries",
                "format=duration",
                "-of",
                "csv=p=0",
            ])
            .arg(&out)
            .output()
            .unwrap();
        assert!(probe.status.success(), "{mode}");
        let seconds: f64 = String::from_utf8_lossy(&probe.stdout)
            .trim()
            .parse()
            .unwrap();
        assert!((seconds - 1.0).abs() < 0.1, "{mode}: {seconds}");
    }
}

#[test]
fn export_clips_writes_each_full_source_once_with_deterministic_names() {
    if !tools() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.mp4");
    let project_path = dir.path().join("clips.vibecut");
    generate_av(&source, 1);
    media_project(&project_path, &source, None);
    // A second asset with the same file stem: its output must take the next free name.
    let second_dir = dir.path().join("second");
    fs::create_dir(&second_dir).unwrap();
    let second = second_dir.join("source.mp4");
    fs::copy(&source, &second).unwrap();
    let mut data = project::load(&project_path).unwrap().project.into_data();
    data.assets
        .push(media::SourceAsset::import(media::AssetId(3), second).unwrap());
    let project = project::Project::new(data).unwrap();
    fs::write(&project_path, project::encode(&project, true).unwrap()).unwrap();
    let out = dir.path().join("clips");
    fs::create_dir(&out).unwrap();

    let export = |assets: &[&str], overwrite: bool| {
        let mut args = vec![os("media"), os("export-clips"), project_path.as_os_str()];
        for asset in assets {
            args.push(os("--asset"));
            args.push(os(asset));
        }
        args.extend([os("--dir"), out.as_os_str()]);
        if overwrite {
            args.push(os("--overwrite"));
        }
        cli(&args)
    };
    let output = export(&["1", "3"], false);
    assert_eq!(
        code(&output),
        0,
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    for name in ["source.mp4", "source-2.mp4"] {
        let probe = tool_command("ffprobe")
            .args([
                "-v",
                "error",
                "-show_entries",
                "format=duration",
                "-of",
                "csv=p=0",
            ])
            .arg(out.join(name))
            .output()
            .unwrap();
        assert!(probe.status.success(), "{name}");
        let seconds: f64 = String::from_utf8_lossy(&probe.stdout)
            .trim()
            .parse()
            .unwrap();
        assert!((seconds - 1.0).abs() < 0.1, "{name}: {seconds}");
    }

    // Existing outputs are refused without --overwrite and stay byte-identical.
    let before = fs::read(out.join("source.mp4")).unwrap();
    assert_ne!(code(&export(&["1", "3"], false)), 0);
    assert_eq!(fs::read(out.join("source.mp4")).unwrap(), before);
    // A repeated asset is refused before any render starts.
    assert_ne!(code(&export(&["1", "1"], true)), 0);
}

#[test]
fn export_paints_each_clip_background_kind_outside_a_clip_that_leaves_canvas_open() {
    if !tools() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("red.mp4");
    let project = dir.path().join("background.vibecut");
    // A one-second solid red source, so the probe colour is known.
    ffmpeg(&[
        os("-f"),
        os("lavfi"),
        os("-i"),
        os("color=c=red:size=128x96:rate=30:duration=1"),
        os("-c:v"),
        os("libx264"),
        os("-pix_fmt"),
        os("yuv420p"),
        os("-color_primaries"),
        os("bt709"),
        os("-color_trc"),
        os("bt709"),
        os("-colorspace"),
        os("bt709"),
        source.as_os_str(),
    ]);
    media_project(&project, &source, None);
    // The clip shrinks to the left half, so the probe at (100, 10) lies outside it.
    for (field, value) in [
        ("transform.scaleX", 0.5),
        ("transform.scaleY", 0.5),
        ("transform.positionX", -32.0),
    ] {
        exec(
            &project,
            "timeline.set_item_param",
            json!({"item":0,"patch":{"field":field,"value":value}}),
        );
    }
    let probe = (10 * 128 + 100) * 3;
    for (kind, background) in [
        ("none", json!({"kind":"none"})),
        ("solid", json!({"kind":"solid","color":[0,0,255,255]})),
        ("blur", json!({"kind":"blur","intensity":100})),
    ] {
        exec(
            &project,
            "timeline.set_background",
            json!({"items":[0],"background":background}),
        );
        let output_path = dir.path().join(format!("{kind}.mp4"));
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
            "{kind}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let frame = &decoded_rgb(&output_path, 128, 96, &[0])[0];
        let [r, g, b] = [frame[probe], frame[probe + 1], frame[probe + 2]];
        match kind {
            "none" => assert!(r < 60 && g < 60 && b < 60, "{kind}: {r} {g} {b}"),
            "solid" => assert!(b > 180 && r < 80 && g < 80, "{kind}: {r} {g} {b}"),
            // The blur of the red source is red, so the outside is not black.
            _ => assert!(r > 120 && g < 80 && b < 80, "{kind}: {r} {g} {b}"),
        }
    }
}

#[test]
fn export_places_the_clip_under_each_fit_mode() {
    if !tools() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("red.mp4");
    let project = dir.path().join("fit.vibecut");
    // A one-second solid red 4:3 source on a black square canvas.
    crate::media_fixture::ffmpeg(&[
        os("-f"),
        os("lavfi"),
        os("-i"),
        os("color=c=red:size=128x96:rate=30:duration=1"),
        os("-c:v"),
        os("libx264"),
        os("-pix_fmt"),
        os("yuv420p"),
        os("-color_primaries"),
        os("bt709"),
        os("-color_trc"),
        os("bt709"),
        os("-colorspace"),
        os("bt709"),
        source.as_os_str(),
    ]);
    media_project(&project, &source, None);
    let exec = crate::cli::fixtures::exec;
    exec(
        &project,
        "project.set_canvas",
        serde_json::json!({"width":192,"height":192}),
    );
    // Probes: A is above the Fit letterbox (y 24..168) and above the Original
    // box (y 48..144), B lies between them, C is left of the Original box.
    let pixel = |frame: &[u8], x: usize, y: usize| {
        let at = (y * 192 + x) * 3;
        (frame[at], frame[at + 1], frame[at + 2])
    };
    let red = |(r, g, b): (u8, u8, u8)| r > 150 && g < 80 && b < 80;
    let black = |(r, g, b): (u8, u8, u8)| r < 60 && g < 60 && b < 60;
    // (mode, fields to set in order, [A, B, C] expected red)
    let rows: [(&str, &[&str], [bool; 3]); 5] = [
        ("fit", &["fit"], [false, true, true]),
        ("fill", &["fill"], [true, true, true]),
        ("stretch", &["stretch"], [true, true, true]),
        ("none", &["none"], [false, false, false]),
        // Manual after Fit records the fit, so the picture is unchanged.
        ("manual", &["fit", "manual"], [false, true, true]),
    ];
    for (name, modes, expected) in rows {
        for mode in modes {
            exec(
                &project,
                "timeline.set_item_param",
                serde_json::json!({"item":0,"patch":{"field":"fit","value":mode}}),
            );
        }
        let output_path = dir.path().join(format!("{name}.mp4"));
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
            "{name}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let frame = &decoded_rgb(&output_path, 192, 192, &[0])[0];
        for (probe, (x, y)) in [(96, 8), (96, 36), (8, 96)].into_iter().enumerate() {
            let value = pixel(frame, x, y);
            if expected[probe] {
                assert!(red(value), "{name} probe {probe}: {value:?}");
            } else {
                assert!(black(value), "{name} probe {probe}: {value:?}");
            }
        }
        // The middle of the canvas is the clip under every mode.
        assert!(red(pixel(frame, 96, 96)), "{name} centre");
    }
}

#[test]
fn export_applies_an_adjustment_layer_blur_to_what_is_below_it() {
    if !tools() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("adjustment.vibecut");
    overlay_project_color(
        &project,
        256,
        256,
        Tick(time::TICKS_PER_SECOND),
        [255, 0, 0, 255],
    );
    // A red square (64..192) on the black canvas: the blur must spill past its edge.
    for field in ["transform.scaleX", "transform.scaleY"] {
        exec(
            &project,
            "timeline.set_item_param",
            json!({"item":0,"patch":{"field":field,"value":0.5}}),
        );
    }
    let probe = (128 * 256 + 62) * 3;
    let export = |name: &str| {
        let output_path = dir.path().join(name);
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
            "{name}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        decoded_rgb(&output_path, 256, 256, &[0])[0][probe]
    };
    let sharp = export("sharp.mp4");
    let inserted = exec(
        &project,
        "timeline.insert",
        json!({
            "placement": {"mode": "auto"},
            "item": {
                "animations": {},
                "source_asset": null,
                "source_range": {"start": 0, "end": time::TICKS_PER_SECOND},
                "start": 0,
                "content": {"kind": "effect", "params": {}}
            }
        }),
    );
    let layer = inserted["placements"][0]["item"].as_u64().unwrap();
    exec(
        &project,
        "effects.add",
        json!({"item":layer,"effect":{"id":"soften","kind":"blur","enabled":true,"params":{"intensity":{"kind":"scalar","value":100.0}}}}),
    );
    let blurred = export("blurred.mp4");
    assert!(
        sharp < 60 && blurred > sharp.saturating_add(30),
        "outside the square: sharp {sharp}, blurred {blurred}"
    );
}

use super::process::{cli, code, os, stdout_json};
use crate::common::{ffmpeg, generate_av, media_project, tool_command, tools};
use serde_json::Value;
use std::path::Path;

/// ffmpeg ebur128 integrated loudness and the audio stream duration of `path`.
fn exported_audio(path: &Path) -> (f64, f64) {
    let meter = tool_command("ffmpeg")
        .args([os("-nostats"), os("-hide_banner"), os("-i")])
        .arg(path)
        .args([os("-af"), os("ebur128"), os("-f"), os("null"), os("-")])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&meter.stderr);
    let summary = &text[text.rfind("Summary:").unwrap()..];
    let lufs = summary
        .lines()
        .find(|line| line.trim_start().starts_with("I:"))
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap()
        .parse()
        .unwrap();
    let probe = tool_command("ffprobe")
        .args([
            os("-v"),
            os("error"),
            os("-select_streams"),
            os("a:0"),
            os("-show_entries"),
            os("stream=duration"),
            os("-of"),
            os("json"),
        ])
        .arg(path)
        .output()
        .unwrap();
    let value: Value = serde_json::from_slice(&probe.stdout).unwrap();
    (
        lufs,
        value["streams"][0]["duration"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap(),
    )
}

#[test]
fn normalize_stores_gains_that_export_applies_without_changing_duration() {
    if !tools() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let video = dir.path().join("video.mp4");
    let audio = dir.path().join("quiet.wav");
    let project = dir.path().join("media.vibecut");
    generate_av(&video);
    ffmpeg(&[
        os("-f"),
        os("lavfi"),
        os("-i"),
        os("sine=frequency=997:sample_rate=48000:duration=3"),
        os("-af"),
        os("volume=-12dB"),
        os("-ac"),
        os("2"),
        os("-c:a"),
        os("pcm_f32le"),
        audio.as_os_str(),
    ]);
    media_project(&project, &video, Some(&audio));
    let export = |name: &str| {
        let path = dir.path().join(name);
        let output = cli(&[
            os("export"),
            project.as_os_str(),
            os("--out"),
            path.as_os_str(),
            os("--json"),
        ]);
        assert_eq!(
            code(&output),
            0,
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        path
    };
    let (plain_lufs, plain_duration) = exported_audio(&export("plain.mp4"));
    assert!(plain_lufs < -30.0, "fixture is quiet: {plain_lufs}");

    let output = cli(&[
        os("normalize"),
        project.as_os_str(),
        os("--master"),
        os("--json"),
    ]);
    assert_eq!(
        code(&output),
        0,
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report = stdout_json(&output);
    let gain = report["master"]["gain_db"].as_f64().unwrap();
    assert!(
        (gain + plain_lufs + 14.0).abs() < 0.5,
        "gain {gain} from {plain_lufs}"
    );
    let settings = project::load(&project)
        .unwrap()
        .project
        .into_data()
        .settings;
    assert!(settings.loudness.master.is_some() && settings.loudness.measured.is_some());

    let (lufs, duration) = exported_audio(&export("normalized.mp4"));
    // -14 LUFS target; the always-on master chain adds its fixed 0.18 dB trim.
    assert!((lufs + 14.0).abs() <= 0.5, "exported {lufs} LUFS");
    assert!(
        (duration - plain_duration).abs() < 0.001,
        "{duration} vs {plain_duration}"
    );

    // Exporting audio that changed after measurement is refused until remeasured.
    let edit = cli(&[
        os("exec"),
        project.as_os_str(),
        os("timeline.set_item_param"),
        os(r#"{"item":1,"patch":{"field":"volume","value":-3.0}}"#),
    ]);
    assert_eq!(code(&edit), 0);
    let stale = cli(&[
        os("export"),
        project.as_os_str(),
        os("--out"),
        dir.path().join("stale.mp4").as_os_str(),
    ]);
    assert_ne!(code(&stale), 0);
    assert!(!dir.path().join("stale.mp4").exists());
}

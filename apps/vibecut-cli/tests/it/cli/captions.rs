use super::process::{cli, code, error_json, os, stdout_json};
use super::project_fixture::new_project;
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
    assert_eq!(cue.style.as_ref().unwrap().alignment.numpad, 2);
    assert!(cue.style.as_ref().unwrap().bold);
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

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

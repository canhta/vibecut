//! Verifies native pause removal behaviour.
use crate::cli::fixtures::new_project;
use crate::cli::process::{cli, code, os, stdout_json};
use crate::media_fixture::{tool_command, tools};
use serde_json::json;
use std::fs;
use std::path::Path;

/// 12 s of speech-like tone over a 60 Hz room tone: pauses at 2-2.8, 4.6-5.8
/// and 7.8-9.4 s plus a continuous final segment.
fn pause_audio(path: &Path) {
    let expression = "(0.25*sin(2*PI*180*t)+0.15*sin(2*PI*320*t)+0.1*sin(2*PI*520*t))*\
        (between(t\\,0\\,2)+between(t\\,2.8\\,4.6)+between(t\\,5.8\\,7.8)+between(t\\,9.4\\,12))+\
        0.01*sin(2*PI*60*t)";
    let input = format!("aevalsrc={expression}:s=16000:d=12");
    let output = tool_command("ffmpeg")
        .args(["-v", "error", "-f", "lavfi", "-i", &input])
        .args(["-c:a", "pcm_s16le", "-y"])
        .arg(path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn pause_project(project_path: &Path, audio: &Path) {
    let asset = media::SourceAsset::import(media::AssetId(1), audio.to_owned()).unwrap();
    let duration = asset.probe().duration.unwrap();
    let mut data = project::Project::empty(project::ProjectId(0), "pauses".into())
        .unwrap()
        .into_data();
    data.assets.push(asset);
    data.scene_mut().timeline.insert_track(project::Track {
        id: project::TrackId(1),
        name: "Audio".into(),
        kind: project::TrackKind::Audio,
        muted: false,
        hidden: false,
        locked: false,
        items: vec![project::Item {
            hidden: false,
            retime: Default::default(),
            blend_mode: Default::default(),
            masks: Vec::new(),
            effects: Vec::new(),
            animations: Default::default(),
            id: project::ItemId(0),
            source_asset: Some(media::AssetId(1)),
            source_range: time::TimeRange::new(time::Tick::ZERO, duration).unwrap(),
            start: time::Tick::ZERO,
            content: project::ItemParams::Audio(project::AudioParams {
                fades: Default::default(),
                volume: 0.0,
                muted: false,
                source: Default::default(),
                cleanup: Default::default(),
            }),
        }],
    });
    let project = project::Project::new(data).unwrap();
    fs::write(project_path, project::encode(&project, true).unwrap()).unwrap();
}

#[test]
fn cut_dry_run_lists_the_same_spans_and_apply_removes_them_once() {
    if !tools() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let audio = dir.path().join("pauses.wav");
    pause_audio(&audio);
    let project = dir.path().join("pauses.vibecut");
    new_project(&project);
    pause_project(&project, &audio);
    let metadata = project.with_extension("vibecut.meta.json");
    let before = fs::read(&project).unwrap();
    let metadata_before = fs::read(&metadata).unwrap();

    let dry = cli(&[
        os("cut"),
        project.as_os_str(),
        os("--remove-pauses"),
        os("--dry-run"),
        os("--json"),
    ]);
    assert_eq!(code(&dry), 0, "{}", String::from_utf8_lossy(&dry.stderr));
    let listed = stdout_json(&dry);
    let spans = listed["spans"].as_array().unwrap();
    assert_eq!(spans.len(), 3, "{listed}");
    assert_eq!(listed["changed"], json!(false));
    assert_eq!(fs::read(&project).unwrap(), before);
    assert_eq!(fs::read(&metadata).unwrap(), metadata_before);
    for span in spans {
        let start = span["start"].as_i64().unwrap();
        let end = span["end"].as_i64().unwrap();
        assert!(end > start);
    }
    // The default min pause of 0.5 s keeps a continuous segment out of the list.
    let longest = spans
        .iter()
        .map(|span| span["end"].as_i64().unwrap() - span["start"].as_i64().unwrap())
        .max()
        .unwrap();
    assert!(longest < 2 * time::TICKS_PER_SECOND);

    let applied = cli(&[
        os("cut"),
        project.as_os_str(),
        os("--remove-pauses"),
        os("--json"),
    ]);
    assert_eq!(
        code(&applied),
        0,
        "{}",
        String::from_utf8_lossy(&applied.stderr)
    );
    let result = stdout_json(&applied);
    assert_eq!(result["changed"], json!(true));
    assert_eq!(result["spans"], listed["spans"]);
    let loaded = project::load(&project).unwrap().project;
    assert!(loaded.duration() < time::Tick(12 * time::TICKS_PER_SECOND));
    let applied_bytes = fs::read(&project).unwrap();
    let applied_metadata = fs::read(&metadata).unwrap();
    assert_ne!(applied_bytes, before);

    // A threshold above every pause lists nothing, so the command reports no
    // change and writes neither project nor metadata bytes.
    let quiet = cli(&[
        os("cut"),
        project.as_os_str(),
        os("--remove-pauses"),
        os("--min-pause-ms"),
        os("5000"),
        os("--json"),
    ]);
    assert_eq!(code(&quiet), 0);
    let empty = stdout_json(&quiet);
    assert_eq!(empty["spans"], json!([]));
    assert_eq!(empty["changed"], json!(false));
    assert_eq!(fs::read(&project).unwrap(), applied_bytes);
    assert_eq!(fs::read(&metadata).unwrap(), applied_metadata);
}

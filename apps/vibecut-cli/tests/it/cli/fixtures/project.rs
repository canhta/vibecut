//! Creates project fixtures for native CLI behaviours.
use crate::cli::process::{cli, code, os, stdout_json};
use serde_json::{Value, json};
use std::ffi::OsStr;
use std::fs;
use std::path::Path;
use time::{Tick, TimeRange};

pub(crate) fn new_project(path: &Path) -> Value {
    let output = cli(&[os("project"), os("new"), path.as_os_str(), os("--json")]);
    assert_eq!(
        code(&output),
        0,
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    stdout_json(&output)
}
pub(crate) fn add_track(project: &Path, name: &str) -> Value {
    let params = json!({"name": name, "kind": "video", "placement":{"mode":"auto"}}).to_string();
    let output = cli(&[
        os("exec"),
        project.as_os_str(),
        os("timeline.add_track"),
        OsStr::new(&params),
        os("--json"),
    ]);
    assert_eq!(
        code(&output),
        0,
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    stdout_json(&output)
}
pub(crate) fn info(project: &Path) -> Value {
    let output = cli(&[os("project"), os("info"), project.as_os_str(), os("--json")]);
    assert_eq!(
        code(&output),
        0,
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    stdout_json(&output)
}
pub(crate) fn overlay_project(project_path: &Path, width: u32, height: u32, duration: Tick) {
    overlay_project_color(project_path, width, height, duration, [30, 160, 90, 255]);
}

pub(crate) fn overlay_project_color(
    project_path: &Path,
    width: u32,
    height: u32,
    duration: Tick,
    color: [u8; 4],
) {
    let mut data = project::Project::empty(project::ProjectId(0), "overlay".into())
        .unwrap()
        .into_data();
    data.canvas.width = width;
    data.canvas.height = height;
    data.scene_mut().timeline.insert_track(project::Track {
        id: project::TrackId(1),
        name: "Overlay".into(),
        kind: project::TrackKind::Video,
        muted: false,
        hidden: false,
        locked: false,
        items: vec![
            testkit::item::item(
                project::ItemId(0),
                project::ItemParams::Overlay(project::OverlayParams {
                    color: project::Color(color),
                }),
            )
            .source(TimeRange::new(Tick::ZERO, duration).unwrap())
            .build(),
        ],
    });
    let project = project::Project::new(data).unwrap();
    fs::write(project_path, project::encode(&project, true).unwrap()).unwrap();
}

pub(crate) fn exec(project: &Path, command: &str, params: Value) -> Value {
    let params = params.to_string();
    let output = cli(&[
        os("exec"),
        project.as_os_str(),
        OsStr::new(command),
        OsStr::new(&params),
        os("--json"),
    ]);
    assert_eq!(
        code(&output),
        0,
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    stdout_json(&output)
}

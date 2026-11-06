//! Verifies native probe command behaviour.
use crate::cli::process::{cli, code, os, stdout_json};
use crate::media_fixture::{generate_av, tools};

#[test]
fn probe_reports_decoded_stream_metadata() {
    if !tools() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let video = dir.path().join("probe source.mp4");
    generate_av(&video);
    let output = cli(&[os("probe"), video.as_os_str(), os("--json")]);
    assert_eq!(
        code(&output),
        0,
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value = stdout_json(&output);
    assert!(
        value["format"]
            .as_str()
            .unwrap()
            .split(',')
            .any(|format| format == "mp4")
    );
    assert!(value["duration"]["ticks"].as_i64().unwrap() > 0);
    assert!(
        value["streams"]
            .as_array()
            .unwrap()
            .iter()
            .any(|stream| stream["kind"] == "video" && stream["video"]["width"] == 128)
    );
    assert!(
        value["streams"]
            .as_array()
            .unwrap()
            .iter()
            .any(|stream| stream["kind"] == "audio")
    );
    assert_eq!(value["metadata"]["kind"], "video");
    assert_eq!(value["metadata"]["width"], 128);
    assert_eq!(value["metadata"]["height"], 96);
    assert_eq!(value["metadata"]["has_audio"], true);
    assert_eq!(value["metadata"]["frame_rate"]["num"], 30);
    assert_eq!(value["metadata"]["frame_rate"]["den"], 1);
    assert_eq!(value["metadata"]["frame_rate"]["display_rounded"], 30);
    let capabilities = value["capabilities"].as_array().unwrap();
    assert!(
        capabilities
            .iter()
            .any(|stream| stream["kind"] == "video" && stream["decodable"] == true)
    );
    assert!(
        capabilities
            .iter()
            .any(|stream| stream["kind"] == "audio" && stream["decodable"] == true)
    );
}

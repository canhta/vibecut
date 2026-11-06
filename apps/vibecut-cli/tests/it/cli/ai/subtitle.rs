//! Verifies native subtitle command behaviour.
use crate::cli::process::{bin, cli, code, error_json, os};
use std::fs;
use std::process::Command;

#[test]
fn subtitle_refuses_missing_output_and_unknown_language_without_side_effects() {
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("input.mp4");
    fs::write(&input, b"not opened before argument validation").unwrap();
    let missing = cli(&[
        os("subtitle"),
        input.as_os_str(),
        os("--to"),
        os("en"),
        os("--json"),
    ]);
    assert_eq!(code(&missing), 2);
    assert_eq!(error_json(&missing)["error"]["code"], "usage");
    let language = cli(&[
        os("subtitle"),
        input.as_os_str(),
        os("--from"),
        os("fr"),
        os("--to"),
        os("en"),
        os("--srt"),
        directory.path().join("out.srt").as_os_str(),
        os("--json"),
    ]);
    // The CLI restricts source languages to the admitted vi/en/zh set.
    assert_eq!(code(&language), 2);
    assert_eq!(error_json(&language)["error"]["code"], "usage");
}

#[test]
fn subtitle_remote_provider_without_key_refuses_before_upload() {
    let directory = tempfile::tempdir().unwrap();
    let input = directory.path().join("input.mp4");
    fs::write(&input, b"never read").unwrap();
    // A parallel CLI run must not hold this credential check's cache lease.
    let output = Command::new(bin())
        .env("HOME", directory.path())
        .env("LOCALAPPDATA", directory.path())
        .env_remove("OPENAI_API_KEY")
        .args([
            os("subtitle"),
            input.as_os_str(),
            os("--from"),
            os("en"),
            os("--to"),
            os("vi"),
            os("--provider"),
            os("openai"),
            os("--srt"),
            directory.path().join("out.srt").as_os_str(),
            os("--yes-upload"),
            os("--json"),
        ])
        .output()
        .unwrap();
    assert_eq!(
        code(&output),
        4,
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let error = error_json(&output);
    assert_eq!(error["error"]["code"], "setup");
    assert!(
        error["error"]["message"]
            .as_str()
            .unwrap()
            .contains("OPENAI_API_KEY")
    );
}

#[test]
fn auto_detection_does_not_require_cloud_translation_credentials() {
    let directory = tempfile::tempdir().unwrap();
    let home = directory.path().join("home");
    fs::create_dir_all(&home).unwrap();
    let input = directory.path().join("input.mp4");
    fs::write(&input, b"not read before model setup").unwrap();
    let output = Command::new(bin())
        .args([
            os("subtitle"),
            input.as_os_str(),
            os("--from"),
            os("auto"),
            os("--to"),
            os("en"),
            os("--provider"),
            os("whisper.cpp"),
            os("--srt"),
            directory.path().join("out.srt").as_os_str(),
            os("--json"),
        ])
        .env("HOME", &home)
        .env_remove("OPENAI_API_KEY")
        .env_remove("ANTHROPIC_API_KEY")
        .env_remove("ELEVENLABS_API_KEY")
        .output()
        .unwrap();
    assert_eq!(
        code(&output),
        4,
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let error = error_json(&output);
    assert_eq!(error["error"]["code"], "setup");
    let message = error["error"]["message"].as_str().unwrap();
    assert!(message.contains("whisper-large-v3-turbo"), "{message}");
    assert!(!message.contains("OPENAI_API_KEY"), "{message}");
    assert!(!message.contains("ANTHROPIC_API_KEY"), "{message}");
}

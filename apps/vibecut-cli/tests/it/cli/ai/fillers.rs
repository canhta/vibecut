//! Verifies native fillers command refusals before any provider work.
use crate::cli::fixtures::new_project;
use crate::cli::process::{bin, code, error_json, os, stdout_json};
use std::process::Command;

#[test]
fn fillers_validation_refuses_before_any_upload() {
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("fillers.vibecut");
    new_project(&project);

    let run = |args: &[&std::ffi::OsStr]| {
        let output = Command::new(bin())
            .env("OPENAI_API_KEY", "test-key")
            .env_remove("ANTHROPIC_API_KEY")
            .args([os("fillers"), project.as_os_str()])
            .args(args)
            .arg(os("--json"))
            .output()
            .unwrap();
        (code(&output), output)
    };

    let (exit, output) = run(&[os("--language"), os("en")]);
    assert_eq!(exit, 2, "{}", String::from_utf8_lossy(&output.stderr));
    let (exit, output) = run(&[os("--llm"), os("--language"), os("fr")]);
    assert_eq!(exit, 2, "{}", String::from_utf8_lossy(&output.stderr));
    let (exit, output) = run(&[
        os("--llm"),
        os("--language"),
        os("en"),
        os("--provider"),
        os("unknown"),
    ]);
    assert_eq!(exit, 5, "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(error_json(&output)["error"]["code"], "rejected");
}

#[test]
fn fillers_missing_credential_refuses_before_media_or_network_work() {
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("fillers.vibecut");
    new_project(&project);
    let output = Command::new(bin())
        .env_remove("OPENAI_API_KEY")
        .args([
            os("fillers"),
            project.as_os_str(),
            os("--language"),
            os("en"),
            os("--llm"),
            os("--yes-upload"),
            os("--json"),
        ])
        .output()
        .unwrap();
    assert_eq!(code(&output), 4);
    assert_eq!(error_json(&output)["error"]["code"], "setup");
}

#[test]
fn fillers_help_lists_the_native_contract() {
    let output = Command::new(bin())
        .args([os("fillers"), os("--help"), os("--json")])
        .output()
        .unwrap();
    assert_eq!(code(&output), 0);
    let usage = stdout_json(&output)["usage"].as_str().unwrap().to_owned();
    assert!(usage.contains("--language"));
    assert!(usage.contains("--llm"));
    assert!(usage.contains("--yes-upload"));
}

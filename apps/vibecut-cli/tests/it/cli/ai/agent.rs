//! Verifies the native agent command refuses before any provider request.
use crate::cli::fixtures::new_project;
use crate::cli::process::{bin, code, error_json, os, stdout_json};
use std::process::Command;

fn agent(project: &std::path::Path, key: Option<&str>, args: &[&str]) -> std::process::Output {
    let mut command = Command::new(bin());
    match key {
        Some(key) => command.env("ANTHROPIC_API_KEY", key),
        None => command.env_remove("ANTHROPIC_API_KEY"),
    };
    command
        .env_remove("OPENAI_API_KEY")
        .args([os("agent"), project.as_os_str(), os("Trim the intro.")])
        .args(args.iter().map(|arg| os(arg)))
        .arg(os("--json"))
        .output()
        .unwrap()
}

#[test]
fn agent_refuses_without_explicit_upload_consent_and_writes_nothing() {
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("agent.vibecut");
    new_project(&project);
    let before = std::fs::read(&project).unwrap();

    let output = agent(&project, Some("test-key"), &["--apply"]);
    assert_eq!(
        code(&output),
        5,
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(error_json(&output)["error"]["code"], "rejected");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("agent consent"), "{stderr}");
    assert!(stderr.contains("--yes-upload"), "{stderr}");
    assert_eq!(std::fs::read(&project).unwrap(), before);
}

#[test]
fn agent_flag_and_credential_errors_come_before_any_request() {
    let root = tempfile::tempdir().unwrap();
    let project = root.path().join("agent.vibecut");
    new_project(&project);

    let output = agent(&project, Some("test-key"), &["--provider", "unknown"]);
    assert_eq!(code(&output), 2);
    let output = agent(&project, Some("test-key"), &["--max-spend", "0"]);
    assert_eq!(code(&output), 2);
    let output = agent(
        &project,
        Some("test-key"),
        &["--max-steps", "0", "--yes-upload"],
    );
    assert_eq!(
        code(&output),
        2,
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let output = agent(&project, None, &["--yes-upload"]);
    assert_eq!(code(&output), 4);
    assert_eq!(error_json(&output)["error"]["code"], "setup");
}

#[test]
fn agent_help_lists_the_native_contract() {
    let output = Command::new(bin())
        .args([os("agent"), os("--help"), os("--json")])
        .output()
        .unwrap();
    assert_eq!(code(&output), 0);
    let usage = stdout_json(&output)["usage"].as_str().unwrap().to_owned();
    assert!(usage.contains("--apply"));
    assert!(usage.contains("--yes-upload"));
    assert!(usage.contains("--max-spend"));
}

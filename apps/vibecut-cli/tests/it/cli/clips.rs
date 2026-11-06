use super::process::{bin, cli, code, error_json, os, stdout_json};
use std::ffi::OsStr;
use std::fs;
use std::process::Command;

#[test]
fn clips_validation_refuses_before_media_or_network_work() {
    let root = tempfile::tempdir().unwrap();
    let missing = root.path().join("missing.mp4");
    let export = root.path().join("exports");
    fs::create_dir_all(&export).unwrap();
    let cases: &[(&[&str], i32, Option<&str>)] = &[
        (&["--export"], 2, None),
        (&["--subs", "vi"], 2, None),
        (&["--select", "0"], 2, None),
        (&["--provider", "unknown"], 5, Some("rejected")),
        (&["--min", "0s"], 2, None),
        (
            &["--export", "--subs", "vi", "--aspect", "16:9"],
            5,
            Some("unsupported"),
        ),
        (&["--export", "--subs", "vi", "--select", "0"], 2, None),
    ];
    for (flags, expected_code, expected_kind) in cases {
        let mut args: Vec<&OsStr> = vec![os("clips"), missing.as_os_str()];
        for flag in *flags {
            match *flag {
                "--export" => {
                    args.push(os("--export"));
                    args.push(export.as_os_str());
                }
                other => args.push(os(other)),
            }
        }
        args.push(os("--json"));
        let output = cli(&args);
        assert_eq!(
            code(&output),
            *expected_code,
            "{flags:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        if let Some(kind) = expected_kind {
            assert_eq!(error_json(&output)["error"]["code"], *kind, "{flags:?}");
        }
    }
}

#[test]
fn clips_missing_suggestion_credential_refuses_before_media_work() {
    let root = tempfile::tempdir().unwrap();
    let missing = root.path().join("missing.mp4");
    let output = Command::new(bin())
        .env_remove("OPENAI_API_KEY")
        .args([
            os("clips"),
            missing.as_os_str(),
            os("--provider"),
            os("openai"),
            os("--yes-upload"),
            os("--json"),
        ])
        .output()
        .unwrap();
    assert_eq!(code(&output), 4);
    let error = error_json(&output);
    assert_eq!(error["error"]["code"], "setup");
    assert!(
        error["error"]["message"]
            .as_str()
            .is_some_and(|value| value.contains("OPENAI_API_KEY"))
    );
}

#[test]
fn clips_provider_catalog_is_explicit_about_local_and_remote() {
    let output = cli(&[
        os("providers"),
        os("--operation"),
        os("clips"),
        os("--json"),
    ]);
    assert_eq!(
        code(&output),
        0,
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let providers = stdout_json(&output)["providers"]
        .as_array()
        .unwrap()
        .clone();
    let heuristic = providers
        .iter()
        .find(|value| value["id"] == "heuristic")
        .unwrap();
    assert_eq!(heuristic["location"], "local");
    assert!(heuristic["policy_url"].is_null());
    for remote in ["openai", "anthropic"] {
        let value = providers
            .iter()
            .find(|value| value["id"] == remote)
            .unwrap();
        assert_eq!(value["location"], "remote");
        assert!(value["policy_url"].is_string());
    }
}

//! Verifies native model command behaviour.
use crate::cli::process::{cli, code, error_json, os, stdout_json};

#[test]
fn model_catalog_is_offline_and_download_requires_explicit_consent() {
    let list = cli(&[os("model"), os("list"), os("--json")]);
    assert_eq!(code(&list), 0, "{}", String::from_utf8_lossy(&list.stderr));
    let value = stdout_json(&list);
    let models = value["models"].as_array().unwrap();
    assert!(
        models
            .iter()
            .any(|model| model["id"] == "whisper-large-v3-turbo")
    );
    assert!(
        models
            .iter()
            .any(|model| model["id"] == "whisper-small-dtw")
    );
    let info = cli(&[
        os("model"),
        os("info"),
        os("whisper-large-v3-turbo"),
        os("--json"),
    ]);
    assert_eq!(code(&info), 0);
    let value = stdout_json(&info);
    assert!(
        value["license_url"]
            .as_str()
            .is_some_and(|v| v.starts_with("https://"))
    );
    assert!(!value["files"].as_array().unwrap().is_empty());
    let refused = cli(&[
        os("model"),
        os("download"),
        os("whisper-large-v3-turbo"),
        os("--json"),
    ]);
    assert_eq!(code(&refused), 5);
    let error = error_json(&refused);
    assert_eq!(error["error"]["code"], "rejected");
}

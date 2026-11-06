use super::process::{cli, code, os, stdout_json};

#[test]
fn providers_list_is_offline_and_capability_filtered() {
    let output = cli(&[
        os("providers"),
        os("--operation"),
        os("transcribe"),
        os("--language"),
        os("vi"),
        os("--json"),
    ]);
    assert_eq!(
        code(&output),
        0,
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value = stdout_json(&output);
    let providers = value["providers"].as_array().unwrap();
    assert!(providers.iter().any(|value| value["id"] == "whisper.cpp"));
    assert!(providers.iter().any(|value| value["id"] == "gipformer"));
    assert!(providers.iter().any(|value| value["id"] == "openai"));
    assert!(providers.iter().any(|value| value["id"] == "elevenlabs"));
    let english = cli(&[
        os("providers"),
        os("--operation"),
        os("transcribe"),
        os("--language"),
        os("en"),
        os("--json"),
    ]);
    assert_eq!(code(&english), 0);
    let value = stdout_json(&english);
    assert!(
        !value["providers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value["id"] == "gipformer")
    );
}

//! Verifies native CLI catalog behaviour.
use crate::cli::process::{cli, code, os, stdout_json};
use std::fs;
use std::path::Path;

#[test]
fn commands_prints_the_generated_registry() {
    let output = cli(&[os("commands"), os("--json")]);
    assert_eq!(code(&output), 0);
    let value = stdout_json(&output);
    assert_eq!(value["api_version"], engine::control::API_VERSION);
    assert_eq!(value["registry_hash"], engine::control::registry_hash());
    assert_eq!(
        value["limits"],
        serde_json::to_value(engine::control::LIMITS).unwrap()
    );
    let entries = value["entries"].as_array().unwrap();
    let add_track = entries
        .iter()
        .find(|entry| entry["id"] == "timeline.add_track")
        .unwrap();
    assert_eq!(
        add_track["params_schema"]["$defs"]["NfcText"]["type"],
        "string"
    );
    assert!(entries.iter().any(|entry| entry["id"] == "batch"));
}

#[test]
fn generated_catalog_is_deterministic_and_matches_checked_in_output() {
    let first = cli(&[os("commands"), os("--json")]);
    let second = cli(&[os("commands"), os("--json")]);
    assert_eq!(code(&first), 0);
    assert_eq!(code(&second), 0);
    assert_eq!(
        first.stdout, second.stdout,
        "catalog generation must be byte-identical twice"
    );
    let value = stdout_json(&first);
    let keys = value
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(keys, ["api_version", "entries", "limits", "registry_hash"]);
    assert_eq!(
        value["entries"],
        serde_json::to_value(engine::registry()).unwrap()
    );
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let digest = fs::read_to_string(root.join("../../crates/engine/catalog.digest")).unwrap();
    assert_eq!(value["registry_hash"].as_str().unwrap(), digest.trim());
    assert_eq!(
        fs::read_to_string(root.join("../../docs/engineering/commands.md"))
            .unwrap()
            .replace("\r\n", "\n"),
        engine::control::reference_markdown().unwrap()
    );

    let first_human = cli(&[os("commands")]);
    let second_human = cli(&[os("commands")]);
    assert_eq!(code(&first_human), 0);
    assert_eq!(first_human.stdout, second_human.stdout);
    let text = String::from_utf8_lossy(&first_human.stdout);
    assert!(text.contains(engine::control::registry_hash()));
}

#[test]
fn schema_prints_the_generated_contract_envelope() {
    let first = cli(&[os("schema"), os("--json")]);
    let second = cli(&[os("schema"), os("--json")]);
    assert_eq!(code(&first), 0);
    assert_eq!(code(&second), 0);
    assert_eq!(first.stdout, second.stdout);
    let value = stdout_json(&first);
    assert_eq!(value["api_version"], engine::control::API_VERSION);
    assert_eq!(value["registry_hash"], engine::control::registry_hash());
    assert_eq!(value["contract_schema"], engine::control::contract_schema());
    assert!(value["contract_schema"]["request"].is_object());
    assert!(value["contract_schema"]["response"].is_object());
    assert!(value["contract_schema"]["event"].is_object());
}

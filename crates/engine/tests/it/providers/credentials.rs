//! Provider credential selection stays independent of unrelated environment values.
#![allow(
    clippy::unwrap_used,
    clippy::panic,
    reason = "isolated synthetic credential fixtures"
)]
use engine::EditError;
use engine::providers::{Credentials, Error};
use std::process::Command;

const PROVIDER: &str = "VIBECUT_CREDENTIAL_TEST_PROVIDER";
const EXPECTED: &str = "VIBECUT_CREDENTIAL_TEST_EXPECTED";
const KEYS: [&str; 3] = ["OPENAI_API_KEY", "ELEVENLABS_API_KEY", "ANTHROPIC_API_KEY"];

#[test]
fn selected_environment_credential_ignores_unrelated_invalid_values() {
    if let Ok(provider) = std::env::var(PROVIDER) {
        let credentials = Credentials::from_env();
        let result = match provider.as_str() {
            "OPENAI_API_KEY" => credentials.openai().map(|_| ()),
            "ELEVENLABS_API_KEY" => credentials.elevenlabs().map(|_| ()),
            "ANTHROPIC_API_KEY" => credentials.anthropic().map(|_| ()),
            _ => panic!("unknown synthetic provider"),
        };
        match std::env::var(EXPECTED).unwrap().as_str() {
            "valid" => result.unwrap(),
            "missing" => assert!(matches!(
                result,
                Err(EditError::Providers(Error::CredentialMissing(name))) if name == provider
            )),
            "invalid" => assert!(matches!(
                result,
                Err(EditError::Ai(ai::Error::Invalid("API key")))
            )),
            _ => panic!("unknown synthetic expectation"),
        }
        return;
    }
    // Child-only synthetic values avoid process-global environment mutation and
    // replace every provider key; no real credential reaches an assertion or log.
    for provider in KEYS {
        for (value, expected) in [
            (Some("synthetic-key"), "valid"),
            (None, "missing"),
            (Some(""), "invalid"),
            (Some("synthetic-key "), "invalid"),
        ] {
            let mut child = Command::new(std::env::current_exe().unwrap());
            child.args([
                "--exact",
                "providers::credentials::selected_environment_credential_ignores_unrelated_invalid_values",
            ]);
            child.env(PROVIDER, provider).env(EXPECTED, expected);
            for (index, key) in KEYS.into_iter().enumerate() {
                child.env(
                    key,
                    if index == 0 {
                        "synthetic-unrelated "
                    } else {
                        ""
                    },
                );
            }
            if let Some(value) = value {
                child.env(provider, value);
            } else {
                child.env_remove(provider);
            }
            let output = child.output().unwrap();
            assert!(
                output.status.success(),
                "{provider}/{expected}: {}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
}

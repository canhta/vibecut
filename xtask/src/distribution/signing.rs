//! Signing, notarization and credential lifecycle for macOS distribution artifacts.
use super::package;
use super::{Host, Result, Tools, arg, field, fresh, text, write_json};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

const REQUIRED: [&str; 6] = [
    "MACOS_CERT_P12",
    "MACOS_CERT_PASSWORD",
    "APPLE_API_KEY_P8",
    "APPLE_API_KEY_ID",
    "APPLE_API_ISSUER",
    "APPLE_TEAM_ID",
];
pub(super) enum Notary {
    Profile(String),
    Key {
        file: PathBuf,
        id: String,
        issuer: String,
    },
}
pub(super) struct Config {
    pub identity: String,
    pub installer: Option<String>,
    pub notary: Option<Notary>,
}
impl Config {
    pub fn unsigned() -> Self {
        Self {
            identity: "-".into(),
            installer: None,
            notary: None,
        }
    }
    pub fn environment(tools: &mut Tools) -> Result<Self> {
        let identity = tools
            .env
            .get("VIBECUT_MAC_SIGN_IDENTITY")
            .cloned()
            .unwrap_or_else(|| "-".into());
        let installer = tools.env.get("VIBECUT_MAC_INSTALLER_IDENTITY").cloned();
        let notary = if let Some(profile) = tools.env.get("VIBECUT_NOTARY_PROFILE") {
            Some(Notary::Profile(profile.clone()))
        } else if let Some(file) = tools.env.get("VIBECUT_NOTARY_KEY_FILE") {
            Some(Notary::Key {
                file: PathBuf::from(file),
                id: tools
                    .env
                    .get("VIBECUT_NOTARY_KEY_ID")
                    .cloned()
                    .ok_or("notary key ID missing")?,
                issuer: tools
                    .env
                    .get("VIBECUT_NOTARY_ISSUER_ID")
                    .cloned()
                    .ok_or("notary issuer missing")?,
            })
        } else {
            None
        };
        if identity == "-" && (installer.is_some() || notary.is_some()) {
            return Err("installer signing/notarization needs an app signing identity".into());
        }
        tools.secrets.push(identity.clone());
        Ok(Self {
            identity,
            installer,
            notary,
        })
    }
}
fn validate(env: &BTreeMap<String, String>) -> Result<()> {
    let missing: Vec<_> = REQUIRED
        .iter()
        .filter(|key| !env.get(**key).is_some_and(|value| !value.is_empty()))
        .copied()
        .collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!("missing release secrets: {}", missing.join(", ")))
    }
}
fn state(tools: &Tools) -> PathBuf {
    tools.runner_temp.join("vibecut-signing")
}
fn private_file(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
        .open(path)
        .map_err(|e| e.to_string())?
        .write_all(bytes)
        .map_err(|e| e.to_string())
}
fn decode_base64(value: &str) -> Result<Vec<u8>> {
    if !value.len().is_multiple_of(4) || value.len() > 16 * 1024 * 1024 {
        return Err("invalid certificate encoding".into());
    }
    let mut output = Vec::new();
    let (blocks, _) = value.as_bytes().as_chunks::<4>();
    let blocks = blocks.iter();
    let count = blocks.len();
    for (index, block) in blocks.enumerate() {
        let mut bits = 0_u32;
        let mut padding = 0;
        for (offset, b) in block.iter().copied().enumerate() {
            let value = match b {
                b'A'..=b'Z' => b - b'A',
                b'a'..=b'z' => b - b'a' + 26,
                b'0'..=b'9' => b - b'0' + 52,
                b'+' => 62,
                b'/' => 63,
                b'=' if index + 1 == count && offset >= 2 => {
                    padding += 1;
                    0
                }
                _ => return Err("invalid certificate encoding".into()),
            };
            if padding > 0 && b != b'=' {
                return Err("invalid certificate padding".into());
            }
            bits = (bits << 6) | u32::from(value);
        }
        if padding > 2 {
            return Err("invalid certificate padding".into());
        }
        let bytes = [(bits >> 16) as u8, (bits >> 8) as u8, bits as u8];
        output.extend_from_slice(bytes.get(..3 - padding).ok_or("certificate bounds")?);
    }
    Ok(output)
}
fn quoted(output: &str) -> Vec<String> {
    output
        .split('"')
        .enumerate()
        .filter(|(i, _)| i % 2 == 1)
        .map(|(_, value)| value.to_owned())
        .collect()
}
fn cleanup(tools: &mut Tools) -> Result<()> {
    let directory = state(tools);
    if !directory.exists() {
        return Ok(());
    }
    let mut failure = None;
    let search = directory.join("search-list.json");
    if search.exists() {
        let restored = (|| {
            let previous: Vec<String> =
                serde_json::from_str(&text(&search)?).map_err(|e| e.to_string())?;
            let args: Vec<_> = ["list-keychains", "-d", "user", "-s"]
                .into_iter()
                .map(arg)
                .chain(previous.iter().map(arg))
                .collect();
            tools.run("security", args).map(|_| ())
        })();
        if let Err(error) = restored {
            failure = Some(error);
        }
    }
    let keychain = directory.join("release.keychain-db");
    if keychain.exists()
        && let Err(error) = run!(tools, "security", "delete-keychain", &keychain)
    {
        failure = Some(failure.unwrap_or(error));
    }
    let removed = fs::remove_dir_all(&directory).map_err(|e| e.to_string());
    if let Some(failure) = failure {
        Err(failure)
    } else {
        removed
    }
}
fn with_credentials(
    tools: &mut Tools,
    operation: impl FnOnce(&mut Tools, &Config) -> Result<()>,
) -> Result<()> {
    validate(&tools.env)?;
    if tools.host != Host::Mac {
        return Err("Developer ID signing requires macOS".into());
    }
    let directory = fresh(&state(tools))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))
            .map_err(|e| e.to_string())?;
    }
    let outcome = (|| {
        let keychain = directory.join("release.keychain-db");
        let cert = directory.join("certificate.p12");
        let key = directory.join("notary.p8");
        private_file(
            &cert,
            &decode_base64(
                tools
                    .env
                    .get("MACOS_CERT_P12")
                    .ok_or("missing certificate")?,
            )?,
        )?;
        private_file(
            &key,
            tools
                .env
                .get("APPLE_API_KEY_P8")
                .ok_or("missing notary key")?
                .as_bytes(),
        )?;
        let mut random = [0_u8; 32];
        fs::File::open("/dev/urandom")
            .and_then(|mut file| file.read_exact(&mut random))
            .map_err(|e| e.to_string())?;
        let password = random
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        tools.secrets.push(password.clone());
        let previous = quoted(&run!(tools, "security", "list-keychains", "-d", "user")?);
        write_json(&directory.join("search-list.json"), &json!(previous))?;
        run!(
            tools,
            "security",
            "create-keychain",
            "-p",
            &password,
            &keychain
        )?;
        run!(
            tools,
            "security",
            "set-keychain-settings",
            "-lut",
            "21600",
            &keychain
        )?;
        run!(
            tools,
            "security",
            "unlock-keychain",
            "-p",
            &password,
            &keychain
        )?;
        let mut search = vec![
            arg("list-keychains"),
            arg("-d"),
            arg("user"),
            arg("-s"),
            arg(&keychain),
        ];
        search.extend(previous.iter().map(arg));
        tools.run("security", search)?;
        let cert_password = tools
            .env
            .get("MACOS_CERT_PASSWORD")
            .cloned()
            .ok_or("certificate password missing")?;
        run!(
            tools,
            "security",
            "import",
            &cert,
            "-k",
            &keychain,
            "-P",
            &cert_password,
            "-T",
            "/usr/bin/codesign",
            "-T",
            "/usr/bin/security"
        )?;
        fs::remove_file(&cert).map_err(|e| e.to_string())?;
        run!(
            tools,
            "security",
            "set-key-partition-list",
            "-S",
            "apple-tool:,apple:,codesign:",
            "-s",
            "-k",
            &password,
            &keychain
        )?;
        let output = run!(
            tools,
            "security",
            "find-identity",
            "-v",
            "-p",
            "codesigning",
            &keychain
        )?;
        let team = tools.env.get("APPLE_TEAM_ID").ok_or("team missing")?;
        let matches: Vec<_> = quoted(&output)
            .into_iter()
            .filter(|identity| {
                identity.starts_with("Developer ID Application: ")
                    && identity.ends_with(&format!("({team})"))
            })
            .collect();
        if matches.len() != 1 {
            return Err("certificate must provide exactly one Developer ID Application identity for APPLE_TEAM_ID".into());
        }
        let identity = matches.first().ok_or("signing identity")?.clone();
        tools.secrets.push(identity.clone());
        let config = Config {
            identity,
            installer: None,
            notary: Some(Notary::Key {
                file: key,
                id: tools.env.get("APPLE_API_KEY_ID").cloned().ok_or("key ID")?,
                issuer: tools.env.get("APPLE_API_ISSUER").cloned().ok_or("issuer")?,
            }),
        };
        operation(tools, &config)
    })();
    let cleaned = cleanup(tools);
    outcome.and(cleaned)
}
pub(super) fn package(tools: &mut Tools, request: &package::Request) -> Result<()> {
    with_credentials(tools, |tools, config| {
        package::package(tools, request, config)
    })
}
pub(super) fn notarize(tools: &mut Tools, path: &Path, config: &Config) -> Result<()> {
    let mut credentials = Vec::new();
    match config
        .notary
        .as_ref()
        .ok_or("notarization credentials missing")?
    {
        Notary::Profile(profile) => credentials.extend([arg("--keychain-profile"), arg(profile)]),
        Notary::Key { file, id, issuer } => credentials.extend([
            arg("--key"),
            arg(file),
            arg("--key-id"),
            arg(id),
            arg("--issuer"),
            arg(issuer),
        ]),
    };
    let mut args = vec![arg("notarytool"), arg("submit"), arg(path)];
    args.extend(credentials.clone());
    args.extend([arg("--wait"), arg("--output-format"), arg("json")]);
    let mut command = Command::new("xcrun");
    command.args(args).current_dir(&tools.root);
    let result = tools.execute(&mut command, Duration::from_secs(1800))?;
    let output: Value = serde_json::from_str(&result.stdout).unwrap_or(Value::Null);
    if result.code == Some(0) && field(&output, "/status") == "Accepted" {
        eprintln!("notarytool: Accepted");
        return Ok(());
    }
    let id = field(&output, "/id").as_str().unwrap_or("");
    let log = if id.len() == 36
        && id
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() || byte == b'-')
    {
        let mut command = Command::new("xcrun");
        command.args([arg("notarytool"), arg("log"), arg(id)]);
        command.args(credentials).args(["--output-format", "json"]);
        command.current_dir(&tools.root);
        tools
            .execute(&mut command, Duration::from_secs(180))
            .ok()
            .filter(|captured| captured.code == Some(0))
            .and_then(|captured| serde_json::from_str::<Value>(&captured.stdout).ok())
            .unwrap_or(Value::Null)
    } else {
        Value::Null
    };
    Err(format!(
        "notarization refused (exit {:?}; {}); private submission details suppressed",
        result.code,
        notary_diagnostic(&log)
    ))
}
fn notary_diagnostic(log: &Value) -> String {
    // Emit only fixed categories and numeric codes, never Apple's free text,
    // submission identifiers, signing identities, paths or signed URLs.
    let mut categories = BTreeSet::new();
    let messages = std::iter::once(field(log, "/statusSummary")).chain(
        log.get("issues")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|issue| field(issue, "/message")),
    );
    for message in messages {
        let message = message.as_str().unwrap_or("").to_ascii_lowercase();
        for (needle, category) in [
            ("not yet configured for notarization", "team-not-configured"),
            ("developer id", "developer-id-certificate"),
            ("signature", "code-signature"),
            ("hardened runtime", "hardened-runtime"),
            ("timestamp", "secure-timestamp"),
            ("sdk", "sdk-version"),
        ] {
            if message.contains(needle) {
                categories.insert(category);
            }
        }
    }
    let code = log
        .get("statusCode")
        .and_then(Value::as_i64)
        .map_or_else(|| "unknown".to_owned(), |code| code.to_string());
    let categories = if categories.is_empty() {
        "unclassified".to_owned()
    } else {
        categories.into_iter().collect::<Vec<_>>().join(",")
    };
    format!("Apple statusCode {code}; categories {categories}")
}
pub(super) fn run(tools: &mut Tools, args: &[String]) -> Result<()> {
    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["validate"] => with_credentials(tools, |_, _| Ok(())),
        ["cleanup"] => cleanup(tools),
        ["sign", prefix, binaries, output, arch] => package(
            tools,
            &package::Request {
                os: "macos".into(),
                arch: (*arch).into(),
                prefix: prefix.into(),
                binaries: binaries.into(),
                output: output.into(),
            },
        ),
        _ => Err("usage: dist signing validate|cleanup|sign PREFIX BINARIES OUTPUT ARCH".into()),
    }
}
#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod tests {
    use super::{
        Config, Notary, REQUIRED, decode_base64, notarize, state, validate, with_credentials,
    };
    use crate::distribution::{Captured, Host, Temp, Tools};
    use serde_json::json;
    use std::collections::BTreeMap;
    use std::env;
    use std::ffi::OsStr;
    use std::path::Path;
    #[test]
    fn rejected_submission_fetches_only_safe_diagnostic_categories() {
        let mut tools = Tools::new(Path::new("."));
        tools.runner = Box::new(|command, _| {
            let log = command.get_args().nth(1) == Some(OsStr::new("log"));
            Ok(Captured {
                code: Some(if log { 0 } else { 68 }),
                stdout: if log {
                    json!({"statusCode":7000,"statusSummary":"Team is not yet configured for notarization PRIVATE-TEAM","issues":[{"path":"PRIVATE-PATH","message":"PRIVATE-MESSAGE"}]}).to_string()
                } else {
                    json!({"id":"00000000-0000-0000-0000-000000000001","status":"Invalid","private":"PRIVATE-SUBMISSION"}).to_string()
                },
                stderr: "PRIVATE-STDERR".into(),
            })
        });
        let config = Config {
            identity: "PRIVATE-IDENTITY".into(),
            installer: None,
            notary: Some(Notary::Profile("PRIVATE-PROFILE".into())),
        };
        let error = notarize(&mut tools, Path::new("PRIVATE-ARCHIVE"), &config).unwrap_err();
        assert!(error.contains("7000"));
        assert!(error.contains("team-not-configured"));
        assert!(!error.contains("PRIVATE"));
        assert_eq!(tools.commands.len(), 2);
    }
    #[test]
    fn missing_secrets_name_fields_without_values() {
        let env = BTreeMap::from([("APPLE_API_KEY_P8".into(), "synthetic private key".into())]);
        let message = validate(&env).expect_err("missing");
        assert!(message.contains("MACOS_CERT_P12"));
        assert!(!message.contains("synthetic private"));
    }
    #[test]
    fn certificate_decode_refuses_malformed_input() {
        assert_eq!(decode_base64("Y2VydA==").expect("base64"), b"cert");
        for (encoded, decoded) in [
            ("Zg==", "f"),
            ("Zm8=", "fo"),
            ("Zm9v", "foo"),
            ("Zm9vYg==", "foob"),
            ("Zm9vYmE=", "fooba"),
            ("Zm9vYmFy", "foobar"),
        ] {
            assert_eq!(
                decode_base64(encoded).expect("RFC vector"),
                decoded.as_bytes()
            );
        }
        assert_eq!(
            decode_base64("AP8A").expect("binary vector"),
            vec![0, 255, 0]
        );
        for input in ["invalid!", "=AAA", "AA=A", "AAAA=AAA", "A==="] {
            assert!(decode_base64(input).is_err(), "{input}");
        }
    }
    #[test]
    fn invalid_certificate_removes_private_directory() {
        let temp = Temp::new(&env::temp_dir()).expect("temp");
        let mut tools = Tools::new(&temp.0);
        tools.runner_temp = temp.0.clone();
        tools.host = Host::Mac;
        for key in REQUIRED {
            tools.env.insert(key.into(), "synthetic".into());
        }
        assert!(with_credentials(&mut tools, |_, _| Ok(())).is_err());
        assert!(!state(&tools).exists());
    }
    #[test]
    fn redaction_covers_team_identity_and_generated_password() {
        let mut tools = Tools::new(Path::new("."));
        tools
            .env
            .insert("APPLE_TEAM_ID".into(), "TESTTEAM42".into());
        tools.secrets.extend([
            "Developer ID Application: Test (TESTTEAM42)".into(),
            "generated password".into(),
        ]);
        let safe = tools.redact("Developer ID Application: Test (TESTTEAM42) generated password");
        assert!(!safe.contains("TESTTEAM42"));
        assert!(!safe.contains("generated password"));
        tools.secrets.push("-".into());
        assert_eq!(
            tools.redact("codesign --sign - /path/with-hyphens"),
            "codesign --sign - /path/with-hyphens"
        );
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::panic, clippy::indexing_slicing)]
mod cleanup_tests {
    #[cfg(unix)]
    use super::{REQUIRED, with_credentials};
    use super::{cleanup, fresh, state};
    #[cfg(unix)]
    use crate::distribution::Host;
    use crate::distribution::{Captured, Temp, Tools};
    use std::env;
    use std::ffi::OsStr;
    use std::fs;

    #[test]
    fn cleanup_removes_keys_even_when_search_restore_fails_or_is_corrupt() {
        for corrupt in [false, true] {
            let temp = Temp::new(&env::temp_dir()).expect("temp");
            let mut tools = Tools::new(&temp.0);
            tools.runner_temp = temp.0.clone();
            let directory = fresh(&state(&tools)).expect("state");
            fs::write(
                directory.join("search-list.json"),
                if corrupt {
                    "not JSON"
                } else {
                    "[\"/previous/login.keychain-db\"]"
                },
            )
            .expect("search list");
            fs::write(directory.join("release.keychain-db"), b"fixture").expect("keychain");
            fs::write(directory.join("notary.p8"), b"synthetic private key").expect("key");
            tools.runner = Box::new(|command, _| {
                let delete = command.get_args().next() == Some(OsStr::new("delete-keychain"));
                Ok(Captured {
                    code: Some(if delete { 0 } else { 1 }),
                    stdout: String::new(),
                    stderr: "synthetic sensitive diagnostic".into(),
                })
            });
            assert!(cleanup(&mut tools).is_err());
            assert!(!directory.exists());
            assert!(
                tools
                    .commands
                    .iter()
                    .any(|args| args.iter().any(|arg| arg == "delete-keychain"))
            );
            cleanup(&mut tools).expect("idempotent cleanup");
        }
    }
    #[cfg(unix)]
    #[test]
    fn signing_failure_always_restores_search_list_and_removes_private_material() {
        let temp = Temp::new(&env::temp_dir()).expect("temp");
        let mut tools = Tools::new(&temp.0);
        tools.host = Host::Mac;
        tools.runner_temp = temp.0.clone();
        for name in REQUIRED {
            tools.env.insert(name.into(), "synthetic".into());
        }
        tools.env.insert("MACOS_CERT_P12".into(), "Y2VydA==".into());
        tools
            .env
            .insert("APPLE_TEAM_ID".into(), "TESTTEAM42".into());
        tools.runner = Box::new(|command, _| {
            let args: Vec<_> = command
                .get_args()
                .map(|a| a.to_string_lossy().into_owned())
                .collect();
            if args.first().is_some_and(|a| a == "create-keychain") {
                fs::write(Path::new(args.last().expect("keychain")), b"fixture").expect("keychain");
            }
            let stdout = if args.first().is_some_and(|a| a == "find-identity") {
                "1) 1234 \"Developer ID Application: Synthetic (TESTTEAM42)\""
            } else if args == ["list-keychains", "-d", "user"] {
                "\"/previous/login.keychain-db\""
            } else {
                ""
            };
            Ok(Captured {
                code: Some(0),
                stdout: stdout.into(),
                stderr: String::new(),
            })
        });
        let result = with_credentials(&mut tools, |_, _| Err("signed packaging failed".into()));
        assert_eq!(result, Err("signed packaging failed".into()));
        assert!(!state(&tools).exists());
        let commands = serde_json::to_string(&tools.commands).expect("commands");
        assert!(!commands.contains("TESTTEAM42"));
        assert!(!commands.contains("Developer ID Application: Synthetic"));
        assert!(commands.contains("delete-keychain"));
        assert!(commands.contains("/previous/login.keychain-db"));
    }
}

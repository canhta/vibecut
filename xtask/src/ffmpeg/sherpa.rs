//! Windows Sherpa static libraries matching Rust's dynamic MSVC CRT.
use super::archives::{cache, sha256};
use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const LIBRARIES: &[&str] = &[
    "sherpa-onnx-c-api",
    "sherpa-onnx-core",
    "kaldi-decoder-core",
    "sherpa-onnx-kaldifst-core",
    "sherpa-onnx-fstfar",
    "sherpa-onnx-fst",
    "kaldi-native-fbank-core",
    "kissfft-float",
    "onnxruntime",
    "ssentencepiece_core",
    "utf8proc",
];

/// Provision the locked Sherpa version's upstream Windows /MD archive.
/// Supports x86_64/aarch64 on any host; native workspace callers use it only on Windows.
/// A complete cached receipt needs no network. Incomplete caches fail with recovery guidance.
/// The supplied transport writes the destination and applies its optional byte cap.
pub fn provision(
    root: &Path,
    arch: &str,
    mut fetch: impl FnMut(&str, &Path, Option<u64>) -> Result<(), String>,
) -> Result<PathBuf, String> {
    let platform = match arch {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        _ => return Err("Sherpa Windows archives support only x86_64 and aarch64".into()),
    };
    let lock: toml::Table = fs::read_to_string(root.join("Cargo.lock"))
        .map_err(|e| e.to_string())?
        .parse()
        .map_err(|e| format!("invalid Cargo.lock: {e}"))?;
    let mut packages = lock
        .get("package")
        .and_then(toml::Value::as_array)
        .ok_or("Cargo.lock has no packages")?
        .iter()
        .filter(|p| p.get("name").and_then(toml::Value::as_str) == Some("sherpa-onnx-sys"));
    let package = packages.next().ok_or("Cargo.lock has no sherpa-onnx-sys")?;
    if packages.next().is_some() {
        return Err("Cargo.lock must contain exactly one sherpa-onnx-sys version".into());
    }
    let version = package
        .get("version")
        .and_then(toml::Value::as_str)
        .ok_or("sherpa-onnx-sys version is missing")?;
    if version.split('.').count() != 3
        || !version
            .split('.')
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
    {
        return Err("Sherpa provisioning requires a stable numeric release version".into());
    }
    let stem = format!("sherpa-onnx-v{version}-win-{platform}-static-MD-Release-lib");
    let base = cache("sherpa")?;
    let destination = base.join(&stem);
    let source = format!(
        "https://github.com/k2-fsa/sherpa-onnx/releases/download/v{version}/{stem}.tar.bz2"
    );
    if destination.exists() {
        validate(&destination, version, arch, &source)?;
        return Ok(destination.join("lib"));
    }
    fs::create_dir_all(&base).map_err(|e| e.to_string())?;
    let metadata = base.join(format!("release-{version}-{}.json", std::process::id()));
    fetch(
        &format!("https://api.github.com/repos/k2-fsa/sherpa-onnx/releases/tags/v{version}"),
        &metadata,
        Some(16 * 1024 * 1024),
    )?;
    let release: Value = serde_json::from_slice(&fs::read(&metadata).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    fs::remove_file(metadata).map_err(|e| e.to_string())?;
    let name = format!("{stem}.tar.bz2");
    let asset = release
        .get("assets")
        .and_then(Value::as_array)
        .and_then(|assets| {
            assets
                .iter()
                .find(|a| a.get("name").and_then(Value::as_str) == Some(&name))
        })
        .ok_or("the locked Sherpa release has no matching Windows static /MD archive")?;
    let digest = asset
        .get("digest")
        .and_then(Value::as_str)
        .and_then(|s| s.strip_prefix("sha256:"))
        .filter(|s| valid_digest(s))
        .ok_or("Sherpa release asset has no valid published SHA-256")?;
    let size = asset
        .get("size")
        .and_then(Value::as_u64)
        .filter(|n| *n > 0 && *n <= 512 * 1024 * 1024)
        .ok_or("Sherpa archive size is missing or exceeds 512 MiB")?;
    let archive = base.join(name);
    if !archive.exists() {
        // A process-specific download avoids sharing curl's partial file with
        // another workspace installing the same release concurrently.
        let download = base.join(format!("{stem}-{}.download", std::process::id()));
        fetch(&source, &download, Some(size))?;
        if fs::metadata(&download).map_err(|e| e.to_string())?.len() != size
            || sha256(&download)? != digest
        {
            fs::remove_file(&download).map_err(|e| e.to_string())?;
            return Err("downloaded Sherpa archive failed the published size/SHA-256 check".into());
        }
        if let Err(error) = fs::rename(&download, &archive) {
            if !archive.exists() {
                return Err(error.to_string());
            }
            fs::remove_file(download).map_err(|e| e.to_string())?;
        }
    }
    if fs::metadata(&archive).map_err(|e| e.to_string())?.len() != size
        || sha256(&archive)? != digest
    {
        return Err(format!(
            "Sherpa archive integrity check failed; remove {} and rerun cargo xtask sherpa",
            archive.display()
        ));
    }
    let staging = base.join(format!("{stem}.partial-{}", std::process::id()));
    fs::create_dir(&staging).map_err(|e| e.to_string())?;
    let result = (|| {
        let status = Command::new("tar")
            .arg("-xf")
            .arg(&archive)
            .arg("--strip-components=1")
            .arg("-C")
            .arg(&staging)
            .status()
            .map_err(|e| e.to_string())?;
        if !status.success() {
            return Err(format!("extracting Sherpa /MD archive failed: {status}"));
        }
        let receipt = json!({"version":version,"arch":arch,"source":source,"sha256":digest});
        fs::write(staging.join("BUILD.json"), receipt.to_string()).map_err(|e| e.to_string())?;
        validate(&staging, version, arch, &source)?;
        // Another workspace may have finished the identical immutable install first.
        if let Err(error) = fs::rename(&staging, &destination) {
            if !destination.exists() {
                return Err(error.to_string());
            }
            validate(&destination, version, arch, &source)?;
        }
        eprintln!("Sherpa {version} Windows {arch}: static /MD, verified SHA-256 {digest}");
        Ok(destination.join("lib"))
    })();
    if staging.exists() {
        fs::remove_dir_all(&staging).map_err(|e| e.to_string())?;
    }
    result
}

fn valid_digest(digest: &str) -> bool {
    digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit())
}

fn validate(dir: &Path, version: &str, arch: &str, source: &str) -> Result<(), String> {
    let complete = fs::read(dir.join("BUILD.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .is_some_and(|r| {
            r.get("version").and_then(Value::as_str) == Some(version)
                && r.get("arch").and_then(Value::as_str) == Some(arch)
                && r.get("source").and_then(Value::as_str) == Some(source)
                && r.get("sha256")
                    .and_then(Value::as_str)
                    .is_some_and(valid_digest)
        })
        && LIBRARIES.iter().all(|name| {
            fs::metadata(dir.join("lib").join(format!("{name}.lib")))
                .is_ok_and(|m| m.is_file() && m.len() > 0)
        });
    if !complete {
        return Err(format!(
            "Sherpa /MD cache is incomplete; remove {} and rerun cargo xtask sherpa",
            dir.display()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archive_selection_requires_one_locked_version_and_complete_md_cache() {
        const CHILD: &str = "VIBECUT_SHERPA_CACHE_CHILD";
        if let Some(root) = std::env::var_os(CHILD) {
            let root = PathBuf::from(root);
            let arch = std::env::var("VIBECUT_SHERPA_ARCH").unwrap();
            let expected = std::env::var("VIBECUT_SHERPA_EXPECTED").unwrap();
            let result = provision(&root, &arch, |_, _, _| {
                Err("unexpected network attempt on an offline cache path".into())
            });
            if expected.is_empty() {
                let platform = if arch == "x86_64" { "x64" } else { "arm64" };
                assert_eq!(result.unwrap(), root.join(format!(".cache/vibecut/sherpa/sherpa-onnx-v2.0.1-win-{platform}-static-MD-Release-lib/lib")));
            } else {
                assert!(result.unwrap_err().contains(&expected));
            }
            return;
        }
        let valid = "[[package]]\nname = 'sherpa-onnx-sys'\nversion = '2.0.1'\n";
        let duplicate = format!("{valid}{valid}");
        let invalid_version = valid.replace("2.0.1", "2..1");
        let missing = valid.replace("sherpa-onnx-sys", "another-crate");
        let root = tempfile::Builder::new()
            .prefix("sherpa cache Tiếng Việt ")
            .tempdir()
            .unwrap();
        for (index, (arch, lock, damage, expected)) in [
            ("x86_64", valid, "", ""),
            ("aarch64", valid, "", ""),
            ("x86", valid, "", "support only"),
            ("x86_64", "not TOML", "", "invalid Cargo.lock"),
            ("x86_64", missing.as_str(), "", "no sherpa-onnx-sys"),
            ("x86_64", duplicate.as_str(), "", "exactly one"),
            ("x86_64", invalid_version.as_str(), "", "stable numeric"),
            ("x86_64", valid, "library", "cache is incomplete"),
            ("x86_64", valid, "receipt", "cache is incomplete"),
            ("x86_64", valid, "digest", "cache is incomplete"),
            ("x86_64", valid, "arch", "cache is incomplete"),
        ]
        .into_iter()
        .enumerate()
        {
            let case = root.path().join(index.to_string());
            fs::create_dir(&case).unwrap();
            fs::write(case.join("Cargo.lock"), lock).unwrap();
            let platform = if arch == "aarch64" { "arm64" } else { "x64" };
            let stem = format!("sherpa-onnx-v2.0.1-win-{platform}-static-MD-Release-lib");
            let install = case.join(".cache/vibecut/sherpa").join(&stem);
            fs::create_dir_all(install.join("lib")).unwrap();
            for name in LIBRARIES {
                fs::write(install.join("lib").join(format!("{name}.lib")), b"fixture").unwrap();
            }
            let receipt = json!({"version":"2.0.1", "arch":if damage == "arch" { "aarch64" } else { arch },
                "source":format!("https://github.com/k2-fsa/sherpa-onnx/releases/download/v2.0.1/{stem}.tar.bz2"),
                "sha256":"0".repeat(if damage == "digest" {63} else {64})});
            fs::write(
                install.join("BUILD.json"),
                if damage == "receipt" {
                    "{}".to_owned()
                } else {
                    receipt.to_string()
                },
            )
            .unwrap();
            if damage == "library" {
                fs::remove_file(install.join("lib/onnxruntime.lib")).unwrap();
            }
            let status = Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "sherpa::tests::archive_selection_requires_one_locked_version_and_complete_md_cache", "--nocapture"])
                .env(CHILD, &case).env("VIBECUT_SHERPA_ARCH", arch)
                .env("VIBECUT_SHERPA_EXPECTED", expected)
                .env("HOME", &case).env("USERPROFILE", &case)
                .status().unwrap();
            assert!(status.success(), "case {index}");
        }
    }
}

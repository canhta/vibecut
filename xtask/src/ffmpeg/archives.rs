//! Cache location, transport and checksums for native build archives.
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::{env, fs};

pub(crate) fn cache(component: &str) -> Result<PathBuf, String> {
    // MSYS HOME can be /home/name, which is not a Windows absolute path and
    // disagrees with the native runner's cache location.
    let variable = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    let home = env::var_os(variable).ok_or_else(|| format!("{variable} is unavailable"))?;
    if !Path::new(&home).is_absolute() {
        return Err(format!("{variable} must be a native absolute path"));
    }
    if home.to_str().is_none() {
        return Err("the C build cache path must be valid UTF-8".into());
    }
    Ok(PathBuf::from(home).join(".cache/vibecut").join(component))
}

pub(crate) fn fetch(url: &str, destination: &Path) -> Result<(), String> {
    fetch_limited(url, destination, None)
}

pub(crate) fn fetch_limited(
    url: &str,
    destination: &Path,
    limit: Option<u64>,
) -> Result<(), String> {
    let tmp = destination.with_extension("part");
    let mut command = Command::new("curl");
    command
        .args([
            "--fail",
            "--location",
            "--retry",
            "2",
            "--connect-timeout",
            "20",
            "--max-time",
            "600",
            "--output",
        ])
        .arg(&tmp)
        .arg(url);
    if let Some(limit) = limit {
        command.args(["--max-filesize", &limit.to_string()]);
    }
    let status = command.status().map_err(|e| e.to_string())?;
    if !status.success() {
        return Err(format!("{command:?}: {status}"));
    }
    fs::rename(tmp, destination).map_err(|e| e.to_string())
}

pub(crate) fn sha256(path: &Path) -> Result<String, String> {
    use std::io::Read;
    let mut file = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 65_536];
    loop {
        let n = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hash.update(buffer.get(..n).ok_or("hash read bounds")?);
    }
    Ok(hash.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

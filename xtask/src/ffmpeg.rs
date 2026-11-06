//! Resolve and build the current stable media libraries outside the repository.
mod archives;
mod command;
mod environment;
mod fetch;
mod oracles;
mod recipes;
mod sherpa;
mod verify;
#[cfg(test)]
mod tests;

pub use environment::{environment, environment_at, external, host, prefix};
pub use fetch::cache_key;
pub(crate) use archives::{cache as archive_cache, fetch_limited};
pub(crate) use sherpa::provision as sherpa_provision;
pub(crate) use environment::{append_encoded_flag, cache};
pub(crate) use oracles::{begin, print_fixtures};
#[cfg(test)]
pub(crate) use oracles::{Version, inventory, parse_version};

use environment::native_target;
use fetch::{ZLIB_REPOSITORY, stable_tag};
use recipes::{build_source, build_tool};
use std::fs;

pub fn provision(target: Option<&str>) -> Result<(), String> {
    if cfg!(windows) {
        for name in ["bash", "make", "cygpath"] {
            eprintln!("MSYS2 {name}: {}", build_tool(name)?.display());
        }
    }
    let host = host()?;
    let target = target.unwrap_or(&host);
    if ![
        "macos-aarch64",
        "macos-x86_64",
        "windows-x86_64",
        "windows-aarch64",
        "linux-x86_64",
        "linux-aarch64",
    ]
    .contains(&target)
    {
        return Err(format!("unsupported FFmpeg target {target}"));
    }
    if target.starts_with("windows-") || target.starts_with("linux-") {
        native_target(&host, target)?;
    }
    let version = stable_tag("https://github.com/FFmpeg/FFmpeg.git", "n")?;
    let opus = stable_tag("https://github.com/xiph/opus.git", "v")?;
    let vpx = stable_tag("https://github.com/webmproject/libvpx.git", "v")?;
    let zlib = stable_tag(ZLIB_REPOSITORY, "v")?;
    let base = cache()?.join(target);
    fs::create_dir_all(&base).map_err(|e| e.to_string())?;
    let lock = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(base.join("building.lock"))
        .map_err(|e| format!("FFmpeg provision already running or stale building.lock: {e}"))?;
    let result = build_source(&base, target, &version, &opus, &vpx, &zlib);
    drop(lock);
    fs::remove_file(base.join("building.lock")).map_err(|e| e.to_string())?;
    result
}

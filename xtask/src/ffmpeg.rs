//! Resolve and build the current stable LGPL media libraries outside the repository.

use sha2::{Digest, Sha256};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn host() -> Result<String, String> {
    let os = match env::consts::OS {
        "macos" => "macos",
        "windows" => "windows",
        other => return Err(format!("unsupported host {other}")),
    };
    Ok(format!("{os}-{}", env::consts::ARCH))
}

pub fn cache() -> Result<PathBuf, String> {
    let home = env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .ok_or("HOME/USERPROFILE is unavailable")?;
    if home.to_str().is_none() {
        return Err("the C build cache path must be valid UTF-8".into());
    }
    Ok(PathBuf::from(home).join(".cache/vibecut/ffmpeg"))
}

fn run(command: &mut Command) -> Result<(), String> {
    let result = command.status().map_err(|e| e.to_string())?;
    if result.success() {
        Ok(())
    } else {
        Err(format!("{command:?}: {result}"))
    }
}

fn capture(command: &mut Command) -> Result<String, String> {
    let result = command.output().map_err(|e| e.to_string())?;
    if !result.status.success() {
        return Err(format!(
            "{command:?}: {}",
            String::from_utf8_lossy(&result.stderr)
        ));
    }
    String::from_utf8(result.stdout).map_err(|e| e.to_string())
}

fn stable_tag(repository: &str, prefix: &str) -> Result<String, String> {
    let text = capture(Command::new("git").args(["ls-remote", "--tags", "--refs", repository]))?;
    text.lines()
        .filter_map(|line| line.split("refs/tags/").nth(1))
        .filter_map(|tag| {
            let version = tag.strip_prefix(prefix)?;
            let parts: Option<Vec<u64>> = version.split('.').map(|n| n.parse().ok()).collect();
            let mut parts = parts?;
            if parts.len() == 2 {
                parts.push(0);
            }
            if parts.len() == 3 {
                Some((parts, version.to_owned()))
            } else {
                None
            }
        })
        .max_by(|a, b| a.0.cmp(&b.0))
        .map(|(_, version)| version)
        .ok_or_else(|| format!("no stable release in {repository}"))
}

fn fetch(url: &str, destination: &Path) -> Result<(), String> {
    let tmp = destination.with_extension("part");
    run(Command::new("curl")
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
        .arg(url))?;
    fs::rename(tmp, destination).map_err(|e| e.to_string())
}

fn sha256(path: &Path) -> Result<String, String> {
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

fn source(
    base: &Path,
    name: &str,
    version: &str,
    url: &str,
    receipt: &mut String,
) -> Result<PathBuf, String> {
    let archive = base.join(format!("{name}-{version}.archive"));
    if !archive.exists() {
        fetch(url, &archive)?;
    }
    let measured = sha256(&archive)?;
    // FFmpeg distributes detached signatures rather than an archive checksum;
    // other upstreams do not consistently publish checksums for source snapshots.
    // Verify a published SHA-256 when the canonical archive has one.
    // Upstream FFmpeg releases publish detached OpenPGP signatures, and GitHub
    // source snapshots publish no SHA-256 sidecar. This measured hash records
    // bytes fetched over verified HTTPS; it is not an upstream checksum claim.
    let published = false;
    receipt.push_str(&format!("{name}: {version}\nsource: {url}\nsha256: {measured}\nupstream checksum verified: {published}\n\n"));
    let dir = base.join(format!("{name}-{version}"));
    if !dir.exists() {
        fs::create_dir(&dir).map_err(|e| e.to_string())?;
        run(Command::new("tar")
            .arg("-xf")
            .arg(&archive)
            .arg("--strip-components=1")
            .arg("-C")
            .arg(&dir))?;
    }
    Ok(dir)
}

pub fn prefix() -> Result<PathBuf, String> {
    let pointer = cache()?.join(host()?).join("current");
    let path = fs::read_to_string(&pointer)
        .map_err(|_| "run cargo xtask ffmpeg before building media".to_owned())?;
    let prefix = PathBuf::from(path.trim());
    if !prefix.join("BUILD.txt").is_file() {
        return Err("FFmpeg cache is incomplete; rerun cargo xtask ffmpeg".into());
    }
    Ok(prefix)
}

/// Give every workspace Cargo process the same shared-library installation.
pub fn environment(command: &mut Command) -> Result<(), String> {
    let prefix = prefix()?;
    command.env("FFMPEG_DIR", &prefix);
    command.env("PKG_CONFIG_PATH", prefix.join("lib/pkgconfig"));
    let library_var = if cfg!(target_os = "macos") {
        "DYLD_LIBRARY_PATH"
    } else {
        "PATH"
    };
    let mut paths = vec![prefix.join(if cfg!(windows) { "bin" } else { "lib" })];
    if let Some(old) = env::var_os(library_var) {
        paths.extend(env::split_paths(&old));
    }
    command.env(
        library_var,
        env::join_paths(paths).map_err(|e| e.to_string())?,
    );
    if cfg!(target_os = "macos") {
        let mut flags = env::var("CARGO_ENCODED_RUSTFLAGS").unwrap_or_default();
        if flags.is_empty() {
            flags = env::var("RUSTFLAGS")
                .unwrap_or_default()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join("\x1f");
        }
        if !flags.is_empty() {
            flags.push('\x1f');
        }
        flags.push_str("-C\x1flink-arg=-Wl,-rpath,");
        flags.push_str(
            prefix
                .join("lib")
                .to_str()
                .ok_or("FFmpeg cache path is not UTF-8")?,
        );
        command.env("CARGO_ENCODED_RUSTFLAGS", flags);
    }
    Ok(())
}

pub fn provision(target: Option<&str>) -> Result<(), String> {
    let host = host()?;
    let target = target.unwrap_or(&host);
    if ![
        "macos-aarch64",
        "macos-x86_64",
        "windows-x86_64",
        "windows-aarch64",
    ]
    .contains(&target)
    {
        return Err(format!("unsupported FFmpeg target {target}"));
    }
    // Windows builds use the shared LGPL distributions; macOS builds use source
    // because a complete LGPL shared SDK is not published by the upstream project.
    if target.starts_with("windows-") {
        return windows(target);
    }
    if !cfg!(target_os = "macos") {
        return Err("macOS FFmpeg builds require a Mac and Xcode command line tools".into());
    }
    let version = stable_tag("https://github.com/FFmpeg/FFmpeg.git", "n")?;
    let opus = stable_tag("https://github.com/xiph/opus.git", "v")?;
    let vpx = stable_tag("https://github.com/webmproject/libvpx.git", "v")?;
    let base = cache()?.join(target);
    fs::create_dir_all(&base).map_err(|e| e.to_string())?;
    let lock = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(base.join("building.lock"))
        .map_err(|e| format!("FFmpeg provision already running or stale building.lock: {e}"))?;
    let result = build_macos(&base, target, &version, &opus, &vpx);
    drop(lock);
    fs::remove_file(base.join("building.lock")).map_err(|e| e.to_string())?;
    result
}

fn build_macos(
    base: &Path,
    target: &str,
    version: &str,
    opus: &str,
    vpx: &str,
) -> Result<(), String> {
    let prefix = base.join(format!("{version}-opus{opus}-vpx{vpx}"));
    if prefix.join("BUILD.txt").exists() {
        tool_runpaths(&prefix)?;
        fs::write(base.join("current"), prefix.as_os_str().as_encoded_bytes())
            .map_err(|e| e.to_string())?;
        println!("FFmpeg {version}: {}", prefix.display());
        return Ok(());
    }
    let mut receipt = format!("FFmpeg LGPL shared build for {target}\n\n");
    let sources = base.join("sources");
    fs::create_dir_all(&sources).map_err(|e| e.to_string())?;
    let ff = source(
        &sources,
        "ffmpeg",
        version,
        &format!("https://ffmpeg.org/releases/ffmpeg-{version}.tar.xz"),
        &mut receipt,
    )?;
    let op = source(
        &sources,
        "opus",
        opus,
        &format!("https://github.com/xiph/opus/archive/refs/tags/v{opus}.tar.gz"),
        &mut receipt,
    )?;
    let vp = source(
        &sources,
        "libvpx",
        vpx,
        &format!("https://github.com/webmproject/libvpx/archive/refs/tags/v{vpx}.tar.gz"),
        &mut receipt,
    )?;
    let arch = if target.ends_with("aarch64") {
        "arm64"
    } else {
        "x86_64"
    };
    fs::create_dir_all(&prefix).map_err(|e| e.to_string())?;
    run(Command::new("cmake")
        .arg("-S")
        .arg(&op)
        .arg("-B")
        .arg(op.join(target))
        .arg(format!("-DCMAKE_INSTALL_PREFIX={}", prefix.display()))
        .args([
            "-DCMAKE_BUILD_TYPE=Release",
            "-DBUILD_SHARED_LIBS=OFF",
            "-DCMAKE_POSITION_INDEPENDENT_CODE=ON",
            "-DOPUS_BUILD_TESTING=OFF",
            "-DOPUS_BUILD_PROGRAMS=OFF",
            "-DOPUS_DNN=OFF",
            "-DCMAKE_OSX_DEPLOYMENT_TARGET=11.0",
        ])
        .arg(format!("-DCMAKE_OSX_ARCHITECTURES={arch}")))?;
    run(Command::new("cmake")
        .arg("--build")
        .arg(op.join(target))
        .args(["--parallel", "3"]))?;
    run(Command::new("cmake").arg("--install").arg(op.join(target)))?;
    let vpbuild = vp.join(target);
    fs::create_dir_all(&vpbuild).map_err(|e| e.to_string())?;
    let vpflags = [
        format!("--prefix={}", prefix.display()),
        format!("--target={arch}-darwin20-gcc"),
        "--disable-shared".into(),
        "--enable-pic".into(),
        "--disable-examples".into(),
        "--disable-tools".into(),
        "--disable-unit-tests".into(),
        "--disable-docs".into(),
    ];
    run(Command::new(vp.join("configure"))
        .current_dir(&vpbuild)
        .args(&vpflags))?;
    run(Command::new("make").current_dir(&vpbuild).args(["-j3"]))?;
    run(Command::new("make").current_dir(&vpbuild).arg("install"))?;
    let ffbuild = ff.join(target);
    fs::create_dir_all(&ffbuild).map_err(|e| e.to_string())?;
    let flags = vec![
        format!("--prefix={}", prefix.display()),
        "--enable-shared".into(),
        "--disable-static".into(),
        "--disable-gpl".into(),
        "--disable-nonfree".into(),
        "--disable-version3".into(),
        "--disable-autodetect".into(),
        "--disable-network".into(),
        "--disable-doc".into(),
        "--disable-avdevice".into(),
        "--disable-ffplay".into(),
        "--enable-libvpx".into(),
        "--enable-libopus".into(),
        "--enable-videotoolbox".into(),
        "--enable-audiotoolbox".into(),
        "--install-name-dir=@rpath".into(),
        "--enable-cross-compile".into(),
        "--target-os=darwin".into(),
        format!("--arch={arch}"),
        format!("--extra-cflags=-arch {arch} -mmacosx-version-min=11.0"),
        format!("--extra-ldflags=-arch {arch} -mmacosx-version-min=11.0 -Wl,-rpath,@loader_path"),
    ];
    run(Command::new(ff.join("configure"))
        .current_dir(&ffbuild)
        .args(&flags)
        .env("PKG_CONFIG_LIBDIR", prefix.join("lib/pkgconfig")))?;
    run(Command::new("make").current_dir(&ffbuild).args(["-j3"]))?;
    run(Command::new("make").current_dir(&ffbuild).arg("install"))?;
    tool_runpaths(&prefix)?;
    let license = fs::read_to_string(ffbuild.join("config.h")).map_err(|e| e.to_string())?;
    if !license.contains("#define CONFIG_GPL 0") || !license.contains("#define CONFIG_NONFREE 0") {
        return Err("FFmpeg build failed LGPL admission".into());
    }
    let compliance = prefix.join("compliance");
    fs::create_dir_all(&compliance).map_err(|e| e.to_string())?;
    for (source, name) in [
        (&ff, "COPYING.LGPLv2.1"),
        (&ff, "LICENSE.md"),
        (&op, "COPYING"),
        (&vp, "LICENSE"),
    ] {
        fs::copy(
            source.join(name),
            compliance.join(format!(
                "{}-{name}",
                source.file_name().ok_or("source name")?.to_string_lossy()
            )),
        )
        .map_err(|e| e.to_string())?;
    }
    receipt.push_str(&format!("opus flags: static PIC, DNN off, architecture {arch}\nlibvpx flags: {vpflags:?}\nFFmpeg configure: {flags:?}\n"));
    fs::write(prefix.join("BUILD.txt"), receipt).map_err(|e| e.to_string())?;
    fs::write(base.join("current"), prefix.as_os_str().as_encoded_bytes())
        .map_err(|e| e.to_string())?;
    println!("FFmpeg {version}: {}", prefix.display());
    Ok(())
}

fn tool_runpaths(prefix: &Path) -> Result<(), String> {
    for name in ["ffmpeg", "ffprobe"] {
        let tool = prefix.join("bin").join(name);
        let paths = capture(Command::new("otool").arg("-l").arg(&tool))?;
        if !paths.contains("path @executable_path/../lib ") {
            run(Command::new("install_name_tool")
                .args(["-add_rpath", "@executable_path/../lib"])
                .arg(&tool))?;
            run(Command::new("codesign")
                .args(["--force", "--sign", "-"])
                .arg(&tool))?;
        }
    }
    Ok(())
}

fn windows(target: &str) -> Result<(), String> {
    let base = cache()?.join(target);
    fs::create_dir_all(&base).map_err(|e| e.to_string())?;
    let lock = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(base.join("building.lock"))
        .map_err(|e| format!("FFmpeg provisioning lock: {e}"))?;
    let result = windows_install(target);
    drop(lock);
    fs::remove_file(base.join("building.lock")).map_err(|e| e.to_string())?;
    result
}

fn windows_install(target: &str) -> Result<(), String> {
    let release: serde_json::Value = serde_json::from_str(&capture(Command::new("curl").args([
        "-fsSL",
        "--max-time",
        "60",
        "https://api.github.com/repos/BtbN/FFmpeg-Builds/releases/latest",
    ]))?)
    .map_err(|e| e.to_string())?;
    let architecture = if target.ends_with("aarch64") {
        "winarm64"
    } else {
        "win64"
    };
    let assets = release
        .get("assets")
        .and_then(serde_json::Value::as_array)
        .ok_or("release has no assets")?;
    let asset = assets
        .iter()
        .filter_map(|a| {
            let name = a.get("name")?.as_str()?;
            if !name.contains(architecture)
                || !name.contains("lgpl-shared")
                || !name.ends_with(".zip")
            {
                return None;
            }
            let version = name.strip_prefix("ffmpeg-n")?.split('-').next()?;
            let parts: Option<Vec<u64>> = version.split('.').map(|p| p.parse().ok()).collect();
            Some((parts?, a))
        })
        .max_by(|a, b| a.0.cmp(&b.0))
        .map(|(_, a)| a)
        .ok_or("no stable LGPL shared build for this target")?;
    let name = asset
        .get("name")
        .and_then(serde_json::Value::as_str)
        .ok_or("asset name")?;
    let url = asset
        .get("browser_download_url")
        .and_then(serde_json::Value::as_str)
        .ok_or("asset URL")?;
    let base = cache()?.join(target);
    fs::create_dir_all(&base).map_err(|e| e.to_string())?;
    let archive = base.join(name);
    let expected = asset
        .get("digest")
        .and_then(serde_json::Value::as_str)
        .ok_or("asset source did not publish a SHA-256 digest")?;
    if !archive.exists() || expected != format!("sha256:{}", sha256(&archive)?) {
        fetch(url, &archive)?;
    }
    let digest = sha256(&archive)?;
    let published = asset
        .get("digest")
        .and_then(serde_json::Value::as_str)
        .ok_or("asset source did not publish a SHA-256 digest")?;
    if published != format!("sha256:{digest}") {
        return Err("FFmpeg published checksum mismatch".into());
    }
    let dir = base.join(format!("{}-{digest}", name.trim_end_matches(".zip")));
    if !dir.exists() {
        let staging = base.join(format!("extract-{digest}"));
        fs::create_dir(&staging).map_err(|e| e.to_string())?;
        let result = run(Command::new("tar")
            .arg("-xf")
            .arg(&archive)
            .arg("--strip-components=1")
            .arg("-C")
            .arg(&staging));
        if let Err(error) = result {
            fs::remove_dir_all(&staging).map_err(|e| e.to_string())?;
            return Err(error);
        }
        fs::rename(staging, &dir).map_err(|e| e.to_string())?;
    }
    fs::write(dir.join("BUILD.txt"),format!("Resolved {name}\nsource: {url}\nsha256: {digest}\nPublished source digest verified.\nBuild recipe: https://github.com/BtbN/FFmpeg-Builds/tree/{}\n",release.get("tag_name").and_then(serde_json::Value::as_str).ok_or("release tag")?)).map_err(|e|e.to_string())?;
    fs::write(base.join("current"), dir.as_os_str().as_encoded_bytes())
        .map_err(|e| e.to_string())?;
    println!("FFmpeg resolved {name}: {}", dir.display());
    Ok(())
}

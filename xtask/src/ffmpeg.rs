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
    if cfg!(windows) {
        command
            .env("CC", "clang")
            .env("CXX", "clang++")
            .env("AR", "llvm-ar")
            .env("NM", "llvm-nm");
    }
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
    let expected_patch_id = patch_id(&patches()?)?;
    if !prefix.join("BUILD.txt").is_file()
        || !prefix
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.ends_with(&expected_patch_id))
    {
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
    if target.starts_with("windows-") && target != host {
        return Err("source provisioning must run on the target host/architecture".into());
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
    let result = build_source(&base, target, &version, &opus, &vpx);
    drop(lock);
    fs::remove_file(base.join("building.lock")).map_err(|e| e.to_string())?;
    result
}

fn build_source(
    base: &Path,
    target: &str,
    version: &str,
    opus: &str,
    vpx: &str,
) -> Result<(), String> {
    let patches = patches()?;
    let patch_id = patch_id(&patches)?;
    let prefix = base.join(format!("{version}-opus{opus}-vpx{vpx}-{patch_id}"));
    if prefix.join("BUILD.txt").exists() {
        record_patches(&prefix, &patches)?;
        if cfg!(target_os = "macos") {
            tool_runpaths(&prefix)?;
        }
        fs::write(base.join("current"), prefix.as_os_str().as_encoded_bytes())
            .map_err(|e| e.to_string())?;
        println!("FFmpeg {version}: {}", prefix.display());
        return Ok(());
    }
    let mut receipt = format!("FFmpeg LGPL shared build for {target}\n\n");
    let sources = base.join("sources");
    fs::create_dir_all(&sources).map_err(|e| e.to_string())?;
    let _pristine = source(
        &sources,
        "ffmpeg",
        version,
        &format!("https://ffmpeg.org/releases/ffmpeg-{version}.tar.xz"),
        &mut receipt,
    )?;
    // Patch a fresh extraction, never mutate the shared pristine source cache.
    let patched = sources.join(format!("ffmpeg-{version}-{patch_id}"));
    if patched.exists() {
        fs::remove_dir_all(&patched).map_err(|e| e.to_string())?;
    }
    fs::create_dir(&patched).map_err(|e| e.to_string())?;
    run(Command::new("tar")
        .arg("-xf")
        .arg(sources.join(format!("ffmpeg-{version}.archive")))
        .arg("--strip-components=1")
        .arg("-C")
        .arg(&patched))?;
    for patch in &patches {
        run(Command::new("git").args(["apply", "--check"]).arg(patch).current_dir(&patched))
            .map_err(|e| format!("FFmpeg security backport no longer applies; review/delete it when stable includes it: {e}"))?;
        run(Command::new("git")
            .arg("apply")
            .arg(patch)
            .current_dir(&patched))?;
        receipt.push_str(&format!(
            "backport: {} sha256: {}\n",
            patch.display(),
            sha256(patch)?
        ));
    }
    let ff = patched;
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
        ])
        .args(if cfg!(target_os = "macos") {
            vec![
                format!("-DCMAKE_OSX_ARCHITECTURES={arch}"),
                "-DCMAKE_OSX_DEPLOYMENT_TARGET=11.0".into(),
            ]
        } else {
            vec![
                "-G".into(),
                "Ninja".into(),
                "-DCMAKE_C_COMPILER=clang".into(),
                "-DCMAKE_CXX_COMPILER=clang++".into(),
            ]
        }))?;
    run(Command::new("cmake")
        .arg("--build")
        .arg(op.join(target))
        .args(["--parallel", "3"]))?;
    run(Command::new("cmake").arg("--install").arg(op.join(target)))?;
    let vpbuild = vp.join(target);
    fs::create_dir_all(&vpbuild).map_err(|e| e.to_string())?;
    let vpflags = [
        format!("--prefix={}", shell_path(&prefix)?),
        format!(
            "--target={}",
            if cfg!(windows) {
                format!(
                    "{}-win64-gcc",
                    if arch == "arm64" { "arm64" } else { "x86_64" }
                )
            } else {
                format!("{arch}-darwin20-gcc")
            }
        ),
        "--disable-shared".into(),
        "--enable-pic".into(),
        "--disable-examples".into(),
        "--disable-tools".into(),
        "--disable-unit-tests".into(),
        "--disable-docs".into(),
    ];
    run(Command::new("bash")
        .arg(shell_path(&vp.join("configure"))?)
        .current_dir(&vpbuild)
        .args(&vpflags))?;
    run(Command::new("make").current_dir(&vpbuild).args(["-j3"]))?;
    run(Command::new("make").current_dir(&vpbuild).arg("install"))?;
    let ffbuild = ff.join(target);
    fs::create_dir_all(&ffbuild).map_err(|e| e.to_string())?;
    let mut flags = vec![
        format!("--prefix={}", shell_path(&prefix)?),
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
    ];
    if cfg!(target_os = "macos") {
        flags.extend([
            "--enable-videotoolbox".into(),
            "--enable-audiotoolbox".into(),
            "--install-name-dir=@rpath".into(),
            "--enable-cross-compile".into(),
            "--target-os=darwin".into(),
            format!("--arch={arch}"),
            format!("--extra-cflags=-arch {arch} -mmacosx-version-min=11.0"),
            format!(
                "--extra-ldflags=-arch {arch} -mmacosx-version-min=11.0 -Wl,-rpath,@loader_path"
            ),
        ]);
    } else {
        flags.extend([
            "--target-os=mingw32".into(),
            "--cc=clang".into(),
            "--cxx=clang++".into(),
            "--ld=clang".into(),
            "--ar=llvm-ar".into(),
            "--nm=llvm-nm".into(),
            "--dlltool=llvm-dlltool".into(),
            format!("--arch={arch}"),
            "--enable-d3d11va".into(),
            "--enable-dxva2".into(),
            "--enable-mediafoundation".into(),
        ]);
    }
    run(Command::new("bash")
        .arg(shell_path(&ff.join("configure"))?)
        .current_dir(&ffbuild)
        .args(&flags)
        .env(
            "PKG_CONFIG_LIBDIR",
            shell_path(&prefix.join("lib/pkgconfig"))?,
        ))?;
    run(Command::new("make").current_dir(&ffbuild).args(["-j3"]))?;
    run(Command::new("make").current_dir(&ffbuild).arg("install"))?;
    if cfg!(windows) {
        // FFmpeg's MinGW recipe also emits MSVC-compatible import libraries.
        for library in [
            "avcodec",
            "avformat",
            "avutil",
            "swresample",
            "swscale",
            "avfilter",
        ] {
            fs::copy(
                ffbuild.join(format!("lib{library}/{library}.lib")),
                prefix.join(format!("lib/{library}.lib")),
            )
            .map_err(|e| e.to_string())?;
        }
    }
    if cfg!(target_os = "macos") {
        tool_runpaths(&prefix)?;
    }
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
    record_patches(&prefix, &patches)?;
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

fn patches() -> Result<Vec<PathBuf>, String> {
    let mut patches = fs::read_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("patches"))
        .map_err(|e| e.to_string())?
        .map(|entry| entry.map(|e| e.path()).map_err(|e| e.to_string()))
        .collect::<Result<Vec<_>, _>>()?;
    patches.retain(|p| p.extension().is_some_and(|e| e == "patch"));
    patches.sort();
    Ok(patches)
}
fn patch_id(patches: &[PathBuf]) -> Result<String, String> {
    let mut hash = Sha256::new();
    for patch in patches {
        hash.update(fs::read(patch).map_err(|e| e.to_string())?);
    }
    Ok(hash
        .finalize()
        .iter()
        .take(8)
        .map(|b| format!("{b:02x}"))
        .collect())
}

fn shell_path(path: &Path) -> Result<String, String> {
    if cfg!(windows) {
        Ok(capture(Command::new("cygpath").arg("-u").arg(path))?
            .trim()
            .to_owned())
    } else {
        path.to_str()
            .map(str::to_owned)
            .ok_or("native build path must be UTF-8".into())
    }
}

fn record_patches(prefix: &Path, patches: &[PathBuf]) -> Result<(), String> {
    let backports = prefix.join("compliance/patches");
    fs::create_dir_all(&backports).map_err(|e| e.to_string())?;
    for patch in patches {
        fs::copy(
            patch,
            backports.join(patch.file_name().ok_or("patch filename")?),
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

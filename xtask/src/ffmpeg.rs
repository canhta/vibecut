//! Resolve and build the current stable LGPL media libraries outside the repository.

use crate::archives::{fetch, sha256};
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

pub(super) fn cache() -> Result<PathBuf, String> {
    crate::archives::cache("ffmpeg")
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

/// Resolve the versions used by provisioning before restoring its hosted cache.
pub fn cache_key() -> Result<(), String> {
    let version = stable_tag("https://github.com/FFmpeg/FFmpeg.git", "n")?;
    let opus = stable_tag("https://github.com/xiph/opus.git", "v")?;
    let vpx = stable_tag("https://github.com/webmproject/libvpx.git", "v")?;
    println!("ffmpeg{version}-opus{opus}-vpx{vpx}");
    Ok(())
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
    environment_at(command, &prefix()?)
}

/// Distribution builds use their explicit audited SDK and the same native guards.
pub fn environment_at(command: &mut Command, prefix: &Path) -> Result<(), String> {
    command.env("FFMPEG_DIR", prefix);
    command.env("PKG_CONFIG_PATH", prefix.join("lib/pkgconfig"));
    let library_var = if cfg!(target_os = "macos") {
        "DYLD_LIBRARY_PATH"
    } else {
        "PATH"
    };
    let runtime = if cfg!(windows) {
        windows_runtime(prefix)?
    } else {
        prefix.join("lib")
    };
    let mut paths = vec![runtime];
    let managed_bin = prefix.join("bin").canonicalize().ok();
    if let Some(old) = env::var_os(library_var) {
        paths.extend(env::split_paths(&old).filter(|path| {
            // An inherited native bin must not reintroduce the managed CLI.
            !cfg!(windows) || managed_bin.is_none() || path.canonicalize().ok() != managed_bin
        }));
    }
    command.env(
        library_var,
        env::join_paths(paths).map_err(|e| e.to_string())?,
    );
    #[cfg(windows)]
    {
        let compiler = cc::windows_registry::find_tool(env::consts::ARCH, "cl.exe").ok_or(
            "install Visual Studio C++ build tools and a Windows SDK before workspace checks",
        )?;
        // cc discovers MSVC and SDK headers even from an MSYS2 shell. An
        // already-configured developer shell supplies INCLUDE by inheritance.
        // Keep our DLL PATH: importing the compiler's full environment loses it.
        let include = compiler
            .get_envs()
            .rfind(|(key, _)| *key == "INCLUDE")
            .map(|(_, value)| value.to_os_string())
            .or_else(|| env::var_os("INCLUDE"))
            .filter(|value| !value.is_empty())
            .ok_or("MSVC INCLUDE is empty; install the Windows SDK or use a Visual Studio developer shell")?;
        windows_bindgen_environment(command, &include);
        if env::var_os("SHERPA_ONNX_LIB_DIR").is_none() {
            let manifest =
                PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").ok_or("run via cargo xtask")?);
            let root = manifest
                .parent()
                .ok_or("xtask must live in workspace root")?;
            command.env(
                "SHERPA_ONNX_LIB_DIR",
                crate::sherpa::provision(root, env::consts::ARCH, crate::archives::fetch_limited)?,
            );
        }
    }
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

/// Expose native DLLs without putting the LGPL CLI ahead of external test oracles.
/// Hard links publish complete files atomically, including across concurrent xtasks,
/// and retain the exact library bytes from this immutable native installation.
fn windows_runtime(prefix: &Path) -> Result<PathBuf, String> {
    let runtime = prefix.join("runtime-dlls");
    fs::create_dir_all(&runtime).map_err(|e| e.to_string())?;
    let mut count = 0;
    for entry in fs::read_dir(prefix.join("bin")).map_err(|e| e.to_string())? {
        let source = entry.map_err(|e| e.to_string())?.path();
        if !source
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("dll"))
        {
            continue;
        }
        let destination = runtime.join(source.file_name().ok_or("DLL filename missing")?);
        match fs::hard_link(&source, &destination) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                if sha256(&source)? != sha256(&destination)? {
                    return Err("FFmpeg runtime DLL differs from native installation".into());
                }
            }
            Err(error) => return Err(format!("cannot stage FFmpeg runtime DLL: {error}")),
        }
        count += 1;
    }
    if count == 0 {
        return Err("FFmpeg installation has no runtime DLLs; rerun cargo xtask ffmpeg".into());
    }
    Ok(runtime)
}

#[cfg(any(windows, test))]
fn windows_bindgen_environment(command: &mut Command, include: &std::ffi::OsStr) {
    command.env("INCLUDE", include);
    // bindgen 0.73.2 translates Clang's packed AMX vector field to an
    // unaligned array, emitting E0587 and an invalid tile offset (p0-core/50).
    // Remove when ffmpeg-sys-next/bindgen handles this layout. AMX intrinsics
    // are outside the FFmpeg API; their dependent headers need guarding too.
    // https://github.com/llvm/llvm-project/blob/main/clang/lib/Headers/amxintrin.h
    const AMX_GUARDS: &str = concat!(
        "-D__AMXINTRIN_H -D__AMX_FP16INTRIN_H -D__AMX_COMPLEXINTRIN_H ",
        "-D__AMXFP8INTRIN_H -D__AMX_TF32INTRIN_H -D__AMXMOVRSINTRIN_H ",
        "-D__AMX_AVX512INTRIN_H -D__AMX_TRANSPOSEINTRIN_H ",
        "-D__AMX_BF16TRANSPOSEINTRIN_H -D__AMX_COMPLEXTRANSPOSEINTRIN_H ",
        "-D__AMX_FP16TRANSPOSEINTRIN_H -D__AMX_TF32TRANSPOSEINTRIN_H ",
        "-D__AMX_MOVRS_TRANSPOSEINTRIN_H",
    );
    let variable = "BINDGEN_EXTRA_CLANG_ARGS";
    let target = format!("{}-pc-windows-msvc", env::consts::ARCH);
    // bindgen reads the hyphenated target key first. Merge all original caller
    // forms before setting it, so synthesizing that key cannot shadow a form.
    let key = format!("{variable}_{target}");
    let mut args = String::new();
    for source in [
        variable.to_owned(),
        format!("{variable}_{}", target.replace('-', "_")),
        key.clone(),
    ] {
        if let Ok(value) = env::var(source) {
            args.push_str(&value);
            args.push(' ');
        }
    }
    args.push_str(AMX_GUARDS);
    // Native bindgen builds trust libclang's default target. MSYS2 Clang
    // defaults to GNU Windows, whose unsigned enums disagree with whisper-rs's
    // MSVC representation. Match the Rust ABI for every generated binding.
    args.push_str(&format!(" --target={target}"));
    command.env(key, args);
}

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
    run(Command::new(build_tool("bash")?)
        .arg(shell_path(&vp.join("configure"))?)
        .current_dir(&vpbuild)
        .args(&vpflags))?;
    run(Command::new(build_tool("make")?)
        .current_dir(&vpbuild)
        .args(["-j3"]))?;
    run(Command::new(build_tool("make")?)
        .current_dir(&vpbuild)
        .arg("install"))?;
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
            // libvpx is static and its MinGW pthread/semaphore dependencies
            // live in Libs.private; normal pkg-config output omits them.
            "--pkg-config-flags=--static".into(),
            // Screen recording is outside the editor's media contract. FFmpeg's
            // gfxcapture C++ build also collides with VERSION on Windows libc++.
            "--disable-filter=gfxcapture".into(),
            format!("--arch={arch}"),
            "--enable-d3d11va".into(),
            "--enable-dxva2".into(),
            "--enable-mediafoundation".into(),
        ]);
    }
    if let Err(error) = run(Command::new(build_tool("bash")?)
        .arg(shell_path(&ff.join("configure"))?)
        .current_dir(&ffbuild)
        .args(&flags)
        .env(
            "PKG_CONFIG_LIBDIR",
            shell_path(&prefix.join("lib/pkgconfig"))?,
        ))
    {
        let log = ffbuild.join("ffbuild/config.log");
        match fs::read_to_string(&log) {
            Ok(log) => {
                // configure starts its log with a shell environment dump. Only
                // emit compiler probes, never inherited credentials/settings.
                if let Some((_, probes)) = log.split_once("\nBEGIN ") {
                    eprintln!("FFmpeg configure compiler probes:\nBEGIN {probes}");
                }
            }
            Err(error) => eprintln!("cannot read {}: {error}", log.display()),
        }
        return Err(error);
    }
    // FFmpeg has no --dlltool configure switch; its MinGW recipe exposes
    // DLLTOOL as a make variable for MSVC-compatible import libraries.
    for task in ["-j3", "install"] {
        run(Command::new(build_tool("make")?)
            .current_dir(&ffbuild)
            .arg(task)
            .args(cfg!(windows).then_some("DLLTOOL=llvm-dlltool")))?;
    }
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
    // Shared target directories can reuse this binary after its build worktree is gone.
    let manifest = env::var_os("CARGO_MANIFEST_DIR").ok_or("run via cargo xtask")?;
    let mut patches = fs::read_dir(Path::new(&manifest).join("patches"))
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

// Rust's native Windows process lookup checks System32 before inherited PATH.
// Resolve MSYS tools ourselves so System32/bash.exe (WSL) can never win. The
// package manager distinguishes a full MSYS2 installation from Git for Windows.
fn build_tool(name: &str) -> Result<PathBuf, String> {
    if !cfg!(windows) {
        return Ok(PathBuf::from(name));
    }
    let path = env::var_os("PATH").ok_or("PATH is unavailable")?;
    let bin = env::split_paths(&path)
        .filter(|directory| directory.is_absolute())
        .find(|directory| {
            ["bash.exe", "cygpath.exe", "pacman.exe", "msys-2.0.dll"]
                .iter()
                .all(|file| directory.join(file).is_file())
        })
        .ok_or("MSYS2 usr/bin is missing from PATH; run in MSYS2 CLANG64/CLANGARM64")?;
    let tool = bin.join(format!("{name}.exe"));
    if !tool.is_file() {
        return Err(format!("MSYS2 tool is missing: {}", tool.display()));
    }
    Ok(tool)
}

fn shell_path(path: &Path) -> Result<String, String> {
    if cfg!(windows) {
        Ok(
            capture(Command::new(build_tool("cygpath")?).arg("-u").arg(path))?
                .trim()
                .to_owned(),
        )
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

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn cargo_children_resolve_external_oracles_with_native_dlls() {
        const CHILD: &str = "VIBECUT_ORACLE_ENV_CHILD";
        const TEST: &str =
            "ffmpeg::tests::cargo_children_resolve_external_oracles_with_native_dlls";
        if let Some(stage) = env::var_os(CHILD) {
            if stage == "oracle" {
                let executable = env::current_exe().unwrap();
                assert_eq!(
                    executable.parent().unwrap().file_name().unwrap(),
                    "external"
                );
                return;
            }
            let mut command = Command::new("ffmpeg.exe");
            command.args(["--exact", TEST]).env(CHILD, "oracle");
            environment(&mut command).unwrap();
            let path = command
                .get_envs()
                .find(|(key, _)| *key == std::ffi::OsStr::new("PATH"))
                .unwrap()
                .1
                .unwrap();
            let runtime = env::split_paths(path).next().unwrap();
            assert_eq!(
                fs::read(runtime.join("avcodec.dll")).unwrap(),
                b"native DLL"
            );
            assert!(!runtime.join("ffmpeg.exe").exists());
            // Repeated preparation must safely reuse an already-published DLL.
            environment(&mut command).unwrap();
            assert!(command.status().unwrap().success());
            return;
        }
        let directory =
            env::temp_dir().join(format!("vibecut oracle space-{}", std::process::id()));
        let base = directory
            .join(".cache/vibecut/ffmpeg")
            .join(host().unwrap());
        let prefix = base.join(format!(
            "fixture-{}",
            patch_id(&patches().unwrap()).unwrap()
        ));
        let external = directory.join("external");
        fs::create_dir_all(prefix.join("bin")).unwrap();
        fs::create_dir_all(&external).unwrap();
        fs::write(prefix.join("BUILD.txt"), "fixture").unwrap();
        fs::write(base.join("current"), prefix.to_str().unwrap()).unwrap();
        fs::write(prefix.join("bin/avcodec.dll"), b"native DLL").unwrap();
        fs::write(prefix.join("bin/ffmpeg.exe"), b"not the external oracle").unwrap();
        fs::copy(env::current_exe().unwrap(), external.join("ffmpeg.exe")).unwrap();
        let mut paths = vec![prefix.join("bin"), external];
        paths.extend(env::split_paths(&env::var_os("PATH").unwrap()));
        let result = Command::new(env::current_exe().unwrap())
            .args(["--exact", TEST, "--nocapture"])
            .env(CHILD, "configure")
            .env("USERPROFILE", &directory)
            .env("SHERPA_ONNX_LIB_DIR", &directory)
            .env("PATH", env::join_paths(paths).unwrap())
            .status()
            .unwrap();
        fs::remove_dir_all(&directory).unwrap();
        assert!(result.success());
    }

    #[test]
    fn provisioning_paths_preserve_native_home_and_shell_arguments() {
        const CHILD: &str = "VIBECUT_PROVISION_PATHS_CHILD";
        if let Some(expected) = env::var_os(CHILD) {
            if expected == "invalid" {
                assert!(cache().is_err());
                return;
            }
            let directory = PathBuf::from(expected);
            assert_eq!(cache().unwrap(), directory.join(".cache/vibecut/ffmpeg"));
            let script = directory.join("configure fixture.sh");
            let input = directory.join("đường dẫn input.txt");
            fs::write(&input, b"fixture").unwrap();
            fs::write(&script, b"test -f \"$1\" || exit 9\nprintf '%s' \"$2\"\n").unwrap();
            let prefix = format!("--prefix={}", shell_path(&directory).unwrap());
            let output = capture(
                Command::new(build_tool("bash").unwrap())
                    .arg(shell_path(&script).unwrap())
                    .arg(shell_path(&input).unwrap())
                    .arg(&prefix)
                    .current_dir(&directory),
            )
            .unwrap();
            assert_eq!(output, prefix);
            return;
        }
        let directory = env::temp_dir().join(format!(
            "vibecut provision Đặng space-{}",
            std::process::id()
        ));
        fs::create_dir_all(&directory).unwrap();
        let variable = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
        for value in [
            directory.as_os_str(),
            std::ffi::OsStr::new("relative"),
            std::ffi::OsStr::new(""),
        ] {
            let valid = value == directory.as_os_str();
            let status = Command::new(env::current_exe().unwrap())
                .args([
                    "--exact",
                    "ffmpeg::tests::provisioning_paths_preserve_native_home_and_shell_arguments",
                    "--nocapture",
                ])
                .env(
                    CHILD,
                    if valid {
                        directory.as_os_str()
                    } else {
                        std::ffi::OsStr::new("invalid")
                    },
                )
                .env("HOME", "/home/msys-only")
                .env(variable, value)
                .status()
                .unwrap();
            assert!(status.success());
        }
        fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn windows_bindgen_environment_preserves_caller_arguments() {
        const CHILD: &str = "VIBECUT_BINDGEN_ENV_CHILD";
        const INCLUDE: &str = "VIBECUT_MSVC_INCLUDE_CHILD";
        let variable = "BINDGEN_EXTRA_CLANG_ARGS";
        let target = format!("{}-pc-windows-msvc", env::consts::ARCH);
        let hyphen = format!("{variable}_{target}");
        let underscore = format!("{variable}_{}", target.replace('-', "_"));
        if let Ok(expected) = env::var(CHILD) {
            let mut command = Command::new("cargo");
            command.env("PATH", "runtime DLL path");
            windows_bindgen_environment(&mut command, &env::var_os(INCLUDE).unwrap());
            let include = command
                .get_envs()
                .find(|(key, _)| *key == std::ffi::OsStr::new("INCLUDE"))
                .unwrap()
                .1
                .unwrap();
            assert_eq!(include, env::var_os(INCLUDE).unwrap());
            assert_eq!(
                command
                    .get_envs()
                    .find(|(key, _)| *key == std::ffi::OsStr::new("PATH"))
                    .unwrap()
                    .1
                    .unwrap(),
                "runtime DLL path"
            );
            let (_, value) = command
                .get_envs()
                .find(|(key, _)| *key == std::ffi::OsStr::new(&hyphen))
                .unwrap();
            let args = value.unwrap().to_str().unwrap();
            let (caller, guards) = args.split_once("-D__AMXINTRIN_H").unwrap();
            assert_eq!(caller.trim_end(), expected);
            assert!(guards.contains("-D__AMX_FP16INTRIN_H"));
            assert!(args.ends_with(&format!(" --target={target}")));
            return;
        }
        for (general, underscored, hyphenated, expected, include) in [
            (None, None, None, "", None),
            (Some("-DGENERAL"), None, None, "-DGENERAL", None),
            (None, Some("-DUNDERSCORE"), None, "-DUNDERSCORE", None),
            (None, None, Some("-DHYPHEN"), "-DHYPHEN", None),
            (
                None,
                None,
                None,
                "",
                Some(r"C:\Program Files\MSVC\include;C:\SDK\Tiếng Việt\ucrt"),
            ),
            (
                Some("--target=x86_64-w64-windows-gnu"),
                None,
                None,
                "--target=x86_64-w64-windows-gnu",
                None,
            ),
            (
                Some("-DGENERAL"),
                Some("-DUNDERSCORE"),
                None,
                "-DGENERAL -DUNDERSCORE",
                None,
            ),
            (
                None,
                Some("-DUNDERSCORE"),
                Some("-DHYPHEN"),
                "-DUNDERSCORE -DHYPHEN",
                None,
            ),
            (
                Some("-I\"path with spaces\""),
                Some("-DUNDERSCORE"),
                Some("-DHYPHEN"),
                "-I\"path with spaces\" -DUNDERSCORE -DHYPHEN",
                None,
            ),
        ] {
            let mut child = Command::new(env::current_exe().unwrap());
            child
                .args([
                    "--exact",
                    "ffmpeg::tests::windows_bindgen_environment_preserves_caller_arguments",
                    "--nocapture",
                ])
                .env(CHILD, expected)
                .env(INCLUDE, include.unwrap_or("MSVC include"));
            for (key, value) in [
                (variable, general),
                (&underscore, underscored),
                (&hyphen, hyphenated),
            ] {
                child.env_remove(key);
                if let Some(value) = value {
                    child.env(key, value);
                }
            }
            assert!(child.status().unwrap().success(), "{expected}");
        }
    }

    #[test]
    fn patch_discovery_uses_the_invoking_worktrees_manifest() {
        const CHILD: &str = "VIBECUT_PATCH_DISCOVERY_CHILD";
        if env::var_os(CHILD).is_some() {
            let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
            assert_eq!(
                patches().unwrap(),
                vec![manifest.join("patches/runtime.patch")]
            );
            return;
        }
        let directory =
            env::temp_dir().join(format!("vibecut-runtime-patches-{}", std::process::id()));
        fs::create_dir_all(directory.join("patches")).unwrap();
        fs::write(directory.join("patches/runtime.patch"), b"runtime fixture").unwrap();
        let status = Command::new(env::current_exe().unwrap())
            .args([
                "--exact",
                "ffmpeg::tests::patch_discovery_uses_the_invoking_worktrees_manifest",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .env("CARGO_MANIFEST_DIR", &directory)
            .status()
            .unwrap();
        fs::remove_dir_all(directory).unwrap();
        assert!(status.success());
    }
}

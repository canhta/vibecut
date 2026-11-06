//! Platform FFmpeg, zlib, opus, libvpx and x264 configure and build recipes.
use super::{
    archives::sha256,
    command::{capture, run},
    fetch::{X264_URL, X264_VERSION, source, zlib_source},
};
use sha2::{Digest, Sha256};
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

pub(super) fn ffmpeg_configure_flags(
    prefix: &Path,
    target: &str,
    arch: &str,
) -> Result<Vec<String>, String> {
    let os = target
        .split('-')
        .next()
        .ok_or("FFmpeg target is missing an OS")?;
    let mut flags = vec![
        format!("--prefix={}", shell_path(prefix)?),
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
        // Static zlib is built from verified source; --disable-autodetect keeps
        // this the only compression library the recipe links.
        "--enable-zlib".into(),
    ];
    if os == "macos" {
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
    } else if os == "linux" {
        flags.extend([
            "--target-os=linux".into(),
            format!(
                "--arch={}",
                if arch == "arm64" { "aarch64" } else { "x86_64" }
            ),
            "--cc=clang".into(),
            "--cxx=clang++".into(),
            "--ld=clang".into(),
            "--ar=llvm-ar".into(),
            "--nm=llvm-nm".into(),
            "--enable-pic".into(),
            // The GPL build admits libx264 for software H.264 export.
            "--enable-gpl".into(),
            "--enable-libx264".into(),
            // libvpx/x264 are static and their pthread dependencies live in
            // Libs.private; normal pkg-config output omits them.
            "--pkg-config-flags=--static".into(),
            "--extra-cflags=-fPIC".into(),
            "--extra-ldflags=-Wl,-rpath,$ORIGIN/../lib".into(),
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
    Ok(flags)
}

pub(super) fn build_zlib(
    source: &Path,
    prefix: &Path,
    target: &str,
    arch: &str,
) -> Result<(), String> {
    let build = source.join(target);
    fs::create_dir_all(&build).map_err(|e| e.to_string())?;
    let mut configure = Command::new("cmake");
    configure
        .arg("-S")
        .arg(source)
        .arg("-B")
        .arg(&build)
        .arg(format!("-DCMAKE_INSTALL_PREFIX={}", prefix.display()))
        .args([
            "-DCMAKE_BUILD_TYPE=Release",
            "-DZLIB_BUILD_SHARED=OFF",
            "-DZLIB_BUILD_STATIC=ON",
            "-DZLIB_BUILD_TESTING=OFF",
            "-DCMAKE_POSITION_INDEPENDENT_CODE=ON",
        ]);
    if cfg!(target_os = "macos") {
        configure.args([
            format!("-DCMAKE_OSX_ARCHITECTURES={arch}"),
            "-DCMAKE_OSX_DEPLOYMENT_TARGET=11.0".into(),
        ]);
    } else {
        configure.args(["-G", "Ninja", "-DCMAKE_C_COMPILER=clang"]);
    }
    run(&mut configure)?;
    run(Command::new("cmake")
        .arg("--build")
        .arg(&build)
        .args(["--parallel", "3"]))?;
    run(Command::new("cmake").arg("--install").arg(&build))?;
    if cfg!(windows) {
        // zlib 1.4 names its Windows static archive libzs.a, but its
        // pkg-config template still requests -lz (the shared import library).
        // Point FFmpeg at our static archive instead of a system zlib.
        let metadata = prefix.join("lib/pkgconfig/zlib.pc");
        let contents = fs::read_to_string(&metadata).map_err(|e| e.to_string())?;
        fs::write(
            metadata,
            contents
                .replace(" -lz\n", " -lzs\n")
                .replace(" -lz\r\n", " -lzs\r\n"),
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}
pub(super) fn build_source(
    base: &Path,
    target: &str,
    version: &str,
    opus: &str,
    vpx: &str,
    zlib: &str,
) -> Result<(), String> {
    let patches = patches()?;
    let patch_id = patch_id(&patches)?;
    let prefix = base.join(format!(
        "{version}-opus{opus}-vpx{vpx}-zlib{zlib}-{patch_id}"
    ));
    if prefix.join("BUILD.txt").exists() {
        record_patches(&prefix, &patches)?;
        if cfg!(target_os = "macos") {
            tool_runpaths(&prefix)?;
        }
        fs::write(base.join("current"), prefix.as_os_str().as_encoded_bytes())
            .map_err(|e| e.to_string())?;
        println!("FFmpeg {version} + zlib {zlib}: {}", prefix.display());
        return Ok(());
    }
    let mut receipt = format!("FFmpeg shared build for {target}\n\n");
    // Verify every pinned source before creating the install prefix, so a bad
    // download can never leave a prefix that later provisioning mistakes for a
    // completed build.
    let zl = zlib_source(base, zlib, &mut receipt).map_err(|error| error.to_string())?;
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
    // Linux carries a software H.264 encoder; the GPL build admits libx264.
    let x264 = if cfg!(target_os = "linux") {
        Some(source(
            &sources,
            "x264",
            X264_VERSION,
            X264_URL,
            &mut receipt,
        )?)
    } else {
        None
    };
    let arch = if target.ends_with("aarch64") {
        "arm64"
    } else {
        "x86_64"
    };
    fs::create_dir_all(&prefix).map_err(|e| e.to_string())?;
    build_zlib(&zl, &prefix, target, arch)?;
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
            } else if cfg!(target_os = "linux") {
                format!("{arch}-linux-gcc")
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
    if let Some(x264) = &x264 {
        let build = x264.join(target);
        fs::create_dir_all(&build).map_err(|e| e.to_string())?;
        run(Command::new(build_tool("bash")?)
            .arg(shell_path(&x264.join("configure"))?)
            .current_dir(&build)
            .args([
                format!("--prefix={}", shell_path(&prefix)?),
                "--enable-static".into(),
                "--enable-pic".into(),
                "--disable-cli".into(),
                "--disable-opencl".into(),
            ]))?;
        run(Command::new(build_tool("make")?)
            .current_dir(&build)
            .arg("-j3"))?;
        run(Command::new(build_tool("make")?)
            .current_dir(&build)
            .arg("install"))?;
    }
    let ffbuild = ff.join(target);
    fs::create_dir_all(&ffbuild).map_err(|e| e.to_string())?;
    let flags = ffmpeg_configure_flags(&prefix, target, arch)?;
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
    let gpl_admitted = cfg!(target_os = "linux") || license.contains("#define CONFIG_GPL 0");
    if !gpl_admitted || !license.contains("#define CONFIG_NONFREE 0") {
        return Err("FFmpeg build failed licence admission".into());
    }
    let compliance = prefix.join("compliance");
    fs::create_dir_all(&compliance).map_err(|e| e.to_string())?;
    for (source, name) in [
        (&ff, "COPYING.LGPLv2.1"),
        (&ff, "LICENSE.md"),
        (&op, "COPYING"),
        (&vp, "LICENSE"),
        (&zl, "LICENSE"),
    ]
    .into_iter()
    .chain(x264.iter().map(|x| (x, "COPYING")))
    {
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
    receipt.push_str(&format!(
        "zlib flags: static, PIC, architecture {arch}\nopus flags: static PIC, DNN off, architecture {arch}\nlibvpx flags: {vpflags:?}\nFFmpeg configure: {flags:?}\n"
    ));
    fs::write(prefix.join("BUILD.txt"), receipt).map_err(|e| e.to_string())?;
    fs::write(base.join("current"), prefix.as_os_str().as_encoded_bytes())
        .map_err(|e| e.to_string())?;
    println!("FFmpeg {version} + zlib {zlib}: {}", prefix.display());
    Ok(())
}
pub(super) fn tool_runpaths(prefix: &Path) -> Result<(), String> {
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

pub(super) fn patches() -> Result<Vec<PathBuf>, String> {
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
pub(super) fn patch_id(patches: &[PathBuf]) -> Result<String, String> {
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
pub(super) fn build_tool(name: &str) -> Result<PathBuf, String> {
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

pub(super) fn shell_path(path: &Path) -> Result<String, String> {
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
pub(super) fn record_patches(prefix: &Path, patches: &[PathBuf]) -> Result<(), String> {
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

//! Host discovery, install prefix, native runtime paths and compiler environment.
use super::{archive_cache, archives::sha256, recipes::{patch_id, patches}};
use std::{env, ffi::OsStr, fs, path::{Path, PathBuf}, process::Command};

pub fn host() -> Result<String, String> {
    let os = match env::consts::OS {
        "macos" => "macos",
        "windows" => "windows",
        "linux" => "linux",
        other => return Err(format!("unsupported host {other}")),
    };
    Ok(format!("{os}-{}", env::consts::ARCH))
}

pub(crate) fn cache() -> Result<PathBuf, String> {
    archive_cache("ffmpeg")
}

/// Append one encoded rustc flag without dropping the caller's flags.
pub(crate) fn append_encoded_flag(command: &mut Command, flag: &str) {
    let existing = command
        .get_envs()
        .filter(|(key, _)| *key == std::ffi::OsStr::new("CARGO_ENCODED_RUSTFLAGS"))
        .last()
        .and_then(|(_, value)| value)
        .and_then(|value| value.to_str())
        .map(str::to_owned);
    let mut flags = existing.unwrap_or_else(|| {
        let mut flags = env::var("CARGO_ENCODED_RUSTFLAGS").unwrap_or_default();
        if flags.is_empty() {
            flags = env::var("RUSTFLAGS")
                .unwrap_or_default()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join("\x1f");
        }
        flags
    });
    if !flags.is_empty() {
        flags.push('\x1f');
    }
    flags.push_str(flag);
    command.env("CARGO_ENCODED_RUSTFLAGS", flags);
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

/// External tools must load their own libraries, not the provisioned SDK that
/// an inherited wrapped environment points at.
pub fn external(program: impl AsRef<OsStr>) -> Command {
    let mut command = Command::new(program);
    command
        .env_remove("DYLD_LIBRARY_PATH")
        .env_remove("LD_LIBRARY_PATH");
    command
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
    } else if cfg!(target_os = "linux") {
        "LD_LIBRARY_PATH"
    } else {
        "PATH"
    };
    let runtime = if cfg!(windows) {
        windows_runtime(prefix)?
    } else {
        prefix.join("lib")
    };
    let mut paths = vec![runtime.clone()];
    let managed_bin = prefix.join("bin").canonicalize().ok();
    if let Some(old) = env::var_os(library_var) {
        paths.extend(env::split_paths(&old).filter(|path| {
            // An inherited native bin must not reintroduce the managed CLI.
            path != &runtime
                && (!cfg!(windows)
                    || managed_bin.is_none()
                    || path.canonicalize().ok() != managed_bin)
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
                crate::sherpa::provision(root, env::consts::ARCH, super::fetch_limited)?,
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
        let link_arg = format!(
            "link-arg=-Wl,-rpath,{}",
            prefix
                .join("lib")
                .to_str()
                .ok_or("FFmpeg cache path is not UTF-8")?
        );
        let arguments: Vec<_> = flags.split('\x1f').collect();
        let has_runpath = arguments.windows(2).any(|pair| pair == ["-C", &link_arg])
            || arguments
                .iter()
                .any(|arg| arg.strip_prefix("-C") == Some(link_arg.as_str()));
        if !has_runpath {
            if !flags.is_empty() {
                flags.push('\x1f');
            }
            flags.push_str("-C\x1f");
            flags.push_str(&link_arg);
        }
        command.env("CARGO_ENCODED_RUSTFLAGS", flags);
    }
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        // candle's gemm-f16 needs ARMv8.2 FP16, which rustc leaves off the
        // baseline aarch64 target; supported Linux ARM hosts implement it.
        append_encoded_flag(command, "-C\x1ftarget-feature=+fp16");
    }
    Ok(())
}

/// Expose native DLLs without putting the managed CLI ahead of external test oracles.
/// Hard links publish complete files atomically, including across concurrent xtasks,
/// and retain the exact library bytes from this immutable native installation.
pub(super) fn windows_runtime(prefix: &Path) -> Result<PathBuf, String> {
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
pub(super) fn windows_bindgen_environment(command: &mut Command, include: &std::ffi::OsStr) {
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

/// Source provisioning runs on the target host: Windows needs MSYS2 and Linux
/// needs the distribution toolchain, so cross-hosts are refused.
pub(super) fn native_target(host: &str, target: &str) -> Result<(), String> {
    if target == host {
        Ok(())
    } else {
        Err("source provisioning must run on the target host/architecture".into())
    }
}

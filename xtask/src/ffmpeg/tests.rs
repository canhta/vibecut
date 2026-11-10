//! Provisioning, environment and source-recipe tests for the native media SDK.
use super::archives::sha256;
use super::command::*;
use super::environment::*;
use super::fetch::*;
use super::recipes::*;
use super::verify::*;
use super::{Version, inventory, parse_version};
use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

#[test]
fn external_tools_drop_the_managed_runtime_search_path() {
    let command = external("ffmpeg");
    let cleared: Vec<_> = command
        .get_envs()
        .filter(|(_, value)| value.is_none())
        .map(|(key, _)| key.to_string_lossy().into_owned())
        .collect();
    for variable in ["DYLD_LIBRARY_PATH", "LD_LIBRARY_PATH"] {
        assert!(
            cleared.iter().any(|key| key == variable),
            "{variable} was not cleared for an external tool"
        );
    }
}

#[cfg(target_os = "macos")]
#[test]
fn inherited_native_environment_keeps_flags_and_library_paths_stable() {
    const CHILD: &str = "VIBECUT_NATIVE_ENV_STABILITY_CHILD";
    const TEST: &str =
        "ffmpeg::tests::inherited_native_environment_keeps_flags_and_library_paths_stable";
    let prefix = env::temp_dir().join("vibecut SDK with spaces");
    if env::var_os(CHILD).is_some() {
        let mut command = Command::new("unused");
        environment_at(&mut command, &prefix).unwrap();
        for variable in ["CARGO_ENCODED_RUSTFLAGS", "DYLD_LIBRARY_PATH"] {
            let reapplied = command
                .get_envs()
                .find(|(key, _)| *key == variable)
                .and_then(|(_, value)| value);
            assert_eq!(reapplied, env::var_os(variable).as_deref(), "{variable}");
        }
        return;
    }
    let mut command = Command::new(env::current_exe().unwrap());
    command
        .args(["--exact", TEST, "--nocapture"])
        .env(CHILD, "1");
    environment_at(&mut command, &prefix).unwrap();
    assert!(command.status().unwrap().success());
}

#[cfg(windows)]
#[test]
fn cargo_children_resolve_external_oracles_with_native_dlls() {
    const CHILD: &str = "VIBECUT_ORACLE_ENV_CHILD";
    const TEST: &str = "ffmpeg::tests::cargo_children_resolve_external_oracles_with_native_dlls";
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
    let directory = tempfile::tempdir().unwrap();
    let base = directory
        .path()
        .join(".cache/vibecut/ffmpeg")
        .join(host().unwrap());
    let prefix = base.join(format!(
        "fixture-{}",
        patch_id(&patches().unwrap()).unwrap()
    ));
    let external = directory.path().join("external");
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
        .env("USERPROFILE", directory.path())
        .env("SHERPA_ONNX_LIB_DIR", directory.path())
        .env("PATH", env::join_paths(paths).unwrap())
        .status()
        .unwrap();
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
    let directory = tempfile::Builder::new()
        .prefix("vibecut provision Đặng space-")
        .tempdir()
        .unwrap();
    let variable = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    for value in [
        directory.path().as_os_str(),
        std::ffi::OsStr::new("relative"),
        std::ffi::OsStr::new(""),
    ] {
        let valid = value == directory.path().as_os_str();
        let status = Command::new(env::current_exe().unwrap())
            .args([
                "--exact",
                "ffmpeg::tests::provisioning_paths_preserve_native_home_and_shell_arguments",
                "--nocapture",
            ])
            .env(
                CHILD,
                if valid {
                    directory.path().as_os_str()
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
}

#[test]
fn oracle_versions_and_fixture_inventory_are_explicit() {
    assert_eq!(
        parse_version("ffmpeg version 7.1.1 Copyright"),
        Some(Version { major: 7, minor: 1 })
    );
    assert_eq!(
        parse_version("ffmpeg version n6.0-static"),
        Some(Version { major: 6, minor: 0 })
    );
    assert_eq!(parse_version("ffmpeg version N-123-gabc"), None);
    assert_eq!(parse_version("not a version"), None);
    let workspace = tempfile::Builder::new()
        .prefix("vibecut-fixtures-")
        .tempdir()
        .unwrap();
    let fixtures = workspace.path().join("target/fixtures/nested");
    fs::create_dir_all(&fixtures).unwrap();
    fs::write(fixtures.join("one.bin"), b"abc").unwrap();
    let list = inventory(workspace.path()).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].relative, PathBuf::from("nested/one.bin"));
    assert_eq!(list[0].bytes, 3);
    assert_eq!(
        list[0].sha256,
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
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
    let directory = tempfile::tempdir().unwrap();
    fs::create_dir_all(directory.path().join("patches")).unwrap();
    fs::write(
        directory.path().join("patches/runtime.patch"),
        b"runtime fixture",
    )
    .unwrap();
    let status = Command::new(env::current_exe().unwrap())
        .args([
            "--exact",
            "ffmpeg::tests::patch_discovery_uses_the_invoking_worktrees_manifest",
            "--nocapture",
        ])
        .env(CHILD, "1")
        .env("CARGO_MANIFEST_DIR", directory.path())
        .status()
        .unwrap();
    assert!(status.success());
}

#[test]
fn ffmpeg_recipe_keeps_disable_autodetect_and_platform_licence_admission() {
    let prefix = env::temp_dir();
    for (target, arch, gpl) in [
        ("macos-aarch64", "arm64", false),
        ("macos-x86_64", "x86_64", false),
        ("windows-x86_64", "x86_64", false),
        ("windows-aarch64", "arm64", false),
        ("linux-x86_64", "x86_64", true),
        ("linux-aarch64", "aarch64", true),
    ] {
        let flags = ffmpeg_configure_flags(&prefix, target, arch).unwrap();
        for required in [
            "--disable-autodetect",
            "--disable-nonfree",
            "--enable-zlib",
            "--disable-gpl",
        ] {
            assert!(
                flags.iter().any(|flag| flag == required),
                "{target}: missing {required}"
            );
        }
        assert_eq!(
            flags.iter().any(|flag| flag == "--enable-gpl"),
            gpl,
            "{target}: GPL admission"
        );
        assert_eq!(
            flags.iter().any(|flag| flag == "--enable-libx264"),
            gpl,
            "{target}: libx264 admission"
        );
    }
}

#[test]
fn zlib_version_participates_in_the_media_cache_key() {
    let current = cache_key_for("9.0.2", "1.6.1", "1.17.0", "1.4");
    assert_eq!(
        current,
        "ffmpeg9.0.2-opus1.6.1-vpx1.17.0-zlib1.4-x26420191217-2245"
    );
    assert_ne!(current, cache_key_for("9.0.2", "1.6.1", "1.17.0", "1.5"));
}

#[test]
fn unpinned_zlib_version_is_refused() {
    assert_eq!(
        pinned_zlib_sha256("1.5").unwrap_err(),
        SourceError::NoPinnedChecksum {
            name: "zlib",
            version: "1.5".into(),
        }
    );
}

#[test]
fn zlib_checksum_mismatch_returns_a_typed_error_and_removes_the_archive() {
    let base = tempfile::tempdir().unwrap();
    fs::create_dir_all(base.path().join("sources")).unwrap();
    fs::write(
        base.path().join("sources/zlib-1.4.archive"),
        b"tampered zlib",
    )
    .unwrap();
    let actual = sha256(&base.path().join("sources/zlib-1.4.archive")).unwrap();
    let mut receipt = String::new();
    let error = zlib_source(base.path(), "1.4", &mut receipt).unwrap_err();
    assert_eq!(
        error,
        SourceError::ChecksumMismatch {
            name: "zlib",
            version: "1.4".into(),
            expected: pinned_zlib_sha256("1.4").unwrap(),
            actual,
        }
    );
    assert!(!base.path().join("sources/zlib-1.4.archive").exists());
    assert!(!base.path().join("sources/zlib-1.4").exists());
    assert!(receipt.is_empty());
}

#[test]
fn zlib_checksum_mismatch_leaves_no_provisioned_prefix() {
    let base = tempfile::tempdir().unwrap();
    fs::create_dir_all(base.path().join("sources")).unwrap();
    fs::write(
        base.path().join("sources/zlib-1.4.archive"),
        b"tampered zlib",
    )
    .unwrap();
    let patch_id = patch_id(&patches().unwrap()).unwrap();
    let prefix = base
        .path()
        .join(format!("1.4-opus1.0-vpx1.0-zlib1.4-{patch_id}"));
    let error = build_source(base.path(), "linux-aarch64", "1.4", "1.0", "1.0", "1.4").unwrap_err();
    assert!(
        error.contains("zlib 1.4 source checksum mismatch"),
        "{error}"
    );
    assert!(!prefix.exists());
    assert!(!base.path().join("sources/zlib-1.4.archive").exists());
}

#[test]
fn source_provisioning_admits_only_the_running_host() {
    assert!(native_target("linux-aarch64", "linux-aarch64").is_ok());
    assert!(native_target("windows-x86_64", "windows-x86_64").is_ok());
    assert_eq!(
        native_target("linux-aarch64", "linux-x86_64").unwrap_err(),
        "source provisioning must run on the target host/architecture"
    );
}

//! macOS deployment, architecture, runpath, signing and package audits.
use super::package::{self, compliance, files_with};
use super::{
    Options, Result, Tools, arg, copy, digest, entries, executable, insert, manifest, signing,
    symlink,
};
use serde_json::json;
use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

pub(super) const MINIMUM: &str = "11.0";
pub(super) fn audit(tools: &mut Tools, path: &Path, arch: &str) -> Result<()> {
    run!(tools, "lipo", "-info", path)?;
    let output = run!(tools, "lipo", "-archs", path)?;
    let actual: BTreeSet<_> = output.split_whitespace().collect();
    let expected: BTreeSet<_> = match arch {
        "aarch64" => vec!["arm64"],
        "x86_64" => vec!["x86_64"],
        "universal" => vec!["arm64", "x86_64"],
        _ => return Err("unsupported macOS architecture".into()),
    }
    .into_iter()
    .collect();
    if actual != expected {
        return Err(format!("wrong Mach-O architecture: {}", path.display()));
    }
    deployment(&run!(tools, "otool", "-l", path)?, expected.len())
}
fn deployment(commands: &str, slices: usize) -> Result<()> {
    let mut versions = Vec::new();
    let mut admitted = false;
    for line in commands.lines() {
        let words: Vec<_> = line.split_whitespace().collect();
        match words.as_slice() {
            ["cmd", kind] => {
                admitted = matches!(*kind, "LC_BUILD_VERSION" | "LC_VERSION_MIN_MACOSX")
            }
            ["minos" | "version", version] if admitted => {
                let numbers = version
                    .split('.')
                    .map(str::parse::<u32>)
                    .collect::<std::result::Result<Vec<_>, _>>()
                    .map_err(|_| "invalid Mach-O deployment version")?;
                let major = *numbers.first().ok_or("missing deployment major")?;
                let minor = *numbers.get(1).ok_or("missing deployment minor")?;
                let patch = numbers.get(2).copied().unwrap_or(0);
                if numbers.len() > 3 {
                    return Err("invalid Mach-O deployment version".into());
                }
                if (major, minor, patch) > (11, 0, 0) {
                    return Err(format!("Mach-O deployment target exceeds macOS {MINIMUM}"));
                }
                versions.push((major, minor));
                admitted = false;
            }
            _ => {}
        }
    }
    if versions.len() != slices {
        return Err("missing/duplicate deployment version for Mach-O slice".into());
    }
    Ok(())
}
pub(super) fn dependencies(tools: &mut Tools, path: &Path) -> Result<Vec<String>> {
    Ok(run!(tools, "otool", "-L", path)?
        .lines()
        .filter(|line| line.starts_with(['\t', ' ']))
        .map(|line| {
            line.trim()
                .split(" (compatibility")
                .next()
                .unwrap_or_default()
                .to_owned()
        })
        .collect())
}
pub(super) fn rpaths(commands: &str) -> Vec<String> {
    let mut active = false;
    let mut result = BTreeSet::new();
    for line in commands.lines() {
        let trimmed = line.trim();
        if let Some(kind) = trimmed.strip_prefix("cmd ") {
            active = kind == "LC_RPATH";
        }
        if active
            && let Some(value) = trimmed
                .strip_prefix("path ")
                .and_then(|value| value.split(" (offset").next())
        {
            result.insert(value.to_owned());
            active = false;
        }
    }
    result.into_iter().collect()
}
pub(super) fn relocate(tools: &mut Tools, files: &[PathBuf], libraries: &[PathBuf]) -> Result<()> {
    let names: BTreeSet<_> = libraries
        .iter()
        .filter_map(|path| path.file_name())
        .collect();
    for path in files {
        if libraries.contains(path) {
            let name = path
                .file_name()
                .and_then(OsStr::to_str)
                .ok_or("library filename")?;
            run!(
                tools,
                "install_name_tool",
                "-id",
                format!("@rpath/{name}"),
                path
            )?;
        }
        for dependency in dependencies(tools, path)? {
            let name = Path::new(&dependency)
                .file_name()
                .ok_or("dependency filename")?;
            if names.contains(name) {
                let name = name.to_str().ok_or("dependency name UTF-8")?;
                let desired = format!("@rpath/{name}");
                if dependency != desired {
                    run!(
                        tools,
                        "install_name_tool",
                        "-change",
                        dependency,
                        desired,
                        path
                    )?;
                }
            } else if !dependency.starts_with("/System/Library/")
                && !dependency.starts_with("/usr/lib/")
            {
                return Err(format!("unbundled macOS dependency: {dependency}"));
            }
        }
        for old in rpaths(&run!(tools, "otool", "-l", path)?) {
            run!(tools, "install_name_tool", "-delete_rpath", old, path)?;
        }
        let location = if libraries.contains(path) {
            "@loader_path"
        } else {
            "@executable_path/../Frameworks"
        };
        run!(tools, "install_name_tool", "-add_rpath", location, path)?;
    }
    Ok(())
}
pub(super) fn sign(tools: &mut Tools, path: &Path, identity: &str) -> Result<()> {
    let mut identifier = "com.canhta.vibecut".to_owned();
    match path.file_name().and_then(OsStr::to_str) {
        Some("vibecut-cli") => identifier.push_str(".cli"),
        _ => match path.extension().and_then(OsStr::to_str) {
            Some("dylib") => {
                identifier.push_str(".ffmpeg.");
                identifier.push_str(
                    path.file_stem()
                        .and_then(OsStr::to_str)
                        .ok_or("library stem")?,
                );
            }
            Some("dmg") => identifier.push_str(".diskimage"),
            _ => {}
        },
    }
    let mut args = vec![
        arg("--force"),
        arg("--sign"),
        arg(identity),
        arg("--identifier"),
        arg(identifier),
    ];
    if identity != "-" {
        args.push(arg("--timestamp"));
        if path.extension() != Some(OsStr::new("dmg")) {
            args.extend([arg("--options"), arg("runtime")]);
        }
    }
    args.push(arg(path));
    tools.run("codesign", args)?;
    run!(tools, "codesign", "--verify", "--deep", "--strict", path)?;
    Ok(())
}
pub(super) fn replace(tools: &mut Tools, o: &Options) -> Result<()> {
    let app = o.path("--app-copy")?;
    let replacement = o.path("--libraries")?;
    let libs = package::files_with(&app.join("Contents/Frameworks"), "dylib")?;
    if libs.len() < 6 {
        return Err("expected a disposable app copy with bundled FFmpeg".into());
    }
    for library in &libs {
        if !replacement
            .join(library.file_name().ok_or("library name")?)
            .is_file()
        {
            return Err("missing replacement library".into());
        }
    }
    for library in &libs {
        copy(
            &replacement.join(library.file_name().ok_or("library name")?),
            library,
        )?;
    }
    relocate(tools, &libs, &libs)?;
    for path in libs
        .into_iter()
        .chain(entries(&app.join("Contents/MacOS"))?)
    {
        sign(tools, &path, "-")?;
    }
    sign(tools, &app, "-")
}

pub(super) fn package(
    tools: &mut Tools,
    r: &package::Request,
    config: &signing::Config,
    prefix: &Path,
    output: &Path,
    receipt: &mut serde_json::Value,
) -> Result<()> {
    let payload = output.join("root");
    let app = payload.join("Applications/VibeCut.app");
    let contents = app.join("Contents");
    let binaries = contents.join("MacOS");
    let libraries = contents.join("Frameworks");
    let resources = contents.join("Resources");
    for path in [&binaries, &libraries, &resources] {
        fs::create_dir_all(path).map_err(|e| e.to_string())?;
    }
    for name in ["vibecut", "vibecut-cli"] {
        copy(&r.binaries.join(name), &binaries.join(name))?;
        executable(&binaries.join(name))?;
    }
    for path in files_with(&prefix.join("lib"), "dylib")? {
        let deps = dependencies(tools, &path)?;
        let name = Path::new(deps.first().ok_or("missing dylib install ID")?)
            .file_name()
            .ok_or("dylib name")?;
        copy(&path, &libraries.join(name))?;
        executable(&libraries.join(name))?;
    }
    let libs = files_with(&libraries, "dylib")?;
    if libs.len() < 6 {
        return Err("incomplete FFmpeg dylib set".into());
    }
    let files: Vec<_> = libs
        .iter()
        .cloned()
        .chain([binaries.join("vibecut-cli"), binaries.join("vibecut")])
        .collect();
    for path in &files {
        audit(tools, path, &r.arch)?;
    }
    relocate(tools, &files, &libs)?;
    copy(
        &tools.root.join("assets/app-icon/vibecut.icns"),
        &resources.join("vibecut.icns"),
    )?;
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd"><plist version="1.0"><dict><key>CFBundleExecutable</key><string>vibecut</string><key>CFBundleIdentifier</key><string>com.canhta.vibecut</string><key>CFBundleName</key><string>VibeCut</string><key>CFBundlePackageType</key><string>APPL</string><key>CFBundleShortVersionString</key><string>{version}</string><key>CFBundleVersion</key><string>{version}</string><key>CFBundleIconFile</key><string>vibecut.icns</string><key>NSHighResolutionCapable</key><true/><key>LSMinimumSystemVersion</key><string>{minimum}</string></dict></plist>"#,
        version = tools.version(),
        minimum = MINIMUM
    );
    fs::write(contents.join("Info.plist"), plist).map_err(|e| e.to_string())?;
    compliance(tools, prefix, &resources.join("compliance"))?;
    fs::create_dir_all(payload.join("usr/local/bin")).map_err(|e| e.to_string())?;
    symlink(
        Path::new("/Applications/VibeCut.app/Contents/MacOS/vibecut-cli"),
        &payload.join("usr/local/bin/vibecut-cli"),
    )?;
    for path in &files {
        sign(tools, path, &config.identity)?;
    }
    sign(tools, &app, &config.identity)?;
    insert(
        receipt,
        "signing",
        json!(if config.identity == "-" {
            "ad-hoc"
        } else {
            "Developer ID requested"
        }),
    )?;
    let components = output.join("components.plist");
    fs::write(&components,r#"<?xml version="1.0"?><plist version="1.0"><array><dict><key>RootRelativeBundlePath</key><string>Applications/VibeCut.app</string><key>BundleIsRelocatable</key><false/><key>BundleHasStrictIdentifier</key><true/><key>BundleIsVersionChecked</key><true/><key>BundleOverwriteAction</key><string>upgrade</string></dict></array></plist>"#).map_err(|e|e.to_string())?;
    let pkg = output.join(format!("vibecut-{}-macos-{}.pkg", tools.version(), r.arch));
    let mut args = vec![
        arg("--root"),
        arg(&payload),
        arg("--identifier"),
        arg("com.canhta.vibecut"),
        arg("--version"),
        arg(tools.version()),
        arg("--install-location"),
        arg("/"),
        arg("--component-plist"),
        arg(&components),
    ];
    if let Some(identity) = &config.installer {
        args.extend([arg("--sign"), arg(identity)]);
    }
    args.push(arg(&pkg));
    tools.run("pkgbuild", args)?;
    if config.installer.is_some() {
        run!(tools, "pkgutil", "--check-signature", &pkg)?;
    }
    if config.notary.is_some() {
        let archive = output.join("notary-app.zip");
        run!(tools, "ditto", "-c", "-k", "--keepParent", &app, &archive)?;
        signing::notarize(tools, &archive, config)?;
        staple(tools, &app)?;
        run!(
            tools,
            "spctl",
            "--assess",
            "--type",
            "execute",
            "--verbose",
            &app
        )?;
        let image = output.join("image");
        fs::create_dir(&image).map_err(|e| e.to_string())?;
        run!(tools, "ditto", &app, image.join("VibeCut.app"))?;
        symlink(Path::new("/Applications"), &image.join("Applications"))?;
        for name in ["install-cli.sh", "README.md"] {
            copy(&tools.root.join("packaging").join(name), &image.join(name))?;
        }
        let dmg = output.join(format!("vibecut-{}-macos-{}.dmg", tools.version(), r.arch));
        run!(
            tools,
            "hdiutil",
            "create",
            "-volname",
            "VibeCut",
            "-srcfolder",
            &image,
            "-format",
            "UDZO",
            &dmg
        )?;
        sign(tools, &dmg, &config.identity)?;
        signing::notarize(tools, &dmg, config)?;
        staple(tools, &dmg)?;
        run!(
            tools,
            "spctl",
            "--assess",
            "--type",
            "open",
            "--context",
            "context:primary-signature",
            "--verbose",
            &dmg
        )?;
        insert(receipt, "image_sha256", json!(digest(&dmg)?))?;
        insert(receipt, "app_notarization", json!({"status":"Accepted"}))?;
        insert(receipt, "image_notarization", json!({"status":"Accepted"}))?;
        if config.installer.is_some() {
            signing::notarize(tools, &pkg, config)?;
            staple(tools, &pkg)?;
        }
    }
    insert(receipt, "package_sha256", json!(digest(&pkg)?))?;
    insert(receipt, "files", manifest(&payload)?)?;
    Ok(())
}
pub(super) fn staple(tools: &mut Tools, path: &Path) -> Result<()> {
    stapler_with_wait(tools, "staple", path, std::thread::sleep)?;
    validate_staple(tools, path)
}
pub(super) fn validate_staple(tools: &mut Tools, path: &Path) -> Result<()> {
    stapler_with_wait(tools, "validate", path, std::thread::sleep)
}
fn stapler_with_wait(
    tools: &mut Tools,
    operation: &str,
    path: &Path,
    mut wait: impl FnMut(Duration),
) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(180);
    for (attempt, delay) in [0, 10, 20, 30, 50, 60].into_iter().enumerate() {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let delay = Duration::from_secs(delay);
        if remaining <= delay {
            break;
        }
        if !delay.is_zero() {
            wait(delay);
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        let mut command = Command::new("xcrun");
        command.args([arg("stapler"), arg(operation), arg(path)]);
        command.current_dir(&tools.root);
        let captured = tools.execute(&mut command, remaining)?;
        if captured.code == Some(0) {
            return Ok(());
        }
        // EX_NOHOST covers unavailable tickets/unexpected ticket-service replies.
        // Invalid/revoked signatures, unsupported files and write errors fail now.
        if captured.code != Some(68) {
            return Err(format!(
                "stapler {operation} failed ({:?}); native diagnostics suppressed",
                captured.code
            ));
        }
        if attempt < 5 {
            eprintln!("stapler {operation}: ticket lookup unavailable; bounded retry");
        }
    }
    Err(format!(
        "stapler {operation}: ticket lookup unavailable after bounded retries; native diagnostics suppressed"
    ))
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod tests {
    use super::{dependencies, deployment, relocate, rpaths, stapler_with_wait};
    use crate::distribution::{Captured, Tools};
    use std::path::{Path, PathBuf};

    #[test]
    fn every_slice_needs_an_admitted_deployment_version() {
        for (versions, slices, valid) in [
            (vec!["11.0"], 1, true),
            (vec!["11.0.0"], 1, true),
            (vec!["11.0.1"], 1, false),
            (vec!["11.0.0.1"], 1, false),
            (vec!["11.0", "11.0.1"], 2, false),
            (vec!["10.15"], 1, true),
            (vec!["11.0", "11.0"], 2, true),
            (vec!["11.0"], 2, false),
            (vec!["11.0", "15.0"], 2, false),
        ] {
            let data = versions
                .iter()
                .map(|v| format!("cmd LC_BUILD_VERSION\n cmdsize 32\n platform 1\n minos {v}\n"))
                .collect::<String>()
                + "cmd LC_SOURCE_VERSION\n cmdsize 16\n version 1.0\n";
            assert_eq!(deployment(&data, slices).is_ok(), valid);
        }
    }
    #[test]
    fn duplicate_slice_runpaths_are_removed_once() {
        assert_eq!(
            rpaths(
                "cmd LC_RPATH\n cmdsize 80\n path /cache/lib (offset 12)\ncmd LC_RPATH\n cmdsize 80\n path /cache/lib (offset 12)"
            ),
            vec!["/cache/lib"]
        );
    }
    #[test]
    fn foreign_dependencies_and_slice_headers_are_not_accepted_as_system_libraries() {
        let mut tools = Tools::new(Path::new("."));
        tools.runner = Box::new(|_, _| {
            Ok(Captured{code:Some(0),stdout:"app (architecture x86_64):\n\t/opt/local/lib/foreign.dylib (compatibility version 1.0.0)\napp (architecture arm64):\n\t/opt/local/lib/foreign.dylib (compatibility version 1.0.0)\n".into(),stderr:String::new()})
        });
        let paths = dependencies(&mut tools, Path::new("app")).expect("dependencies");
        assert_eq!(paths, vec!["/opt/local/lib/foreign.dylib"; 2]);
        assert!(
            relocate(&mut tools, &[PathBuf::from("app")], &[])
                .expect_err("external library")
                .contains("unbundled")
        );
    }
    #[test]
    fn ticket_lookup_retries_are_bounded_and_do_not_retry_invalid_data() {
        for operation in ["staple", "validate"] {
            for (codes, expected_success, expected_delays) in [
                (vec![68, 68, 0], true, vec![10, 20]),
                (vec![68; 6], false, vec![10, 20, 30, 50, 60]),
                (vec![65], false, vec![]),
            ] {
                let mut tools = Tools::new(Path::new("."));
                let mut codes = std::collections::VecDeque::from(codes);
                tools.runner = Box::new(move |_, _| {
                    Ok(Captured {
                        code: codes.pop_front(),
                        stdout: "PRIVATE-STDOUT".into(),
                        stderr: "PRIVATE-STDERR".into(),
                    })
                });
                let mut delays = Vec::new();
                let result =
                    stapler_with_wait(&mut tools, operation, Path::new("private.app"), |delay| {
                        delays.push(delay.as_secs())
                    });
                assert_eq!(result.is_ok(), expected_success);
                assert_eq!(delays, expected_delays);
                if let Err(error) = result {
                    assert!(!error.contains("PRIVATE"));
                }
            }
        }
    }
}

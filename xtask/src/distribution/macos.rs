#[allow(clippy::wildcard_imports)]
use super::*;
use std::collections::BTreeSet;

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
                if (major, minor) > (11, 0) {
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
#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
#[allow(clippy::wildcard_imports)]
mod tests {
    use super::*;
    #[test]
    fn every_slice_needs_an_admitted_deployment_version() {
        for (versions, slices, valid) in [
            (vec!["11.0"], 1, true),
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
}

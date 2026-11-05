#[allow(clippy::wildcard_imports)]
use super::*;
use std::collections::BTreeSet;

pub(super) fn export(
    tools: &mut Tools,
    prefix: &Path,
    binaries: &Path,
    output: &Path,
) -> Result<()> {
    let output = fresh(output)?;
    tree(binaries, &output.join("binaries"))?;
    let sdk = output.join("sdk/prefix");
    fs::create_dir_all(sdk.join("lib")).map_err(|e| e.to_string())?;
    let verified = output.join("verified-compliance");
    package::compliance(tools, prefix, &verified)?;
    copy(&prefix.join("BUILD.txt"), &sdk.join("BUILD.txt"))?;
    tree(&prefix.join("compliance"), &sdk.join("compliance"))?;
    tree(&verified.join("sources"), &output.join("sdk/sources"))?;
    fs::remove_dir_all(verified).map_err(|e| e.to_string())?;
    for path in package::files_with(&prefix.join("lib"), "dylib")? {
        let dependencies = macos::dependencies(tools, &path)?;
        let name = Path::new(dependencies.first().ok_or("dylib install ID missing")?)
            .file_name()
            .ok_or("dylib name")?;
        copy(&path, &sdk.join("lib").join(name))?;
    }
    let libraries = package::files_with(&sdk.join("lib"), "dylib")?;
    let files: Vec<_> = libraries
        .iter()
        .cloned()
        .chain([
            output.join("binaries/vibecut"),
            output.join("binaries/vibecut-cli"),
        ])
        .collect();
    macos::relocate(tools, &files, &libraries)
}
fn names(root: &Path) -> Result<BTreeSet<OsString>> {
    Ok(package::files_with(&root.join("sdk/prefix/lib"), "dylib")?
        .iter()
        .filter_map(|p| p.file_name().map(OsStr::to_owned))
        .collect())
}
fn admission(arm: &Path, intel: &Path) -> Result<BTreeSet<OsString>> {
    let first: Value = serde_json::from_str(&text(&arm.join("binaries/build.json"))?)
        .map_err(|e| e.to_string())?;
    let second: Value = serde_json::from_str(&text(&intel.join("binaries/build.json"))?)
        .map_err(|e| e.to_string())?;
    if first != second {
        return Err("native executable build revisions/profiles differ".into());
    }
    if package::sources(&text(&arm.join("sdk/prefix/BUILD.txt"))?)?
        != package::sources(&text(&intel.join("sdk/prefix/BUILD.txt"))?)?
    {
        return Err("native media source identities differ".into());
    }
    let libs = names(arm)?;
    if libs.len() < 6 || libs != names(intel)? {
        return Err("native FFmpeg dylib sets differ".into());
    }
    Ok(libs)
}
fn combine(tools: &mut Tools, arm: &Path, intel: &Path, output: &Path) -> Result<()> {
    let names = admission(arm, intel)?;
    let output = fresh(output)?;
    for (root, arch) in [(arm, "aarch64"), (intel, "x86_64")] {
        let destination = output.join(format!("verify-{arch}"));
        package::compliance(tools, &root.join("sdk/prefix"), &destination)?;
        fs::remove_dir_all(destination).map_err(|e| e.to_string())?;
    }
    tree(&arm.join("sdk"), &output.join("sdk"))?;
    tree(&arm.join("binaries"), &output.join("binaries"))?;
    for (root, arch) in [(arm, "aarch64"), (intel, "x86_64")] {
        copy(
            &root.join("sdk/prefix/BUILD.txt"),
            &output
                .join("sdk/prefix/compliance/architectures")
                .join(format!("{arch}-BUILD.txt")),
        )?;
    }
    let paths = std::iter::once(PathBuf::from("binaries/vibecut"))
        .chain([PathBuf::from("binaries/vibecut-cli")])
        .chain(
            names
                .into_iter()
                .map(|name| Path::new("sdk/prefix/lib").join(name)),
        );
    for relative in paths {
        macos::audit(tools, &arm.join(&relative), "aarch64")?;
        macos::audit(tools, &intel.join(&relative), "x86_64")?;
        let target = output.join(&relative);
        fs::remove_file(&target).map_err(|e| e.to_string())?;
        run!(
            tools,
            "lipo",
            "-create",
            arm.join(&relative),
            intel.join(&relative),
            "-output",
            &target
        )?;
        executable(&target)?;
        macos::audit(tools, &target, "universal")?;
    }
    Ok(())
}
fn finish(
    tools: &mut Tools,
    arch: &str,
    prefix: &Path,
    binaries: &Path,
    signed: bool,
) -> Result<()> {
    let request = package::Request {
        os: match tools.host {
            Host::Mac => "macos",
            Host::Windows => "windows",
            _ => "linux",
        }
        .into(),
        arch: arch.into(),
        prefix: prefix.into(),
        binaries: binaries.into(),
        output: tools.root.join("target/package"),
    };
    if signed {
        signing::package(tools, &request)?;
    } else {
        package::package(tools, &request, &signing::Config::unsigned())?;
    }
    let suffix = if signed {
        "dmg"
    } else if tools.host == Host::Mac {
        "pkg"
    } else if tools.host == Host::Linux {
        "gz"
    } else {
        "zip"
    };
    let files = package::files_with(&request.output, suffix)?;
    if files.len() != 1 {
        return Err("expected exactly one distribution package".into());
    }
    let file = files.first().ok_or("package missing")?;
    let name = file.file_name().ok_or("package filename")?;
    let dist = tools.root.join("target/dist");
    copy(file, &dist.join(name))?;
    fs::write(
        dist.join(format!("{}.sha256", name.to_string_lossy())),
        format!("{}  {}\n", digest(file)?, name.to_string_lossy()),
    )
    .map_err(|e| e.to_string())
}
pub(super) fn native(tools: &mut Tools, o: &Options) -> Result<()> {
    let arch = o.get("--arch")?;
    if arch != env::consts::ARCH {
        return Err("native runner architecture differs from distribution".into());
    }
    let prefix = crate::ffmpeg::prefix()?;
    let inputs = tools.root.join("target/inputs");
    build(tools, &prefix, &inputs, false)?;
    if tools.host == Host::Mac {
        let output = tools.root.join("target/native");
        export(tools, &prefix, &inputs, &output)?;
    }
    finish(tools, arch, &prefix, &inputs, o.has("--signed"))?;
    let helper = tools
        .root
        .join("target/smoke-tool")
        .join(if tools.host == Host::Windows {
            "xtask.exe"
        } else {
            "xtask"
        });
    copy(&env::current_exe().map_err(|e| e.to_string())?, &helper)?;
    executable(&helper)
}
pub(super) fn universal(tools: &mut Tools, o: &Options) -> Result<()> {
    if tools.host != Host::Mac {
        return Err("universal assembly requires macOS".into());
    }
    let output = tools.root.join("target/universal");
    combine(
        tools,
        &tools.root.join("target/raw/aarch64"),
        &tools.root.join("target/raw/x86_64"),
        &output,
    )?;
    finish(
        tools,
        "universal",
        &output.join("sdk/prefix"),
        &output.join("binaries"),
        o.has("--signed"),
    )
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
    fn incompatible_native_inputs_refuse_before_merging() {
        for mismatch in ["revision", "sources", "libraries"] {
            let temp = Temp::new(&env::temp_dir()).expect("temp");
            for arch in ["arm", "intel"] {
                let root = temp.0.join(arch);
                fs::create_dir_all(root.join("binaries")).expect("binaries");
                fs::create_dir_all(root.join("sdk/prefix/lib")).expect("libraries");
                let revision = if mismatch == "revision" && arch == "intel" {
                    "other"
                } else {
                    "same"
                };
                write_json(
                    &root.join("binaries/build.json"),
                    &json!({"revision":revision,"profile":"release"}),
                )
                .expect("build record");
                let mut receipt = String::new();
                for name in ["ffmpeg", "opus", "libvpx", "zlib"] {
                    let version = if mismatch == "sources" && arch == "intel" && name == "ffmpeg" {
                        "2.0"
                    } else {
                        "1.0"
                    };
                    receipt.push_str(&format!(
                        "{name}: {version}\nsource: https://example.invalid/source\nsha256: {}\n",
                        "0".repeat(64)
                    ));
                }
                fs::write(root.join("sdk/prefix/BUILD.txt"), receipt).expect("receipt");
                let count = if mismatch == "libraries" && arch == "intel" {
                    7
                } else {
                    6
                };
                for index in 0..count {
                    fs::write(
                        root.join(format!("sdk/prefix/lib/lib{index}.dylib")),
                        b"fixture",
                    )
                    .expect("dylib");
                }
            }
            assert!(
                admission(&temp.0.join("arm"), &temp.0.join("intel")).is_err(),
                "{mismatch}"
            );
        }
    }
}

//! Windows packaging: PE audit, bundled DLL set, signing and ZIP archive.
use super::package::{self, Request};
use super::{Result, Tools, copy, digest, entries, insert, manifest, write_json};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

fn pe(path: &Path) -> Result<(u16, u16)> {
    let mut file = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut header = [0_u8; 64];
    file.read_exact(&mut header).map_err(|e| e.to_string())?;
    if header.get(..2) != Some(b"MZ") {
        return Err("not a PE image".into());
    }
    let offset = u32::from_le_bytes(
        header
            .get(0x3c..0x40)
            .ok_or("PE offset")?
            .try_into()
            .map_err(|_| "PE offset")?,
    );
    file.seek(SeekFrom::Start(u64::from(offset)))
        .map_err(|e| e.to_string())?;
    let mut image = [0_u8; 96];
    file.read_exact(&mut image).map_err(|e| e.to_string())?;
    if image.get(..4) != Some(b"PE\0\0") {
        return Err("invalid PE signature".into());
    }
    Ok((
        u16::from_le_bytes(
            image
                .get(4..6)
                .ok_or("PE machine")?
                .try_into()
                .map_err(|_| "machine")?,
        ),
        u16::from_le_bytes(
            image
                .get(0x5c..0x5e)
                .ok_or("PE subsystem")?
                .try_into()
                .map_err(|_| "subsystem")?,
        ),
    ))
}

fn audit(tools: &mut Tools, files: &[PathBuf], arch: &str) -> Result<()> {
    let expected = match arch {
        "x86_64" => 0x8664,
        "aarch64" => 0xaa64,
        _ => return Err("unsupported Windows architecture".into()),
    };
    let names: BTreeSet<_> = files
        .iter()
        .filter_map(|p| p.file_name())
        .map(|name| name.to_string_lossy().to_lowercase())
        .collect();
    let system =
        PathBuf::from(tools.variable("SystemRoot").ok_or("missing SystemRoot")?).join("System32");
    for path in files {
        let (machine, subsystem) = pe(path)?;
        if machine != expected {
            return Err("wrong PE architecture".into());
        }
        if path.extension() == Some(OsStr::new("exe"))
            && subsystem
                != if path.file_stem() == Some(OsStr::new("vibecut")) {
                    2
                } else {
                    3
                }
        {
            return Err("wrong executable subsystem".into());
        }
        let output = run!(tools, "dumpbin", "/dependents", path)?;
        let dependencies: Vec<_> = output
            .lines()
            .map(str::trim)
            .filter(|line| {
                line.to_lowercase().ends_with(".dll") && !line.contains(char::is_whitespace)
            })
            .collect();
        if dependencies.is_empty() {
            return Err("no PE dependencies reported".into());
        }
        for dep in dependencies {
            let name = dep.to_lowercase();
            if !names.contains(&name)
                && !name.starts_with("api-ms-")
                && !name.starts_with("ext-ms-")
            {
                let ffmpeg = [
                    "avcodec-",
                    "avformat-",
                    "avutil-",
                    "avfilter-",
                    "swscale-",
                    "swresample-",
                ]
                .iter()
                .any(|prefix| name.starts_with(prefix));
                if ffmpeg || !system.join(dep).is_file() {
                    return Err(format!("unbundled dependency {dep}"));
                }
            }
        }
    }
    Ok(())
}

pub(super) fn package(
    tools: &mut Tools,
    r: &Request,
    prefix: &Path,
    output: &Path,
    receipt: &mut Value,
) -> Result<()> {
    let payload = output.join("VibeCut");
    fs::create_dir(&payload).map_err(|e| e.to_string())?;
    for name in ["vibecut.exe", "vibecut-cli.exe"] {
        copy(&r.binaries.join(name), &payload.join(name))?;
    }
    for path in package::files_with(&prefix.join("bin"), "dll")? {
        copy(&path, &payload.join(path.file_name().ok_or("DLL name")?))?;
    }
    let files = entries(&payload)?;
    if files.len() < 8 {
        return Err("incomplete FFmpeg DLL set".into());
    }
    audit(tools, &files, &r.arch)?;
    package::compliance(tools, prefix, &payload.join("compliance"))?;
    for name in ["install.ps1", "uninstall.ps1"] {
        copy(
            &tools.root.join("packaging/windows").join(name),
            &payload.join(name),
        )?;
    }
    if let Some(certificate) = tools.env.get("VIBECUT_WINDOWS_CERT_SHA1").cloned() {
        let timestamp = tools
            .env
            .get("VIBECUT_WINDOWS_TIMESTAMP_URL")
            .cloned()
            .ok_or("Windows signing timestamp URL missing")?;
        for path in package::files_with(&payload, "dll")?
            .into_iter()
            .chain([payload.join("vibecut-cli.exe"), payload.join("vibecut.exe")])
        {
            run!(
                tools,
                "signtool",
                "sign",
                "/sha1",
                &certificate,
                "/fd",
                "SHA256",
                "/tr",
                &timestamp,
                "/td",
                "SHA256",
                &path
            )?;
            run!(tools, "signtool", "verify", "/pa", "/all", &path)?;
        }
        for name in ["uninstall.ps1", "install.ps1"] {
            run!(
                tools,
                "powershell",
                "-NoProfile",
                "-File",
                tools.root.join("packaging/windows/sign-script.ps1"),
                payload.join(name),
                &certificate,
                &timestamp
            )?;
        }
    }
    write_json(&payload.join("install-manifest.json"), &manifest(&payload)?)?;
    let zip = output.join(format!(
        "vibecut-{}-windows-{}.zip",
        tools.version(),
        r.arch
    ));
    let script = "$ErrorActionPreference='Stop'; Compress-Archive -LiteralPath $env:VIBECUT_ZIP_SOURCE -DestinationPath $env:VIBECUT_ZIP_TARGET";
    let mut command = Command::new("powershell");
    command
        .args(["-NoProfile", "-Command", script])
        .env("VIBECUT_ZIP_SOURCE", &payload)
        .env("VIBECUT_ZIP_TARGET", &zip);
    if tools.execute(&mut command, Duration::from_secs(600))?.code != Some(0) {
        return Err("Windows archive creation failed".into());
    }
    insert(receipt, "package_sha256", json!(digest(&zip)?))?;
    insert(receipt, "files", manifest(&payload)?)?;
    Ok(())
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic
)]
mod tests {
    use super::{audit, pe};
    use crate::distribution::{Temp, Tools};
    use std::env;
    use std::fs;

    #[test]
    fn pe_headers_are_bounded_and_preserve_architecture_and_subsystem() {
        let temp = Temp::new(&env::temp_dir()).expect("temp");
        let path = temp.0.join("vibecut.exe");
        let mut data = vec![0_u8; 256];
        data[..2].copy_from_slice(b"MZ");
        data[0x3c..0x40].copy_from_slice(&64_u32.to_le_bytes());
        data[64..68].copy_from_slice(b"PE\0\0");
        data[68..70].copy_from_slice(&0xaa64_u16.to_le_bytes());
        data[64 + 0x5c..64 + 0x5e].copy_from_slice(&2_u16.to_le_bytes());
        fs::write(&path, &data).expect("PE");
        assert_eq!(pe(&path).expect("header"), (0xaa64, 2));
        let mut tools = Tools::new(&temp.0);
        tools
            .env
            .insert("SystemRoot".into(), temp.0.to_string_lossy().into_owned());
        assert!(
            audit(&mut tools, std::slice::from_ref(&path), "x86_64")
                .expect_err("wrong arch")
                .contains("architecture")
        );
        for data in [b"not PE".to_vec(), b"MZ".to_vec(), vec![0; 65]] {
            fs::write(&path, data).expect("hostile");
            assert!(pe(&path).is_err());
        }
    }
}

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

/// A PE image's headers, enough to map an RVA to a file offset.
struct Image {
    machine: u16,
    subsystem: u16,
    import_rva: u32,
    sections: Vec<Section>,
}

struct Section {
    virtual_address: u32,
    virtual_size: u32,
    raw_size: u32,
    raw_offset: u32,
}

fn image(path: &Path) -> Result<Image> {
    let mut file = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut dos = [0_u8; 64];
    file.read_exact(&mut dos).map_err(|e| e.to_string())?;
    if dos.get(..2) != Some(b"MZ") {
        return Err("not a PE image".into());
    }
    let offset = u32::from_le_bytes(
        dos.get(0x3c..0x40)
            .ok_or("PE offset")?
            .try_into()
            .map_err(|_| "PE offset")?,
    );
    file.seek(SeekFrom::Start(u64::from(offset)))
        .map_err(|e| e.to_string())?;
    let mut coff = [0_u8; 24];
    file.read_exact(&mut coff).map_err(|e| e.to_string())?;
    if coff.get(..4) != Some(b"PE\0\0") {
        return Err("invalid PE signature".into());
    }
    let machine = u16::from_le_bytes(
        coff.get(4..6)
            .ok_or("PE machine")?
            .try_into()
            .map_err(|_| "machine")?,
    );
    let count = u16::from_le_bytes(
        coff.get(6..8)
            .ok_or("PE sections")?
            .try_into()
            .map_err(|_| "sections")?,
    );
    if count == 0 || count > 96 {
        return Err("PE section count out of range".into());
    }
    let optional_size = usize::from(u16::from_le_bytes(
        coff.get(20..22)
            .ok_or("PE optional size")?
            .try_into()
            .map_err(|_| "optional size")?,
    ));
    let mut optional = vec![0_u8; optional_size];
    file.read_exact(&mut optional).map_err(|e| e.to_string())?;
    let magic = u16::from_le_bytes(
        optional
            .get(..2)
            .ok_or("PE optional magic")?
            .try_into()
            .map_err(|_| "optional magic")?,
    );
    // Data directories start after the fixed fields of each optional header.
    let directories = match magic {
        0x10b => 96,
        0x20b => 112,
        _ => return Err("unknown PE optional header".into()),
    };
    let subsystem = u16::from_le_bytes(
        optional
            .get(68..70)
            .ok_or("PE subsystem")?
            .try_into()
            .map_err(|_| "subsystem")?,
    );
    let import_rva = u32::from_le_bytes(
        optional
            .get(directories + 8..directories + 12)
            .ok_or("PE import directory")?
            .try_into()
            .map_err(|_| "import directory")?,
    );
    let mut table = vec![0_u8; usize::from(count) * 40];
    file.read_exact(&mut table).map_err(|e| e.to_string())?;
    let mut sections = Vec::with_capacity(usize::from(count));
    for index in 0..usize::from(count) {
        let base = index * 40;
        let field = |range: std::ops::Range<usize>| -> Result<u32> {
            Ok(u32::from_le_bytes(
                table
                    .get(range)
                    .ok_or("PE section")?
                    .try_into()
                    .map_err(|_| "PE section")?,
            ))
        };
        sections.push(Section {
            virtual_size: field(base + 8..base + 12)?,
            virtual_address: field(base + 12..base + 16)?,
            raw_size: field(base + 16..base + 20)?,
            raw_offset: field(base + 20..base + 24)?,
        });
    }
    Ok(Image {
        machine,
        subsystem,
        import_rva,
        sections,
    })
}

/// Maps an RVA to a file offset through the section that owns it.
fn offset(sections: &[Section], rva: u32) -> Result<u64> {
    for section in sections {
        let size = section.virtual_size.max(section.raw_size);
        if let Some(relative) = rva.checked_sub(section.virtual_address)
            && relative < size
        {
            return Ok(u64::from(section.raw_offset) + u64::from(relative));
        }
    }
    Err("PE RVA outside every section".into())
}

fn name(file: &mut fs::File, at: u64) -> Result<String> {
    file.seek(SeekFrom::Start(at)).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    let mut byte = [0_u8; 1];
    loop {
        if bytes.len() >= 260 {
            return Err("PE import name is too long".into());
        }
        file.read_exact(&mut byte).map_err(|e| e.to_string())?;
        if byte.first() == Some(&0) {
            break;
        }
        bytes.push(byte.first().copied().ok_or("PE import name byte")?);
    }
    if bytes.is_empty() {
        return Err("empty PE import name".into());
    }
    String::from_utf8(bytes).map_err(|_| "PE import name is not UTF-8".into())
}

/// Every DLL named by the import directory, without an external dump tool.
fn imports(path: &Path) -> Result<Vec<String>> {
    let image = image(path)?;
    if image.import_rva == 0 {
        return Err("no PE import directory".into());
    }
    let mut file = fs::File::open(path).map_err(|e| e.to_string())?;
    let base = offset(&image.sections, image.import_rva)?;
    let mut names = Vec::new();
    for index in 0..4096_u64 {
        let at = base + index * 20;
        file.seek(SeekFrom::Start(at)).map_err(|e| e.to_string())?;
        let mut entry = [0_u8; 20];
        file.read_exact(&mut entry).map_err(|e| e.to_string())?;
        if entry.iter().all(|byte| *byte == 0) {
            return Ok(names);
        }
        let name_rva = u32::from_le_bytes(
            entry
                .get(12..16)
                .ok_or("PE import descriptor")?
                .try_into()
                .map_err(|_| "import descriptor")?,
        );
        if name_rva == 0 {
            return Err("PE import descriptor has no name".into());
        }
        names.push(name(&mut file, offset(&image.sections, name_rva)?)?);
    }
    Err("PE import table has no terminator".into())
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
        let header = image(path)?;
        if header.machine != expected {
            return Err("wrong PE architecture".into());
        }
        if path.extension() == Some(OsStr::new("exe"))
            && header.subsystem
                != if path.file_stem() == Some(OsStr::new("vibecut")) {
                    2
                } else {
                    3
                }
        {
            return Err("wrong executable subsystem".into());
        }
        let dependencies = imports(path)?;
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
                if ffmpeg || !system.join(&dep).is_file() {
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
    use super::{audit, image, imports};
    use crate::distribution::{Temp, Tools};
    use std::env;
    use std::fs;

    /// A PE32+ image with one section; `linked` adds one import descriptor.
    fn synthetic(linked: bool) -> Vec<u8> {
        let mut data = vec![0_u8; 1024];
        data[..2].copy_from_slice(b"MZ");
        data[0x3c..0x40].copy_from_slice(&64_u32.to_le_bytes());
        data[64..68].copy_from_slice(b"PE\0\0");
        data[68..70].copy_from_slice(&0xaa64_u16.to_le_bytes());
        data[70..72].copy_from_slice(&1_u16.to_le_bytes());
        data[84..86].copy_from_slice(&240_u16.to_le_bytes());
        let optional = 88;
        data[optional..optional + 2].copy_from_slice(&0x20b_u16.to_le_bytes());
        data[optional + 68..optional + 70].copy_from_slice(&2_u16.to_le_bytes());
        let table = optional + 240;
        data[table + 8..table + 12].copy_from_slice(&0x200_u32.to_le_bytes());
        data[table + 12..table + 16].copy_from_slice(&0x1000_u32.to_le_bytes());
        data[table + 16..table + 20].copy_from_slice(&0x200_u32.to_le_bytes());
        data[table + 20..table + 24].copy_from_slice(&512_u32.to_le_bytes());
        if linked {
            data[optional + 120..optional + 124].copy_from_slice(&0x1000_u32.to_le_bytes());
            data[524..528].copy_from_slice(&0x1040_u32.to_le_bytes());
            data[576..588].copy_from_slice(b"KERNEL32.dll");
        }
        data
    }

    #[test]
    fn pe_headers_are_bounded_and_preserve_architecture_and_subsystem() {
        let temp = Temp::new(&env::temp_dir()).expect("temp");
        let path = temp.0.join("vibecut.exe");
        fs::write(&path, synthetic(false)).expect("PE");
        let header = image(&path).expect("header");
        assert_eq!((header.machine, header.subsystem), (0xaa64, 2));
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
            assert!(image(&path).is_err());
        }
    }

    #[test]
    fn imports_name_every_linked_library_and_refuse_hostile_tables() {
        let temp = Temp::new(&env::temp_dir()).expect("temp");
        let path = temp.0.join("vibecut.exe");
        fs::write(&path, synthetic(true)).expect("PE");
        assert_eq!(imports(&path).expect("imports"), ["KERNEL32.dll"]);
        fs::write(&path, synthetic(false)).expect("PE");
        assert!(
            imports(&path)
                .expect_err("no import directory")
                .contains("import directory")
        );
        fs::write(&path, &synthetic(true)[..100]).expect("truncated");
        assert!(imports(&path).is_err());
        assert!(image(&path).is_err());
    }
}

//! Linux packaging: tarball tree, bundled FFmpeg libraries and runpath audit.
use super::package::{self, Request};
use super::{
    Result, Tools, copy, digest, entries, executable, insert, manifest, symlink, write_json,
};
use serde_json::{Value, json};
use std::ffi::OsStr;
use std::fs;
use std::path::Path;

pub(super) fn package(
    tools: &mut Tools,
    r: &Request,
    prefix: &Path,
    output: &Path,
    receipt: &mut Value,
) -> Result<()> {
    let name = format!("vibecut-{}-linux-{}", tools.version(), r.arch);
    let payload = output.join(&name);
    let bin = payload.join("bin");
    fs::create_dir_all(&bin).map_err(|e| e.to_string())?;
    for binary in ["vibecut", "vibecut-cli"] {
        let destination = bin.join(binary);
        copy(&r.binaries.join(binary), &destination)?;
        executable(&destination)?;
        // The package keeps FFmpeg beside the binaries; refuse an unrelocated build.
        let dynamic = run!(tools, "readelf", "-d", &destination)?;
        if !dynamic.contains("$ORIGIN/../lib") {
            return Err(format!("{binary} is missing the $ORIGIN/../lib runpath"));
        }
    }
    let lib = payload.join("lib");
    fs::create_dir_all(&lib).map_err(|e| e.to_string())?;
    let mut codec = false;
    for path in entries(&prefix.join("lib"))? {
        let Some(name) = path.file_name().and_then(OsStr::to_str) else {
            continue;
        };
        if !name.contains(".so") {
            continue;
        }
        let destination = lib.join(name);
        if path.is_symlink() {
            symlink(
                &fs::read_link(&path).map_err(|e| e.to_string())?,
                &destination,
            )?;
        } else {
            copy(&path, &destination)?;
        }
        codec |= name.starts_with("libavcodec.so.");
    }
    if !codec {
        return Err("incomplete FFmpeg shared library set".into());
    }
    package::compliance(tools, prefix, &payload.join("compliance"))?;
    for (source, name) in [
        ("packaging/linux/install.sh", "install.sh"),
        ("packaging/linux/vibecut.desktop", "vibecut.desktop"),
    ] {
        copy(&tools.root.join(source), &payload.join(name))?;
    }
    copy(
        &tools.root.join("assets/app-icon/vibecut-1024.png"),
        &payload.join("vibecut.png"),
    )?;
    executable(&payload.join("install.sh"))?;
    write_json(&payload.join("install-manifest.json"), &manifest(&payload)?)?;
    let archive = output.join(format!("{name}.tar.gz"));
    run!(tools, "tar", "-czf", &archive, "-C", output, &name)?;
    insert(receipt, "package_sha256", json!(digest(&archive)?))?;
    insert(receipt, "files", manifest(&payload)?)?;
    Ok(())
}

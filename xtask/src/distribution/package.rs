//! Common package tree assembly, compliance files and per-platform dispatch.
use super::{Host, Result, Tools, canonical, copy, digest, entries, fresh, insert, manifest};
use super::{linux, macos, signing, text, tree, windows, write_json};
use serde_json::json;
use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

pub(super) struct Request {
    pub os: String,
    pub arch: String,
    pub prefix: PathBuf,
    pub binaries: PathBuf,
    pub output: PathBuf,
}
impl Request {
    pub fn options(o: &super::Options) -> Result<Self> {
        Ok(Self {
            os: o.get("--os")?.into(),
            arch: o.get("--arch")?.into(),
            prefix: o.path("--ffmpeg")?,
            binaries: o.path("--binaries")?,
            output: o.path("--output")?,
        })
    }
}
pub(super) fn files_with(root: &Path, extension: &str) -> Result<Vec<PathBuf>> {
    Ok(entries(root)?
        .into_iter()
        .filter(|p| p.extension() == Some(OsStr::new(extension)) && !p.is_symlink())
        .collect())
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Source {
    pub name: String,
    pub version: String,
    pub hash: String,
}
pub(super) fn sources(receipt: &str) -> Result<Vec<Source>> {
    let lines: Vec<_> = receipt.lines().collect();
    let mut result = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let Some((name, version)) = line.split_once(": ") else {
            continue;
        };
        if !matches!(name, "ffmpeg" | "opus" | "libvpx" | "zlib") {
            continue;
        }
        if version.is_empty()
            || !version
                .bytes()
                .all(|byte| byte.is_ascii_digit() || byte == b'.')
        {
            return Err("invalid source version".into());
        }
        if !lines
            .get(i + 1)
            .is_some_and(|line| line.starts_with("source: "))
        {
            return Err("source URL missing".into());
        }
        let hash = lines
            .get(i + 2)
            .and_then(|line| line.strip_prefix("sha256: "))
            .ok_or("source hash missing")?;
        if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("invalid source hash".into());
        }
        result.push(Source {
            name: name.into(),
            version: version.into(),
            hash: hash.into(),
        });
    }
    let names: BTreeSet<_> = result.iter().map(|s| s.name.as_str()).collect();
    if result.len() != 4 || names != BTreeSet::from(["ffmpeg", "opus", "libvpx", "zlib"]) {
        return Err("missing/duplicate corresponding source identities".into());
    }
    Ok(result)
}
pub(super) fn compliance(tools: &Tools, prefix: &Path, destination: &Path) -> Result<()> {
    let receipt = text(&prefix.join("BUILD.txt"))?;
    let gpl = receipt.contains("--enable-gpl");
    for flag in ["--disable-nonfree", "--disable-autodetect"] {
        if !receipt.contains(flag) {
            return Err(format!("FFmpeg receipt missing {flag}"));
        }
    }
    if !gpl && !receipt.contains("--disable-gpl") {
        return Err("FFmpeg receipt names no licence build".into());
    }
    let licenses = prefix.join("compliance");
    let names = entries(&licenses)?;
    let mut admitted_names = vec!["ffmpeg-lgpl", "ffmpeg-license"];
    if gpl {
        admitted_names.push("x264");
    }
    admitted_names.extend(["opus", "libvpx", "zlib"]);
    for admitted in admitted_names {
        let matching: Vec<_> = names
            .iter()
            .filter(|p| {
                let name = p.file_name().unwrap_or_default().to_string_lossy();
                match admitted {
                    "ffmpeg-lgpl" => name.ends_with("COPYING.LGPLv2.1"),
                    "ffmpeg-license" => name.ends_with("LICENSE.md"),
                    "x264" => name.starts_with("x264-") && name.ends_with("-COPYING"),
                    "opus" => name.starts_with("opus-") && name.ends_with("-COPYING"),
                    "zlib" => name.starts_with("zlib-") && name.ends_with("-LICENSE"),
                    _ => name.starts_with("libvpx-") && name.ends_with("-LICENSE"),
                }
            })
            .collect();
        if matching.is_empty()
            || matching
                .iter()
                .any(|p| fs::metadata(p).map(|m| m.len() == 0).unwrap_or(true))
        {
            return Err(format!("missing license text: {admitted}"));
        }
    }
    for line in receipt
        .lines()
        .filter_map(|line| line.strip_prefix("backport: "))
    {
        let (name, hash) = line
            .split_once(" sha256: ")
            .ok_or("invalid backport receipt")?;
        if digest(
            &licenses
                .join("patches")
                .join(Path::new(name).file_name().ok_or("patch filename")?),
        )? != hash
        {
            return Err("FFmpeg backport checksum mismatch".into());
        }
    }
    let identities = sources(&receipt)?;
    for identity in &identities {
        let archive = prefix
            .parent()
            .ok_or("source parent")?
            .join("sources")
            .join(format!("{}-{}.archive", identity.name, identity.version));
        if !fs::symlink_metadata(&archive).is_ok_and(|metadata| metadata.file_type().is_file()) {
            return Err("corresponding source archive must be a regular file".into());
        }
        if digest(&archive)? != identity.hash {
            return Err("corresponding source checksum mismatch".into());
        }
    }
    tree(&licenses, destination)?;
    copy(&prefix.join("BUILD.txt"), &destination.join("BUILD.txt"))?;
    // The recipe is the whole `ffmpeg` module (interface beside its folder), not
    // one file: the build steps live in the folder.
    let recipe = destination.join("build-recipe");
    copy(
        &tools.root.join("xtask/src/ffmpeg.rs"),
        &recipe.join("ffmpeg.rs"),
    )?;
    tree(&tools.root.join("xtask/src/ffmpeg"), &recipe.join("ffmpeg"))?;
    for identity in identities {
        let name = format!("{}-{}.archive", identity.name, identity.version);
        copy(
            &prefix
                .parent()
                .ok_or("source parent")?
                .join("sources")
                .join(&name),
            &destination.join("sources").join(name),
        )?;
    }
    copy(
        &tools.root.join("LICENSE"),
        &destination.join("VibeCut-LICENSE.txt"),
    )?;
    fn assets(root: &Path, path: &Path, destination: &Path) -> Result<()> {
        for path in entries(path)? {
            let kind = fs::symlink_metadata(&path)
                .map_err(|e| e.to_string())?
                .file_type();
            if kind.is_symlink() {
                return Err("asset notices must not follow symlinks".into());
            }
            if kind.is_dir() {
                assets(root, &path, destination)?;
            } else {
                let name = path.file_name().ok_or("asset name")?.to_string_lossy();
                if name.starts_with("OFL")
                    || name.starts_with("LICENSE")
                    || name.starts_with("NOTICE")
                    || path.extension() == Some(OsStr::new("attribution"))
                {
                    copy(
                        &path,
                        &destination
                            .join("assets")
                            .join(path.strip_prefix(root).map_err(|e| e.to_string())?),
                    )?;
                }
            }
        }
        Ok(())
    }
    assets(
        &tools.root.join("assets"),
        &tools.root.join("assets"),
        destination,
    )?;
    let licence = if gpl {
        "GPL-3.0 FFmpeg with libx264, Opus, libvpx and zlib"
    } else {
        "LGPL-2.1-or-later FFmpeg with Opus, libvpx and zlib"
    };
    fs::write(destination.join("NOTICE.txt"),format!("VibeCut dynamically links {licence}. Corresponding source archives, verified build receipts, backports, recipe and license texts accompany this distribution. Reverse engineering to debug modified libraries is permitted. See README.md for rebuilding and replacement. No model weights are included.\n")).map_err(|e|e.to_string())?;
    copy(
        &tools.root.join("packaging/README.md"),
        &destination.join("README.md"),
    )
}
pub(super) fn package(
    tools: &mut Tools,
    request: &Request,
    config: &signing::Config,
) -> Result<()> {
    if !matches!(
        (tools.host, request.os.as_str()),
        (Host::Mac, "macos") | (Host::Windows, "windows") | (Host::Linux, "linux")
    ) {
        return Err("packaging must run on the target OS".into());
    }
    if !["aarch64", "x86_64", "universal"].contains(&request.arch.as_str())
        || (request.os != "macos" && request.arch == "universal")
    {
        return Err("unsupported distribution architecture".into());
    }
    let output = fresh(&request.output)?;
    let prefix = canonical(&request.prefix)?;
    let revision = run!(tools, "git", "rev-parse", "HEAD")?;
    let mut receipt = json!({"version":tools.version(),"os":request.os,"arch":request.arch,"revision":revision.trim(),"release_acceptance":false,"input_sha256":manifest(&request.binaries)?});
    if request.binaries.join("build.json").is_file() {
        insert(
            &mut receipt,
            "build",
            serde_json::from_str(&text(&request.binaries.join("build.json"))?)
                .map_err(|e| e.to_string())?,
        )?;
    }
    let result = (|| {
        match tools.host {
            Host::Mac => macos::package(tools, request, config, &prefix, &output, &mut receipt)?,
            Host::Windows => windows::package(tools, request, &prefix, &output, &mut receipt)?,
            _ => linux::package(tools, request, &prefix, &output, &mut receipt)?,
        }
        insert(
            &mut receipt,
            "status",
            json!("staged; release checks remain mandatory"),
        )?;
        Ok(())
    })();
    if result.is_err() {
        insert(&mut receipt, "status", json!("failed; do not distribute"))?;
    }
    insert(&mut receipt, "commands", json!(tools.commands))?;
    let written = write_json(&output.join("receipt.json"), &receipt);
    result.and(written)
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic
)]
mod tests {
    use super::{compliance, text};
    use crate::distribution::{Captured, Temp, Tools, arg, digest, entries};
    use std::collections::BTreeSet;
    use std::env;
    use std::fs;
    use std::path::{Path, PathBuf};

    fn sdk(root: &Path) -> PathBuf {
        let prefix = root.join("prefix");
        fs::create_dir_all(prefix.join("compliance")).expect("licenses");
        fs::create_dir(root.join("sources")).expect("sources");
        for name in [
            "ffmpeg-COPYING.LGPLv2.1",
            "ffmpeg-LICENSE.md",
            "opus-1.0-COPYING",
            "libvpx-1.0-LICENSE",
            "zlib-1.0-LICENSE",
        ] {
            fs::write(prefix.join("compliance").join(name), b"license fixture").expect("license");
        }
        let mut receipt = "--disable-gpl --disable-nonfree --disable-autodetect\n".to_owned();
        for name in ["ffmpeg", "opus", "libvpx", "zlib"] {
            let path = root.join("sources").join(format!("{name}-1.0.archive"));
            fs::write(&path, b"synthetic corresponding source").expect("archive");
            receipt.push_str(&format!(
                "{name}: 1.0\nsource: https://example.invalid/source\nsha256: {}\n",
                digest(&path).expect("hash")
            ));
        }
        fs::write(prefix.join("BUILD.txt"), receipt).expect("receipt");
        prefix
    }
    fn workspace() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("workspace")
            .to_owned()
    }
    #[test]
    fn source_archives_are_verified_and_transfer_excludes_build_caches() {
        let temp = Temp::new(&env::temp_dir()).expect("temp");
        let prefix = sdk(&temp.0);
        let mut tools = Tools::new(&workspace());
        fs::create_dir(prefix.join("lib")).expect("lib");
        fs::create_dir(temp.0.join("binaries")).expect("bins");
        for name in ["vibecut", "vibecut-cli"] {
            fs::write(temp.0.join("binaries").join(name), b"synthetic").expect("binary");
        }
        fs::create_dir(temp.0.join("sources/stale-build-cache")).expect("stale");
        fs::write(
            temp.0.join("sources/stale-build-cache/object.o"),
            b"not distributable",
        )
        .expect("stale object");
        tools.runner = Box::new(|_, _| {
            Ok(Captured {
                code: Some(0),
                stdout: String::new(),
                stderr: String::new(),
            })
        });
        crate::distribution::universal::export(
            &mut tools,
            &prefix,
            &temp.0.join("binaries"),
            &temp.0.join("export"),
        )
        .expect("export");
        let names: BTreeSet<_> = entries(&temp.0.join("export/sdk/sources"))
            .expect("sources")
            .into_iter()
            .map(|p| p.file_name().expect("name").to_owned())
            .collect();
        assert_eq!(
            names,
            BTreeSet::from([
                arg("ffmpeg-1.0.archive"),
                arg("opus-1.0.archive"),
                arg("libvpx-1.0.archive"),
                arg("zlib-1.0.archive")
            ])
        );
        fs::write(temp.0.join("sources/opus-1.0.archive"), b"tampered").expect("tamper");
        assert!(
            compliance(&tools, &prefix, &temp.0.join("rejected"))
                .expect_err("must refuse")
                .contains("checksum mismatch")
        );
    }
    #[test]
    fn missing_license_and_modified_backport_are_refused() {
        let temp = Temp::new(&env::temp_dir()).expect("temp");
        let prefix = sdk(&temp.0);
        let tools = Tools::new(&workspace());
        fs::create_dir(prefix.join("compliance/patches")).expect("patches");
        fs::write(
            prefix.join("compliance/patches/fix.patch"),
            b"changed patch",
        )
        .expect("patch");
        let receipt = text(&prefix.join("BUILD.txt")).expect("receipt")
            + &format!("backport: fix.patch sha256: {}\n", "0".repeat(64));
        fs::write(prefix.join("BUILD.txt"), receipt).expect("receipt");
        assert!(
            compliance(&tools, &prefix, &temp.0.join("bad-patch"))
                .expect_err("must refuse")
                .contains("backport checksum")
        );
        fs::remove_file(prefix.join("compliance/opus-1.0-COPYING")).expect("remove");
        assert!(
            compliance(&tools, &prefix, &temp.0.join("bad-license"))
                .expect_err("must refuse")
                .contains("license")
        );
    }
    #[test]
    fn distribution_carries_the_whole_ffmpeg_build_recipe() {
        let temp = Temp::new(&env::temp_dir()).expect("temp");
        let prefix = sdk(&temp.0);
        let tools = Tools::new(&workspace());
        let out = temp.0.join("valid");
        compliance(&tools, &prefix, &out).expect("complete distribution");
        for (file, needle) in [
            ("ffmpeg.rs", "pub fn provision"),
            ("ffmpeg/recipes.rs", "--disable-autodetect"),
            ("ffmpeg/fetch.rs", "ZLIB_REPOSITORY"),
        ] {
            let shipped = text(&out.join("build-recipe").join(file)).expect(file);
            assert!(shipped.contains(needle), "{file} lacks {needle}");
        }
    }

    #[test]
    fn zlib_license_and_corresponding_source_are_required() {
        for (removed, expected) in [
            ("prefix/compliance/zlib-1.0-LICENSE", "zlib"),
            ("sources/zlib-1.0.archive", "source archive"),
        ] {
            let temp = Temp::new(&env::temp_dir()).expect("temp");
            let prefix = sdk(&temp.0);
            let tools = Tools::new(&workspace());
            let valid = temp.0.join("valid");
            compliance(&tools, &prefix, &valid).expect("complete zlib distribution");
            assert!(
                text(&valid.join("NOTICE.txt"))
                    .expect("notice")
                    .contains("zlib")
            );
            fs::remove_file(temp.0.join(removed)).expect("remove zlib artifact");
            let error = compliance(&tools, &prefix, &temp.0.join("rejected"))
                .expect_err("missing zlib artifact must be refused");
            assert!(error.contains(expected), "{removed}: {error}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn manifest_records_symlink_without_reading_external_target() {
        use crate::distribution::{manifest, symlink};
        use serde_json::json;
        let temp = Temp::new(&env::temp_dir()).expect("temp");
        symlink(Path::new("/missing/app/cli"), &temp.0.join("cli")).expect("link");
        assert_eq!(
            manifest(&temp.0).expect("manifest"),
            json!({"cli":{"symlink":"/missing/app/cli"}})
        );
    }
}

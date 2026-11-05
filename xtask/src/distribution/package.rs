#[allow(clippy::wildcard_imports)]
use super::*;
use std::collections::BTreeSet;

pub(super) struct Request {
    pub os: String,
    pub arch: String,
    pub prefix: PathBuf,
    pub binaries: PathBuf,
    pub output: PathBuf,
}
impl Request {
    pub fn options(o: &Options) -> Result<Self> {
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
        if !matches!(name, "ffmpeg" | "opus" | "libvpx") {
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
    if result.len() != 3 || names != BTreeSet::from(["ffmpeg", "opus", "libvpx"]) {
        return Err("missing/duplicate corresponding source identities".into());
    }
    Ok(result)
}
pub(super) fn compliance(tools: &Tools, prefix: &Path, destination: &Path) -> Result<()> {
    let receipt = text(&prefix.join("BUILD.txt"))?;
    for flag in ["--disable-gpl", "--disable-nonfree", "--disable-autodetect"] {
        if !receipt.contains(flag) {
            return Err(format!("FFmpeg receipt missing {flag}"));
        }
    }
    let licenses = prefix.join("compliance");
    let names = entries(&licenses)?;
    for admitted in ["ffmpeg-lgpl", "ffmpeg-license", "opus", "libvpx"] {
        let matching: Vec<_> = names
            .iter()
            .filter(|p| {
                let name = p.file_name().unwrap_or_default().to_string_lossy();
                match admitted {
                    "ffmpeg-lgpl" => name.ends_with("COPYING.LGPLv2.1"),
                    "ffmpeg-license" => name.ends_with("LICENSE.md"),
                    "opus" => name.starts_with("opus-") && name.ends_with("-COPYING"),
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
        if !fs::symlink_metadata(&archive)
            .map_err(|e| e.to_string())?
            .file_type()
            .is_file()
        {
            return Err("corresponding source archive must be a regular file".into());
        }
        if digest(&archive)? != identity.hash {
            return Err("corresponding source checksum mismatch".into());
        }
    }
    tree(&licenses, destination)?;
    copy(&prefix.join("BUILD.txt"), &destination.join("BUILD.txt"))?;
    copy(
        &tools.root.join("xtask/src/ffmpeg.rs"),
        &destination.join("build-recipe.rs"),
    )?;
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
        &destination.join("VibeCut-MIT.txt"),
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
    fs::write(destination.join("NOTICE.txt"),"VibeCut dynamically links LGPL-2.1-or-later FFmpeg with Opus and libvpx. Corresponding source archives, verified build receipts, backports, recipe and license texts accompany this distribution. Reverse engineering to debug modified libraries is permitted. See README.md for rebuilding and replacement. No model weights are included.\n").map_err(|e|e.to_string())?;
    copy(
        &tools.root.join("packaging/README.md"),
        &destination.join("README.md"),
    )
}
fn pe(path: &Path) -> Result<(u16, u16)> {
    use std::io::{Seek, SeekFrom};
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

fn windows_audit(tools: &mut Tools, files: &[PathBuf], arch: &str) -> Result<()> {
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
    request: &Request,
    config: &signing::Config,
) -> Result<()> {
    if !matches!(
        (tools.host, request.os.as_str()),
        (Host::Mac, "macos") | (Host::Windows, "windows")
    ) {
        return Err("packaging must run on the target OS".into());
    }
    if !["aarch64", "x86_64", "universal"].contains(&request.arch.as_str())
        || (request.os == "windows" && request.arch == "universal")
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
        if tools.host == Host::Mac {
            mac_package(tools, request, config, &prefix, &output, &mut receipt)?;
        } else {
            windows_package(tools, request, &prefix, &output, &mut receipt)?;
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
fn mac_package(
    tools: &mut Tools,
    r: &Request,
    config: &signing::Config,
    prefix: &Path,
    output: &Path,
    receipt: &mut Value,
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
        let deps = macos::dependencies(tools, &path)?;
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
        macos::audit(tools, path, &r.arch)?;
    }
    macos::relocate(tools, &files, &libs)?;
    copy(
        &tools.root.join("assets/app-icon/vibecut.icns"),
        &resources.join("vibecut.icns"),
    )?;
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd"><plist version="1.0"><dict><key>CFBundleExecutable</key><string>vibecut</string><key>CFBundleIdentifier</key><string>com.canhta.vibecut</string><key>CFBundleName</key><string>VibeCut</string><key>CFBundlePackageType</key><string>APPL</string><key>CFBundleShortVersionString</key><string>{version}</string><key>CFBundleVersion</key><string>{version}</string><key>CFBundleIconFile</key><string>vibecut.icns</string><key>NSHighResolutionCapable</key><true/><key>LSMinimumSystemVersion</key><string>{minimum}</string></dict></plist>"#,
        version = tools.version(),
        minimum = macos::MINIMUM
    );
    fs::write(contents.join("Info.plist"), plist).map_err(|e| e.to_string())?;
    compliance(tools, prefix, &resources.join("compliance"))?;
    fs::create_dir_all(payload.join("usr/local/bin")).map_err(|e| e.to_string())?;
    symlink(
        Path::new("/Applications/VibeCut.app/Contents/MacOS/vibecut-cli"),
        &payload.join("usr/local/bin/vibecut-cli"),
    )?;
    for path in &files {
        macos::sign(tools, path, &config.identity)?;
    }
    macos::sign(tools, &app, &config.identity)?;
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
    let pkg = output.join(format!("VibeCut-{}-macos-{}.pkg", tools.version(), r.arch));
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
        let dmg = output.join(format!("VibeCut-{}-macos-{}.dmg", tools.version(), r.arch));
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
        macos::sign(tools, &dmg, &config.identity)?;
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
fn windows_package(
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
    for path in files_with(&prefix.join("bin"), "dll")? {
        copy(&path, &payload.join(path.file_name().ok_or("DLL name")?))?;
    }
    let files = entries(&payload)?;
    if files.len() < 8 {
        return Err("incomplete FFmpeg DLL set".into());
    }
    windows_audit(tools, &files, &r.arch)?;
    compliance(tools, prefix, &payload.join("compliance"))?;
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
        for path in files_with(&payload, "dll")?
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
        "VibeCut-{}-windows-{}.zip",
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
#[allow(clippy::wildcard_imports)]
mod tests {
    use super::*;
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
    fn sdk(root: &Path) -> PathBuf {
        let prefix = root.join("prefix");
        fs::create_dir_all(prefix.join("compliance")).expect("licenses");
        fs::create_dir(root.join("sources")).expect("sources");
        for name in [
            "ffmpeg-COPYING.LGPLv2.1",
            "ffmpeg-LICENSE.md",
            "opus-1.0-COPYING",
            "libvpx-1.0-LICENSE",
        ] {
            fs::write(prefix.join("compliance").join(name), b"license fixture").expect("license");
        }
        let mut receipt = "--disable-gpl --disable-nonfree --disable-autodetect\n".to_owned();
        for name in ["ffmpeg", "opus", "libvpx"] {
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
        universal::export(
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
                arg("libvpx-1.0.archive")
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
            windows_audit(&mut tools, std::slice::from_ref(&path), "x86_64")
                .expect_err("wrong arch")
                .contains("architecture")
        );
        for data in [b"not PE".to_vec(), b"MZ".to_vec(), vec![0; 65]] {
            fs::write(&path, data).expect("hostile");
            assert!(pe(&path).is_err());
        }
    }
    #[cfg(unix)]
    #[test]
    fn manifest_records_symlink_without_reading_external_target() {
        let temp = Temp::new(&env::temp_dir()).expect("temp");
        symlink(Path::new("/missing/app/cli"), &temp.0.join("cli")).expect("link");
        assert_eq!(
            manifest(&temp.0).expect("manifest"),
            json!({"cli":{"symlink":"/missing/app/cli"}})
        );
    }
}

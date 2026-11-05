#[allow(clippy::wildcard_imports)]
use super::*;

fn sanitized(tools: &Tools, program: &Path, cwd: &Path) -> Command {
    let mut command = Command::new(program);
    command.current_dir(cwd);
    for key in tools.env.keys().filter(|key| {
        key.starts_with("DYLD_") || key.starts_with("FFMPEG_") || key.starts_with("PKG_CONFIG_")
    }) {
        command.env_remove(key);
    }
    command.env(
        "PATH",
        if tools.host == Host::Mac {
            PathBuf::from("/usr/bin:/bin:/usr/sbin:/sbin")
        } else {
            PathBuf::from(tools.variable("SystemRoot").unwrap_or("C:\\Windows")).join("System32")
        },
    );
    command
}
fn cli(
    tools: &mut Tools,
    binary: &Path,
    cwd: &Path,
    args: Vec<OsString>,
    receipts: &mut Vec<Value>,
) -> Result<Value> {
    let mut command = sanitized(tools, binary, cwd);
    command.args(&args).arg("--json");
    let result = tools.execute(&mut command, Duration::from_secs(180))?;
    receipts.push(json!({"args":args.iter().map(|arg|arg.to_string_lossy()).collect::<Vec<_>>(),"exit":result.code,"stdout":if result.code == Some(0) { result.stdout.as_str() } else { "<failed-process output suppressed>" }}));
    if result.code != Some(0) {
        return Err(format!(
            "distribution CLI smoke failed (exit {:?}); platform diagnostics suppressed",
            result.code
        ));
    }
    serde_json::from_str(&result.stdout).map_err(|_| "CLI smoke did not produce JSON".into())
}
fn verify_signed(tools: &mut Tools, app: &Path) -> Result<()> {
    run!(tools, "codesign", "--verify", "--deep", "--strict", app)?;
    run!(
        tools,
        "spctl",
        "--assess",
        "--type",
        "execute",
        "--verbose",
        app
    )?;
    package::validate_staple(tools, app)?;
    Ok(())
}
pub(super) fn run(tools: &mut Tools, o: &Options) -> Result<()> {
    if tools.host == Host::Other {
        return Err("distribution smoke requires macOS or Windows".into());
    }
    let installed = canonical(&o.path("--installed")?)?;
    let output = fresh(&o.path("--output")?)?;
    let binaries = if tools.host == Host::Mac {
        installed.join("Contents/MacOS")
    } else {
        installed.clone()
    };
    let suffix = if tools.host == Host::Windows {
        ".exe"
    } else {
        ""
    };
    let temp = Temp::new(&env::temp_dir())?;
    let project = temp.0.join("synthetic project.vibecut");
    let binary = binaries.join(format!("vibecut-cli{suffix}"));
    let mut receipts = Vec::new();
    let result = (|| {
        if o.has("--signed") {
            verify_signed(tools, &installed)?;
        }
        cli(
            tools,
            &binary,
            &temp.0,
            vec![arg("--version")],
            &mut receipts,
        )?;
        if let Some(media) = o.values.get("--media") {
            let media = canonical(Path::new(media))?;
            cli(
                tools,
                &binary,
                &temp.0,
                vec![arg("probe"), arg(media)],
                &mut receipts,
            )?;
        }
        cli(
            tools,
            &binary,
            &temp.0,
            vec![arg("project"), arg("new"), arg(&project)],
            &mut receipts,
        )?;
        let mut data: Value = serde_json::from_str(&text(&project)?).map_err(|e| e.to_string())?;
        let canvas = data
            .pointer_mut("/project/canvas")
            .and_then(Value::as_object_mut)
            .ok_or("project canvas missing")?;
        canvas.insert("width".into(), json!(64));
        canvas.insert("height".into(), json!(64));
        write_json(&project, &data)?;
        let params = json!({"placement":{"mode":"auto"},"item":{"animations":{},"source_asset":null,"source_range":{"start":0,"end":508_032_000_000_i64},"start":0,"content":{"kind":"overlay","params":{"color":[30,160,90,255]}}}});
        cli(
            tools,
            &binary,
            &temp.0,
            vec![
                arg("exec"),
                arg(&project),
                arg("timeline.insert"),
                arg(params.to_string()),
            ],
            &mut receipts,
        )?;
        let exported = output.join("synthetic.mp4");
        cli(
            tools,
            &binary,
            &temp.0,
            vec![arg("export"), arg(&project), arg("--out"), arg(&exported)],
            &mut receipts,
        )?;
        let probe = cli(
            tools,
            &binary,
            &temp.0,
            vec![arg("probe"), arg(&exported)],
            &mut receipts,
        )?;
        write_json(&output.join("export-probe.json"), &probe)?;
        if !field(&probe, "/duration/seconds")
            .as_f64()
            .is_some_and(|duration| (duration - 2.0).abs() <= 0.1)
            || !field(&probe, "/streams").as_array().is_some_and(|streams| {
                streams
                    .iter()
                    .filter(|stream| field(stream, "/kind") == "video")
                    .count()
                    == 1
            })
            || fs::metadata(&exported).map_err(|e| e.to_string())?.len() == 0
        {
            return Err("synthetic export must contain two seconds and one video stream".into());
        }
        let app = binaries.join(format!("vibecut{suffix}"));
        if o.has("--observe") {
            observe(
                tools,
                &app,
                &installed,
                &temp.0,
                &output,
                o.has("--signed"),
                &mut receipts,
            )?;
        } else {
            let mut command = sanitized(tools, &app, &temp.0);
            command
                .arg("--headless-smoke")
                .arg(temp.0.join("workspace"));
            let result = tools.execute(&mut command, Duration::from_secs(60))?;
            receipts.push(json!({"app":"headless workspace/frontend startup","exit":result.code}));
            if result.code != Some(0) {
                return Err("distribution app headless startup failed".into());
            }
        }
        Ok(())
    })();
    let written = write_json(
        &output.join("smoke.json"),
        &json!({"commands":receipts,"release_acceptance":false}),
    );
    result.and(written)
}
fn observe(
    tools: &mut Tools,
    app: &Path,
    installed: &Path,
    cwd: &Path,
    output: &Path,
    signed: bool,
    receipts: &mut Vec<Value>,
) -> Result<()> {
    let evidence = output.join("app");
    let mut command = sanitized(tools, app, cwd);
    command.args([arg("--observe"), arg("8"), arg(&evidence)]);
    if !signed && tools.host == Host::Mac {
        command.env("DYLD_PRINT_LIBRARIES", "1");
    }
    // Only explicit --observe opens a window. The hosted/default smoke never does.
    let stdout = fs::File::create(output.join("app.stdout")).map_err(|e| e.to_string())?;
    let stderr = fs::File::create(output.join("app.stderr")).map_err(|e| e.to_string())?;
    let mut child = command
        .stdout(stdout)
        .stderr(stderr)
        .spawn()
        .map_err(|e| e.to_string())?;
    let started = Instant::now();
    let mut sampled = false;
    let result = (|| {
        loop {
            if signed
                && tools.host == Host::Mac
                && !sampled
                && started.elapsed() >= Duration::from_secs(3)
            {
                let loaded = run!(tools, "/usr/sbin/lsof", "-p", child.id().to_string(), "-Fn")?;
                fs::write(output.join("loaded-modules.txt"), &loaded).map_err(|e| e.to_string())?;
                mapped(&loaded, installed)?;
                sampled = true;
            }
            if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
                if !status.success() {
                    return Err("observed app failed".into());
                }
                break;
            }
            if started.elapsed() > Duration::from_secs(60) {
                return Err("observed app timed out".into());
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let exit: Value =
            serde_json::from_str(&text(&evidence.join("exit.json"))?).map_err(|e| e.to_string())?;
        if field(&exit, "/completed") != true || !evidence.join("window.png").is_file() {
            return Err("app observation/capture did not complete".into());
        }
        if signed && tools.host == Host::Mac && !sampled {
            return Err("app exited before runtime-library observation".into());
        }
        receipts.push(json!({"app":"native observation/capture","exit":exit}));
        Ok(())
    })();
    if result.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }
    result
}
fn mapped(output: &str, app: &Path) -> Result<()> {
    for library in [
        "libavcodec.",
        "libavformat.",
        "libavutil.",
        "libswscale.",
        "libswresample.",
    ] {
        let paths: Vec<_> = output
            .lines()
            .filter_map(|line| line.strip_prefix('n'))
            .filter(|line| line.contains(library))
            .collect();
        let root = app.join("Contents/Frameworks");
        if paths.is_empty() || paths.iter().any(|path| !Path::new(path).starts_with(&root)) {
            return Err("app loaded FFmpeg outside its bundle".into());
        }
    }
    Ok(())
}
pub(super) fn install(tools: &mut Tools, o: &Options) -> Result<()> {
    let downloaded = tools.root.join("target/download");
    let suffix = if tools.host == Host::Mac {
        if o.has("--signed") { "dmg" } else { "pkg" }
    } else {
        "zip"
    };
    let packages = package::files_with(&downloaded, suffix)?;
    if packages.len() != 1 {
        return Err("expected one downloaded distribution".into());
    }
    let file = packages.first().ok_or("download missing")?;
    if tools.host == Host::Mac && o.has("--signed") {
        run!(tools, "codesign", "--verify", "--strict", file)?;
        package::validate_staple(tools, file)?;
        run!(
            tools,
            "spctl",
            "--assess",
            "--type",
            "open",
            "--context",
            "context:primary-signature",
            file
        )?;
        let temp = Temp::new(&tools.runner_temp)?;
        let mount = temp.0.join("volume");
        fs::create_dir(&mount).map_err(|e| e.to_string())?;
        run!(
            tools,
            "hdiutil",
            "attach",
            "-nobrowse",
            "-readonly",
            "-mountpoint",
            &mount,
            file
        )?;
        let result = (|| {
            run!(
                tools,
                "sudo",
                "ditto",
                mount.join("VibeCut.app"),
                "/Applications/VibeCut.app"
            )?;
            run!(tools, "sh", mount.join("install-cli.sh"))?;
            verify_signed(tools, Path::new("/Applications/VibeCut.app"))?;
            installed_smoke(tools, "/Applications/VibeCut.app", true)
        })();
        let detached = run!(tools, "hdiutil", "detach", &mount).map(|_| ());
        result.and(detached)
    } else if tools.host == Host::Mac {
        run!(tools, "sudo", "installer", "-pkg", file, "-target", "/")?;
        installed_smoke(tools, "/Applications/VibeCut.app", false)
    } else {
        let target = tools.root.join("target/installed");
        let mut command = Command::new("powershell");
        command.args(["-NoProfile","-Command","$ErrorActionPreference='Stop'; Expand-Archive -LiteralPath $env:VIBECUT_ZIP_SOURCE -DestinationPath $env:VIBECUT_ZIP_TARGET"]).env("VIBECUT_ZIP_SOURCE",file).env("VIBECUT_ZIP_TARGET",&target);
        if tools.execute(&mut command, Duration::from_secs(120))?.code != Some(0) {
            return Err("Windows archive extraction failed".into());
        }
        let path = target.join("VibeCut");
        installed_smoke(tools, path.to_str().ok_or("install path UTF-8")?, false)
    }
}
fn installed_smoke(tools: &mut Tools, installed: &str, signed: bool) -> Result<()> {
    let mut flags = Vec::new();
    if signed {
        flags.push("--signed".into());
    }
    let values = BTreeMap::from([
        ("--installed".into(), installed.into()),
        (
            "--output".into(),
            tools
                .root
                .join("target/smoke")
                .to_str()
                .ok_or("smoke path")?
                .into(),
        ),
    ]);
    run(tools, &Options { flags, values })
}

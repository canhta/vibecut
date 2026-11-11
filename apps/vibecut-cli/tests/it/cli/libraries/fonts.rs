//! Verifies native fonts command behaviour.
use crate::cli::process::{bin, code, error_json, os, stdout_json};
use std::ffi::OsStr;
use std::process::Command;

#[test]
fn fonts_list_and_resolve_are_offline_and_download_requires_acceptance() {
    let home = tempfile::tempdir().unwrap();
    let font_cli = |args: &[&OsStr]| {
        Command::new(bin())
            .env("HOME", home.path())
            .env("LOCALAPPDATA", home.path())
            .env("XDG_CACHE_HOME", home.path())
            .args(args)
            .output()
            .unwrap()
    };
    let listed = font_cli(&[
        os("fonts"),
        os("list"),
        os("--query"),
        os("lobster"),
        os("--json"),
    ]);
    assert_eq!(
        code(&listed),
        0,
        "{}",
        String::from_utf8_lossy(&listed.stderr)
    );
    let fonts = stdout_json(&listed)["fonts"].clone();
    assert_eq!(fonts.as_array().unwrap().len(), 1);
    assert_eq!(fonts[0]["family"], "Lobster");
    assert_eq!(fonts[0]["origin"], "google");
    assert_eq!(fonts[0]["state"], "not_downloaded");
    assert_eq!(fonts[0]["license"], "OFL-1.1");
    assert!(
        fonts[0]["source"]
            .as_str()
            .unwrap()
            .starts_with("https://github.com/google/fonts/")
    );
    let human = font_cli(&[os("fonts"), os("list"), os("--query"), os("lobster")]);
    assert_eq!(code(&human), 0);
    assert!(
        String::from_utf8_lossy(&human.stdout).starts_with("Lobster\tgoogle\tnot_downloaded\t"),
        "{}",
        String::from_utf8_lossy(&human.stdout)
    );

    let bundled = font_cli(&[os("fonts"), os("resolve"), os("inter"), os("--json")]);
    assert_eq!(code(&bundled), 0);
    let value = stdout_json(&bundled);
    assert_eq!(value["state"], "loaded");
    assert_eq!(value["origin"], "bundled");

    // A cold catalog family and an unknown family are typed states with the fallback named.
    for (family, state) in [("Lobster", "not_downloaded"), ("No Such Family", "unknown")] {
        let cold = font_cli(&[os("fonts"), os("resolve"), os(family), os("--json")]);
        assert_eq!(code(&cold), 4, "{family}");
        let value = stdout_json(&cold);
        assert_eq!(value["state"], state);
        assert_eq!(value["effective_family"], "Inter");
    }

    let refused = font_cli(&[os("fonts"), os("download"), os("Lobster"), os("--json")]);
    assert_eq!(code(&refused), 5);
    assert_eq!(error_json(&refused)["error"]["code"], "rejected");
    let outside = font_cli(&[
        os("fonts"),
        os("download"),
        os("Inter"),
        os("--accept-license"),
        os("--json"),
    ]);
    assert_eq!(code(&outside), 5);
    assert_eq!(code(&font_cli(&[os("fonts"), os("nope"), os("--json")])), 2);
    assert!(
        !home.path().join("VibeCut").join("fonts").exists(),
        "refusals write nothing"
    );
}

#[test]
fn custom_font_import_persists_across_cli_control_reopen_and_export() {
    use serde_json::{Value, json};
    use std::io::{BufRead, Write};
    use std::process::Stdio;
    use time::{Tick, TimeRange};
    if !crate::media_fixture::tools() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let cache = dir.path().join("isolated cache");
    let source = dir.path().join("private source font.ttf");
    // Reuse admitted OFL Noto Serif with an equal-length family rename, so the
    // rendered outline differs independently from the bundled Inter fallback.
    let mut font = include_bytes!("../../../../../../assets/fonts/NotoSerif-Regular.ttf").to_vec();
    let old: Vec<u8> = "Noto Serif"
        .encode_utf16()
        .flat_map(u16::to_be_bytes)
        .collect();
    let new: Vec<u8> = "User Serif"
        .encode_utf16()
        .flat_map(u16::to_be_bytes)
        .collect();
    let mut names = 0;
    for offset in 0..=font.len() - old.len() {
        if font[offset..offset + old.len()] == old {
            font[offset..offset + new.len()].copy_from_slice(&new);
            names += 1;
        }
    }
    assert!(names > 0);
    std::fs::write(&source, font).unwrap();
    let command = || {
        let mut command = Command::new(bin());
        command
            .env("HOME", &cache)
            .env("LOCALAPPDATA", &cache)
            .env("XDG_CACHE_HOME", &cache);
        command
    };
    let imported = command()
        .args([
            os("fonts"),
            os("import"),
            source.as_os_str(),
            os("--accept-license"),
            os("--json"),
        ])
        .output()
        .unwrap();
    assert_eq!(
        code(&imported),
        0,
        "{}",
        String::from_utf8_lossy(&imported.stderr)
    );
    let missing_acceptance = command()
        .args([os("fonts"), os("import"), source.as_os_str(), os("--json")])
        .output()
        .unwrap();
    assert_ne!(code(&missing_acceptance), 0);
    let project_path = dir.path().join("custom.vibecut");
    crate::cli::fixtures::overlay_project(&project_path, 320, 180, Tick(time::TICKS_PER_SECOND));
    let mut data = project::load(&project_path).unwrap().project.into_data();
    let track = data
        .scene_mut()
        .timeline
        .iter_mut()
        .find(|track| !track.items.is_empty())
        .unwrap();
    track.kind = project::TrackKind::Text;
    let item = &mut track.items[0];
    item.content = project::ItemParams::Text(project::TextParams {
        font: "User Serif".into(),
        font_size: 34.0,
        ..project::TextParams::new("Hello Aa \u{1eef}")
    });
    item.source_range = TimeRange::new(Tick::ZERO, Tick(time::TICKS_PER_SECOND)).unwrap();
    let project = project::Project::new(data).unwrap();
    let encoded = project::encode(&project, true).unwrap();
    assert!(!String::from_utf8_lossy(&encoded).contains("private source"));
    std::fs::write(&project_path, &encoded).unwrap();
    // Each invocation starts a fresh text registry. The control import also proves
    // exact-hash deduplication uses the real registry and job-publication lifetime.
    let snapshot = |name: &str, do_import: bool| {
        let mut child = command()
            .arg("control")
            .arg(&project_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut input = child.stdin.take().unwrap();
        let mut output = std::io::BufReader::new(child.stdout.take().unwrap());
        let mut send = |value: Value| -> Value {
            writeln!(input, "{value}").unwrap();
            input.flush().unwrap();
            let mut line = String::new();
            output.read_line(&mut line).unwrap();
            assert!(!line.is_empty(), "control unexpectedly closed");
            let response: Value = serde_json::from_str(&line).unwrap();
            assert_eq!(response["exit_code"], 0, "{response}");
            response
        };
        let inspection = send(json!({"request_id":"inspect","command":"inspect","params":{}}));
        let revision = inspection["rev_after"].clone();
        if do_import {
            let accepted = send(
                json!({"request_id":"import","command":"fonts.import","base_rev":revision,"idempotency_key":"import-once","params":{"path":source,"accept_license":true}}),
            );
            let job = accepted["value"]["value"]["job"].clone();
            assert!(
                accepted["value"]["value"]["paths"]
                    .as_array()
                    .unwrap()
                    .is_empty()
            );
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
            loop {
                let status = send(
                    json!({"request_id":"status","command":"job_status","params":{"job":job}}),
                );
                let state = &status["value"]["value"]["snapshot"]["state"];
                if state == "completed" {
                    break;
                }
                assert!(state != "failed" && state != "cancelled", "{status}");
                assert!(std::time::Instant::now() < deadline);
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
        let target = dir.path().join(name);
        send(
            json!({"request_id":"snapshot","command":"preview.snapshot","base_rev":revision,"idempotency_key":"snapshot","params":{"time":0,"path":target}}),
        );
        drop(send);
        drop(input);
        let result = child.wait_with_output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        target
    };
    let first = snapshot("first.png", true);
    std::fs::rename(&source, dir.path().join("moved original.ttf")).unwrap();
    let resolved = command()
        .args(["fonts", "resolve", "User Serif", "--json"])
        .output()
        .unwrap();
    assert_eq!(code(&resolved), 0);
    assert_eq!(stdout_json(&resolved)["origin"], "custom");
    let second = snapshot("reopened.png", false);
    assert_eq!(
        std::fs::read(&first).unwrap(),
        std::fs::read(&second).unwrap(),
        "fresh-process reopen preserves the raster after source relocation"
    );
    let export = dir.path().join("custom.webm");
    let settings = serde_json::to_string(&render::OutputOptions {
        codec: render::VideoCodec::Vp9,
        ..Default::default()
    })
    .unwrap();
    let exported = command()
        .args([
            os("export"),
            project_path.as_os_str(),
            os("--out"),
            export.as_os_str(),
            os("--settings"),
            os(&settings),
            os("--json"),
        ])
        .output()
        .unwrap();
    assert_eq!(
        code(&exported),
        0,
        "{}",
        String::from_utf8_lossy(&exported.stderr)
    );
    let preview = crate::cli::fixtures::decoded_rgb(&second, 320, 180, &[0]).remove(0);
    let video = crate::cli::fixtures::decoded_rgb(&export, 320, 180, &[0]).remove(0);
    let mean_error = preview
        .iter()
        .zip(&video)
        .map(|(a, b)| (f64::from(*a) - f64::from(*b)).abs())
        .sum::<f64>()
        / preview.len() as f64;
    assert!(
        mean_error < 3.0,
        "preview/export mean RGB error {mean_error}"
    );
    #[cfg(target_os = "macos")]
    let fonts = cache.join("Library/Caches/VibeCut/fonts");
    #[cfg(not(target_os = "macos"))]
    let fonts = cache.join("VibeCut/fonts");
    std::fs::rename(&fonts, dir.path().join("retained fonts")).unwrap();
    let fallback = snapshot("missing-font.png", false);
    let fallback = crate::cli::fixtures::decoded_rgb(&fallback, 320, 180, &[0]).remove(0);
    assert!(
        preview
            .iter()
            .zip(&fallback)
            .filter(|(a, b)| a.abs_diff(**b) > 20)
            .count()
            > 100,
        "a cold missing store must visibly use fallback, proving custom bytes were consumed earlier"
    );
    let reopened = project::load(&project_path).unwrap().project;
    assert_eq!(
        reopened, project,
        "font work and missing-cache fallback never rewrite the authored project"
    );
}

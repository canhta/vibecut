use super::media_fixture::decoded_rgb;
use super::process::{cli, code, error_json, os, stdout_json};
use super::project_fixture::{overlay_project, overlay_project_color};
use crate::common::{generate_av, media_project, tools};
use serde_json::{Value, json};
use std::fs;
use std::path::Path;
use std::process::Output;
use time::Tick;

fn write_job(path: &Path, value: Value) {
    fs::write(path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
}

fn batch_refusal(output: &Output, exit: i32) -> Value {
    assert_eq!(
        code(output),
        exit,
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    error_json(output)
}

fn batch_result(output: &Output, exit: i32) -> Value {
    assert_eq!(
        code(output),
        exit,
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    stdout_json(output)
}

#[test]
fn batch_exports_in_file_order_without_mutating_projects() {
    let dir = tempfile::tempdir().unwrap();
    let first_project = dir.path().join("đầu tiên.vibecut");
    let second_project = dir.path().join("thứ hai.vibecut");
    overlay_project_color(
        &first_project,
        64,
        64,
        Tick(time::TICKS_PER_SECOND),
        [255, 0, 0, 255],
    );
    overlay_project_color(
        &second_project,
        64,
        64,
        Tick(time::TICKS_PER_SECOND),
        [0, 255, 0, 255],
    );
    let output_dir = dir.path().join("out");
    fs::create_dir(&output_dir).unwrap();
    let job_file = dir.path().join("jobs.json");
    write_job(
        &job_file,
        json!({
            "version": 1,
            "jobs": [
                {"project": "đầu tiên.vibecut", "output": "out/kết quả 1.mp4"},
                {"project": "thứ hai.vibecut", "output": "out/kết quả 2.mp4"}
            ]
        }),
    );
    let first_before = fs::read(&first_project).unwrap();
    let second_before = fs::read(&second_project).unwrap();
    let output = cli(&[os("batch"), job_file.as_os_str(), os("--json")]);
    assert_eq!(
        code(&output),
        0,
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value = stdout_json(&output);
    assert_eq!(value["status"], "completed");
    let jobs = value["jobs"].as_array().unwrap();
    assert_eq!(jobs.len(), 2);
    assert_eq!(jobs[0]["index"], 0);
    assert_eq!(jobs[0]["status"], "completed");
    assert_eq!(jobs[1]["index"], 1);
    assert_eq!(jobs[1]["status"], "completed");
    let first_output = output_dir.join("kết quả 1.mp4");
    let second_output = output_dir.join("kết quả 2.mp4");
    assert_eq!(
        jobs[0]["output"],
        json_path_string(&first_output),
        "result order must follow job-file order"
    );
    assert_eq!(jobs[1]["output"], json_path_string(&second_output));
    assert!(fs::metadata(&first_output).unwrap().len() > 0);
    assert!(fs::metadata(&second_output).unwrap().len() > 0);
    assert_eq!(fs::read(&first_project).unwrap(), first_before);
    assert_eq!(fs::read(&second_project).unwrap(), second_before);
    if tools() {
        let first = decoded_rgb(&first_output, 64, 64, &[0]);
        let second = decoded_rgb(&second_output, 64, 64, &[0]);
        for (actual, expected) in first[0][..3].iter().zip([255, 0, 0]) {
            assert!(
                (i32::from(*actual) - expected).abs() <= 12,
                "first job exported the wrong project: {actual}"
            );
        }
        for (actual, expected) in second[0][..3].iter().zip([0, 255, 0]) {
            assert!(
                (i32::from(*actual) - expected).abs() <= 12,
                "second job exported the wrong project: {actual}"
            );
        }
    }
}

fn json_path_string(path: &Path) -> String {
    fs::canonicalize(path)
        .unwrap()
        .to_string_lossy()
        .into_owned()
}

#[test]
fn batch_refuses_malformed_oversized_and_hostile_job_files() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("a.vibecut");
    overlay_project(&project, 64, 64, Tick(time::TICKS_PER_SECOND));
    let output_dir = dir.path().join("out");
    fs::create_dir(&output_dir).unwrap();

    let case = |name: &str, bytes: Vec<u8>, exit: i32| {
        let path = dir.path().join(name);
        fs::write(&path, bytes).unwrap();
        let output = cli(&[os("batch"), path.as_os_str(), os("--json")]);
        batch_refusal(&output, exit);
    };
    case("truncated.json", b"{".to_vec(), 2);
    case(
        "deep.json",
        format!("{}0{}", "[".repeat(64), "]".repeat(64)).into_bytes(),
        2,
    );
    case(
        "empty-path.json",
        br#"{"version":1,"jobs":[{"project":"a.vibecut","output":""}]}"#.to_vec(),
        2,
    );
    case(
        "duplicate.json",
        br#"{"version":1,"version":1,"jobs":[]}"#.to_vec(),
        2,
    );
    case(
        "unknown.json",
        br#"{"version":1,"jobs":[],"extra":true}"#.to_vec(),
        2,
    );
    case("empty.json", br#"{"version":1,"jobs":[]}"#.to_vec(), 2);
    case("version.json", br#"{"version":2,"jobs":[]}"#.to_vec(), 5);
    case(
        "nul.json",
        json!({"version":1,"jobs":[{"project":"a.vibecut","output":"bad\u{0}name.mp4"}]})
            .to_string()
            .into_bytes(),
        2,
    );
    case(
        "long.json",
        json!({
            "version":1,
            "jobs":[{"project":"a.vibecut","output":"a".repeat(4097)}]
        })
        .to_string()
        .into_bytes(),
        2,
    );
    let too_many: Vec<Value> = (0..1001)
        .map(|index| {
            json!({
                "project": "a.vibecut",
                "output": format!("out/{index}.mp4")
            })
        })
        .collect();
    case(
        "too-many.json",
        json!({"version":1,"jobs":too_many})
            .to_string()
            .into_bytes(),
        2,
    );
    case("oversized.json", vec![b' '; (1 << 20) + 1], 2);
    let directory = dir.path().join("directory.json");
    fs::create_dir(&directory).unwrap();
    let output = cli(&[os("batch"), directory.as_os_str(), os("--json")]);
    batch_refusal(&output, 2);
    let missing = cli(&[
        os("batch"),
        dir.path().join("missing.json").as_os_str(),
        os("--json"),
    ]);
    batch_refusal(&missing, 7);
}

#[test]
fn batch_refuses_unsafe_or_ambiguous_destinations() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("a.vibecut");
    overlay_project(&project, 64, 64, Tick(time::TICKS_PER_SECOND));
    let output_dir = dir.path().join("out");
    fs::create_dir(&output_dir).unwrap();
    let run = |name: &str, value: Value, flags: &[&str], exit: i32| {
        let path = dir.path().join(name);
        write_job(&path, value);
        let mut args = vec![os("batch"), path.as_os_str(), os("--json")];
        args.extend(flags.iter().map(|flag| os(flag)));
        let output = cli(&args);
        batch_refusal(&output, exit);
    };
    run(
        "duplicate-output.json",
        json!({"version":1,"jobs":[
            {"project":"a.vibecut","output":"out/same.mp4"},
            {"project":"a.vibecut","output":"out/same.mp4"}
        ]}),
        &[],
        2,
    );
    #[cfg(any(windows, target_os = "macos"))]
    run(
        "case-output.json",
        json!({"version":1,"jobs":[
            {"project":"a.vibecut","output":"out/Result.mp4","overwrite":true},
            {"project":"a.vibecut","output":"out/result.mp4","overwrite":true}
        ]}),
        &["--allow-overwrite"],
        2,
    );
    #[cfg(any(windows, target_os = "macos"))]
    run(
        "unicode-case-output.json",
        json!({"version":1,"jobs":[
            {"project":"a.vibecut","output":"out/KẾT QUẢ.mp4","overwrite":true},
            {"project":"a.vibecut","output":"out/kết quả.mp4","overwrite":true}
        ]}),
        &["--allow-overwrite"],
        2,
    );
    #[cfg(target_os = "macos")]
    run(
        "normalized-output.json",
        json!({"version":1,"jobs":[
            {"project":"a.vibecut","output":"out/kết quả.mp4","overwrite":true},
            {"project":"a.vibecut","output":"out/ke\u{0302}\u{0301}t qua\u{0309}.mp4","overwrite":true}
        ]}),
        &["--allow-overwrite"],
        2,
    );
    let first_alias = output_dir.join("first-alias.mp4");
    let second_alias = output_dir.join("second-alias.mp4");
    fs::write(&first_alias, b"previous").unwrap();
    fs::hard_link(&first_alias, &second_alias).unwrap();
    run(
        "hard-linked-outputs.json",
        json!({"version":1,"jobs":[
            {"project":"a.vibecut","output":"out/first-alias.mp4","overwrite":true},
            {"project":"a.vibecut","output":"out/second-alias.mp4","overwrite":true}
        ]}),
        &["--allow-overwrite"],
        2,
    );
    assert_eq!(fs::read(&first_alias).unwrap(), b"previous");
    assert_eq!(fs::read(&second_alias).unwrap(), b"previous");
    run(
        "aliases-project.json",
        json!({"version":1,"jobs":[
            {"project":"a.vibecut","output":"a.vibecut","overwrite":true}
        ]}),
        &["--allow-overwrite"],
        2,
    );
    run(
        "aliases-job-file.json",
        json!({"version":1,"jobs":[
            {"project":"a.vibecut","output":"aliases-job-file.json","overwrite":true}
        ]}),
        &["--allow-overwrite"],
        2,
    );
    run(
        "aliases-metadata.json",
        json!({"version":1,"jobs":[
            {"project":"a.vibecut","output":"a.vibecut.meta.json","overwrite":true}
        ]}),
        &["--allow-overwrite"],
        2,
    );
    #[cfg(any(windows, target_os = "macos"))]
    run(
        "aliases-metadata-case.json",
        json!({"version":1,"jobs":[
            {"project":"a.vibecut","output":"A.VIBECUT.META.JSON","overwrite":true}
        ]}),
        &["--allow-overwrite"],
        2,
    );
    run(
        "reserved-lock.json",
        json!({"version":1,"jobs":[
            {"project":"a.vibecut","output":"out/.export.lock","overwrite":true}
        ]}),
        &["--allow-overwrite"],
        5,
    );
    run(
        "missing-directory.json",
        json!({"version":1,"jobs":[
            {"project":"a.vibecut","output":"missing/out.mp4"}
        ]}),
        &[],
        2,
    );
    let nested = output_dir.join("nested.mp4");
    fs::create_dir(&nested).unwrap();
    run(
        "non-regular-output.json",
        json!({"version":1,"jobs":[
            {"project":"a.vibecut","output":"out/nested.mp4","overwrite":true}
        ]}),
        &["--allow-overwrite"],
        2,
    );
    let existing = output_dir.join("existing.mp4");
    fs::write(&existing, b"previous").unwrap();
    run(
        "existing-output.json",
        json!({"version":1,"jobs":[
            {"project":"a.vibecut","output":"out/existing.mp4"}
        ]}),
        &[],
        5,
    );
    assert_eq!(fs::read(&existing).unwrap(), b"previous");
    run(
        "overwrite-without-grant.json",
        json!({"version":1,"jobs":[
            {"project":"a.vibecut","output":"out/existing.mp4","overwrite":true}
        ]}),
        &[],
        5,
    );
    assert_eq!(fs::read(&existing).unwrap(), b"previous");
    run(
        "degraded-without-grant.json",
        json!({"version":1,"jobs":[
            {"project":"a.vibecut","output":"out/degraded.mp4","allow_degraded":true}
        ]}),
        &[],
        5,
    );
    assert!(!dir.path().join("out/degraded.mp4").exists());
    let output = cli(&[
        os("batch"),
        dir.path().join("overwrite-without-grant.json").as_os_str(),
        os("--allow-overwrite"),
        os("--json"),
    ]);
    assert_eq!(
        code(&output),
        0,
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_ne!(fs::read(&existing).unwrap(), b"previous");
}

#[test]
fn batch_reports_missing_project_and_empty_timeline() {
    let dir = tempfile::tempdir().unwrap();
    let output_dir = dir.path().join("out");
    fs::create_dir(&output_dir).unwrap();
    let missing_job = dir.path().join("missing-project.json");
    write_job(
        &missing_job,
        json!({"version":1,"jobs":[
            {"project":"missing.vibecut","output":"out/missing.mp4"}
        ]}),
    );
    let output = cli(&[os("batch"), missing_job.as_os_str(), os("--json")]);
    batch_refusal(&output, 7);

    let empty = dir.path().join("empty.vibecut");
    let project = project::Project::empty(project::ProjectId(0), "empty".into()).unwrap();
    fs::write(&empty, project::encode(&project, false).unwrap()).unwrap();
    let empty_job = dir.path().join("empty-project.json");
    write_job(
        &empty_job,
        json!({"version":1,"jobs":[
            {"project":"empty.vibecut","output":"out/empty.mp4"}
        ]}),
    );
    let output = cli(&[os("batch"), empty_job.as_os_str(), os("--json")]);
    let value = batch_result(&output, 5);
    assert_eq!(value["jobs"][0]["error"]["code"], "rejected");
    assert!(!dir.path().join("out/empty.mp4").exists());
}

#[test]
fn batch_stops_after_failure_without_promoting_partial_output() {
    if !tools() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.mp4");
    let good = dir.path().join("good.vibecut");
    let broken = dir.path().join("broken.vibecut");
    overlay_project(&good, 64, 64, Tick(time::TICKS_PER_SECOND));
    generate_av(&source);
    media_project(&broken, &source, None);
    fs::remove_file(&source).unwrap();
    let output_dir = dir.path().join("out");
    fs::create_dir(&output_dir).unwrap();
    let preserved = output_dir.join("broken.mp4");
    fs::write(&preserved, b"previous").unwrap();
    let job_file = dir.path().join("jobs.json");
    write_job(
        &job_file,
        json!({"version":1,"jobs":[
            {"project":"good.vibecut","output":"out/good.mp4"},
            {"project":"broken.vibecut","output":"out/broken.mp4","overwrite":true},
            {"project":"good.vibecut","output":"out/after.mp4"}
        ]}),
    );
    let output = cli(&[
        os("batch"),
        job_file.as_os_str(),
        os("--allow-overwrite"),
        os("--json"),
    ]);
    let value = batch_result(&output, 4);
    assert_eq!(value["jobs"][1]["error"]["code"], "unavailable");
    assert!(output_dir.join("good.mp4").exists());
    assert_eq!(fs::read(&preserved).unwrap(), b"previous");
    assert!(!output_dir.join("after.mp4").exists());
}

#[test]
fn batch_allow_degraded_is_explicit_consent() {
    if !tools() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.mp4");
    let project = dir.path().join("degraded.vibecut");
    generate_av(&source);
    media_project(&project, &source, None);
    fs::remove_file(&source).unwrap();
    let output_dir = dir.path().join("out");
    fs::create_dir(&output_dir).unwrap();
    let job_file = dir.path().join("jobs.json");
    write_job(
        &job_file,
        json!({"version":1,"jobs":[
            {"project":"degraded.vibecut","output":"out/degraded.mp4","allow_degraded":true}
        ]}),
    );
    let refused = cli(&[os("batch"), job_file.as_os_str(), os("--json")]);
    batch_refusal(&refused, 5);
    assert!(!output_dir.join("degraded.mp4").exists());
    let accepted = cli(&[
        os("batch"),
        job_file.as_os_str(),
        os("--allow-degraded"),
        os("--json"),
    ]);
    assert_eq!(
        code(&accepted),
        0,
        "{}",
        String::from_utf8_lossy(&accepted.stderr)
    );
    assert!(output_dir.join("degraded.mp4").exists());
}

#[test]
fn batch_refuses_an_output_that_aliases_another_jobs_source() {
    if !tools() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.mp4");
    let media = dir.path().join("media.vibecut");
    let overlay = dir.path().join("overlay.vibecut");
    generate_av(&source);
    media_project(&media, &source, None);
    overlay_project(&overlay, 64, 64, Tick(time::TICKS_PER_SECOND));
    let output_dir = dir.path().join("out");
    fs::create_dir(&output_dir).unwrap();
    let job_file = dir.path().join("jobs.json");
    write_job(
        &job_file,
        json!({"version":1,"jobs":[
            {"project":"media.vibecut","output":"out/media.mp4"},
            {"project":"overlay.vibecut","output":"source.mp4","overwrite":true}
        ]}),
    );
    let output = cli(&[
        os("batch"),
        job_file.as_os_str(),
        os("--allow-overwrite"),
        os("--json"),
    ]);
    batch_refusal(&output, 2);
    assert!(source.exists());
    assert!(!output_dir.join("media.mp4").exists());
}

#[test]
fn batch_refuses_an_output_hard_linked_to_another_jobs_source() {
    if !tools() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source.mp4");
    let media = dir.path().join("media.vibecut");
    let overlay = dir.path().join("overlay.vibecut");
    generate_av(&source);
    let source_before = fs::read(&source).unwrap();
    media_project(&media, &source, None);
    overlay_project(&overlay, 64, 64, Tick(time::TICKS_PER_SECOND));
    let output_dir = dir.path().join("out");
    fs::create_dir(&output_dir).unwrap();
    let linked = output_dir.join("linked.mp4");
    fs::hard_link(&source, &linked).unwrap();
    let job_file = dir.path().join("jobs.json");
    write_job(
        &job_file,
        json!({"version":1,"jobs":[
            {"project":"media.vibecut","output":"out/media.mp4"},
            {"project":"overlay.vibecut","output":"out/linked.mp4","overwrite":true}
        ]}),
    );
    let output = cli(&[
        os("batch"),
        job_file.as_os_str(),
        os("--allow-overwrite"),
        os("--json"),
    ]);
    batch_refusal(&output, 2);
    assert_eq!(fs::read(&source).unwrap(), source_before);
    assert_eq!(fs::read(&linked).unwrap(), source_before);
    assert!(!output_dir.join("media.mp4").exists());
}

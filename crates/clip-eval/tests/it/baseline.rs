#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
use clip_eval::{Options, Provider};
use std::path::{Path, PathBuf};

fn options(
    root: &Path,
    out: &Path,
    labels: Option<PathBuf>,
    energy: bool,
    k: usize,
    baseline: Option<PathBuf>,
    write: Option<PathBuf>,
) -> Options {
    Options {
        set: "synthetic".into(),
        labels,
        provider: Provider::Heuristic,
        model: "fixture-text-model".into(),
        asr_model: "fixture-asr-model".into(),
        accept_unknown_price: false,
        live: false,
        max_cost_micros: 0,
        k,
        out_dir: out.to_path_buf(),
        repo_root: root.to_path_buf(),
        energy,
        transcribe: false,
        baseline,
        write_baseline: write,
        baseline_revision: Some("pinned-main-test".into()),
        auto_baseline: false,
    }
}

fn root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap()
}

fn copy_fixtures(dest: &Path) -> PathBuf {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/synthetic");
    std::fs::create_dir_all(dest).unwrap();
    for name in [
        "en-single.json",
        "en-single.transcript.json",
        "vi-multi.json",
        "vi-multi.transcript.json",
    ] {
        std::fs::copy(source.join(name), dest.join(name)).unwrap();
    }
    dest.to_path_buf()
}

fn write_pinned(labels: &Path, out: &Path, energy: bool, k: usize) -> PathBuf {
    let path = out.join("baseline.json");
    clip_eval::run(&options(
        root(),
        out,
        Some(labels.to_path_buf()),
        energy,
        k,
        None,
        Some(path.clone()),
    ))
    .unwrap();
    path
}

fn assert_refused(labels: &Path, out: &Path, energy: bool, k: usize, baseline: PathBuf) {
    let error = clip_eval::run(&options(
        root(),
        out,
        Some(labels.to_path_buf()),
        energy,
        k,
        Some(baseline),
        None,
    ))
    .unwrap_err();
    assert!(matches!(
        error,
        clip_eval::Error::BaselineMismatch(label) if label == "en-single"
    ));
}

#[test]
fn candidate_metrics_are_compared_against_pinned_main_outputs() {
    let out = tempfile::tempdir().unwrap();
    let baseline_path = out.path().join("baseline.json");
    clip_eval::run(&options(
        root(),
        out.path(),
        None,
        false,
        5,
        None,
        Some(baseline_path.clone()),
    ))
    .unwrap();

    let mut baseline: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&baseline_path).unwrap()).unwrap();
    let english = baseline
        .get_mut("videos")
        .and_then(|videos| videos.get_mut("en-single"))
        .unwrap();
    english["hit_rate"] = 0.0.into();
    english["must_hit"] = 0.into();
    english["precision_at_k"] = 0.0.into();
    english["precision_matches"] = 0.into();
    english["boundary_quality"] = 0.0.into();
    english["duration_compliance"] = 0.0.into();
    std::fs::write(
        &baseline_path,
        serde_json::to_vec_pretty(&baseline).unwrap(),
    )
    .unwrap();

    let output = clip_eval::run(&options(
        root(),
        out.path(),
        None,
        false,
        5,
        Some(baseline_path),
        None,
    ))
    .unwrap();
    let english = output
        .report
        .videos
        .iter()
        .find(|video| video.label == "en-single")
        .unwrap();
    assert_eq!(english.current.must_hit, 1);
    assert_eq!(english.main_baseline.as_ref().unwrap().must_hit, 0);
    assert_eq!(
        output.report.main_baseline_revision.as_deref(),
        Some("pinned-main-test")
    );
    assert_eq!(output.report.aggregate.current.must_hit, 1);
    assert_eq!(
        output
            .report
            .aggregate
            .main_baseline
            .as_ref()
            .unwrap()
            .must_hit,
        0
    );
    let markdown = std::fs::read_to_string(&output.markdown_path).unwrap();
    assert!(markdown.contains("pinned-main-test"));
    assert!(markdown.contains("Hit main"));
}

#[test]
fn mismatched_pinned_inputs_are_refused() {
    let out = tempfile::tempdir().unwrap();
    let baseline_path = out.path().join("baseline.json");
    clip_eval::run(&options(
        root(),
        out.path(),
        None,
        false,
        5,
        None,
        Some(baseline_path.clone()),
    ))
    .unwrap();
    let mut baseline: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&baseline_path).unwrap()).unwrap();
    baseline["inputs"]["en-single"]["source_sha256"] = "0".repeat(64).into();
    std::fs::write(
        &baseline_path,
        serde_json::to_vec_pretty(&baseline).unwrap(),
    )
    .unwrap();
    assert_refused(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures/synthetic")
            .as_path(),
        out.path(),
        false,
        5,
        baseline_path,
    );
}

#[test]
fn changed_picks_are_refused_by_the_pinned_baseline() {
    let out = tempfile::tempdir().unwrap();
    let labels = copy_fixtures(&out.path().join("labels"));
    let baseline = write_pinned(&labels, out.path(), false, 5);
    let mut label: serde_json::Value =
        serde_json::from_slice(&std::fs::read(labels.join("en-single.json")).unwrap()).unwrap();
    label["clips"][0]["priority"] = "nice".into();
    std::fs::write(
        labels.join("en-single.json"),
        serde_json::to_vec_pretty(&label).unwrap(),
    )
    .unwrap();
    assert_refused(&labels, out.path(), false, 5, baseline);
}

#[test]
fn changed_k_is_refused_by_the_pinned_baseline() {
    let out = tempfile::tempdir().unwrap();
    let labels = copy_fixtures(&out.path().join("labels"));
    let baseline = write_pinned(&labels, out.path(), false, 5);
    assert_refused(&labels, out.path(), false, 1, baseline);
}

#[test]
fn changed_energy_mode_is_refused_by_the_pinned_baseline() {
    let out = tempfile::tempdir().unwrap();
    let labels = copy_fixtures(&out.path().join("labels"));
    let baseline = write_pinned(&labels, out.path(), false, 5);
    assert_refused(&labels, out.path(), true, 5, baseline);
}

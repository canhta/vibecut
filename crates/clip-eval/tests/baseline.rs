use clip_eval::{Options, Provider};
use std::path::Path;

fn options(
    root: &Path,
    out: &Path,
    baseline: Option<std::path::PathBuf>,
    write: Option<std::path::PathBuf>,
) -> Options {
    Options {
        set: "synthetic".into(),
        labels: None,
        provider: Provider::Heuristic,
        live: false,
        max_cost_micros: 0,
        k: 5,
        out_dir: out.to_path_buf(),
        repo_root: root.to_path_buf(),
        energy: false,
        transcribe: false,
        baseline,
        write_baseline: write,
        baseline_revision: Some("pinned-main-test".into()),
    }
}

#[test]
fn candidate_metrics_are_compared_against_pinned_main_outputs() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap();
    let out = tempfile::tempdir().unwrap();
    let baseline_path = out.path().join("baseline.json");
    clip_eval::run(&options(
        root,
        out.path(),
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

    let output = clip_eval::run(&options(root, out.path(), Some(baseline_path), None)).unwrap();
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
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap();
    let out = tempfile::tempdir().unwrap();
    let baseline_path = out.path().join("baseline.json");
    clip_eval::run(&options(
        root,
        out.path(),
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
    let error = clip_eval::run(&options(root, out.path(), Some(baseline_path), None)).unwrap_err();
    assert!(matches!(
        error,
        clip_eval::Error::BaselineMismatch(label) if label == "en-single"
    ));
}

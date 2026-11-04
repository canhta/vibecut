use clip_eval::{Options, Provider};
use std::path::Path;

#[test]
fn synthetic_set_runs_end_to_end_and_writes_both_reports() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap();
    let out = tempfile::tempdir().unwrap();
    let output = clip_eval::run(&Options {
        set: "synthetic".into(),
        labels: None,
        provider: Provider::Heuristic,
        live: false,
        max_cost_micros: 0,
        k: 5,
        out_dir: out.path().to_path_buf(),
        repo_root: root.to_path_buf(),
        energy: false,
        transcribe: false,
        baseline: None,
        write_baseline: None,
        baseline_revision: None,
    })
    .unwrap();
    assert_eq!(output.report.videos.len(), 2);
    assert_eq!(output.report.aggregate.current.must_total, 2);
    assert_eq!(output.report.aggregate.current.must_hit, 1);
    let english = output
        .report
        .videos
        .iter()
        .find(|video| video.label == "en-single")
        .unwrap();
    assert_eq!(english.current.must_hit, 1);
    assert_eq!(english.current.boundary_quality, Some(1.0));
    let vietnamese = output
        .report
        .videos
        .iter()
        .find(|video| video.label == "vi-multi")
        .unwrap();
    assert_eq!(vietnamese.current.must_hit, 0);
    assert!(output.json_path.exists());
    assert!(output.markdown_path.exists());
    let markdown = std::fs::read_to_string(&output.markdown_path).unwrap();
    assert!(markdown.contains("## TL;DR"));
    assert!(markdown.contains("Heuristic baseline"));
    assert!(markdown.contains("Main baseline"));
}

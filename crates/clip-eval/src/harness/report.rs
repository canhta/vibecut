//! Owns the persisted evaluation report and its Markdown rendering.

use crate::Result;
use crate::metrics::Metrics;
use ai::text::clips::ClipLabel;
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Serialize)]
pub struct SuggestionReport {
    pub score: f64,
    pub source: String,
    pub label: ClipLabel,
    pub title: String,
    pub segments: Vec<RangeReport>,
}

fn label_name(label: ClipLabel) -> &'static str {
    match label {
        ClipLabel::Hook => "hook",
        ClipLabel::OnTopic => "on_topic",
        ClipLabel::CleanStart => "clean_start",
        ClipLabel::Standalone => "standalone",
    }
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct RangeReport {
    pub start: i64,
    pub end: i64,
}

#[derive(Clone, Debug, Serialize)]
pub struct VideoReport {
    pub label: String,
    pub source: String,
    pub sha256: String,
    pub language: String,
    pub transcript: String,
    pub transcript_cache_hit: bool,
    pub current: Metrics,
    pub baseline: Metrics,
    pub main_baseline: Option<Metrics>,
    pub suggestions: Vec<SuggestionReport>,
    pub notes: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Aggregate {
    pub videos: usize,
    pub current: Metrics,
    pub baseline: Metrics,
    pub main_baseline: Option<Metrics>,
    pub current_cost_micros: Option<u64>,
    pub baseline_cost_micros: Option<u64>,
    pub wall_ms: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Report {
    pub schema: u32,
    pub set: String,
    pub provider: String,
    pub model: String,
    pub live: bool,
    pub k: usize,
    pub revision: String,
    pub baseline: String,
    pub baseline_revision: String,
    pub main_baseline_revision: Option<String>,
    pub max_cost_micros: u64,
    pub aggregate: Aggregate,
    pub videos: Vec<VideoReport>,
    pub notes: Vec<String>,
}

pub(super) fn write_report(
    report: &Report,
    out_dir: &Path,
    set: &str,
) -> Result<(PathBuf, PathBuf)> {
    std::fs::create_dir_all(out_dir)?;
    let stem = format!(
        "{}-{}-{}",
        sanitize(set),
        sanitize(&report.provider),
        crate::label::sha256_bytes(report.model.as_bytes())
    );
    let json_path = out_dir.join(format!("{stem}.json"));
    let markdown_path = out_dir.join(format!("{stem}.md"));
    std::fs::write(&json_path, serde_json::to_vec_pretty(report)?)?;
    std::fs::write(&markdown_path, markdown(report))?;
    Ok((json_path, markdown_path))
}

pub(super) fn markdown(report: &Report) -> String {
    let mut out = String::new();
    out.push_str("## TL;DR\n");
    out.push_str(&format!(
        "- Set `{}` ({}), provider `{}` ({}), k={}.\n",
        report.set,
        report.videos.len(),
        report.provider,
        if report.live { "live" } else { "offline" },
        report.k
    ));
    out.push_str(&format!(
        "- Hit rate: current {:.3} vs baseline {:.3}.\n",
        report.aggregate.current.hit_rate, report.aggregate.baseline.hit_rate
    ));
    out.push_str(&format!(
        "- Precision@{}: current {:.3} vs baseline {:.3}.\n",
        report.k, report.aggregate.current.precision_at_k, report.aggregate.baseline.precision_at_k
    ));
    out.push_str(&format!(
        "- Boundary quality: current {} vs baseline {}.\n",
        percent(report.aggregate.current.boundary_quality),
        percent(report.aggregate.baseline.boundary_quality)
    ));
    out.push_str(&format!(
        "- Duration compliance: current {:.3} vs baseline {:.3}.\n",
        report.aggregate.current.duration_compliance, report.aggregate.baseline.duration_compliance
    ));
    out.push_str(&format!(
        "- Cost estimate: current {} vs baseline {}; wall {} ms.\n",
        cost(report.aggregate.current_cost_micros),
        cost(report.aggregate.baseline_cost_micros),
        report.aggregate.wall_ms
    ));
    out.push_str(&format!(
        "- Revision `{}`; heuristic baseline at `{}`; main baseline {}.\n",
        report.revision,
        report.baseline_revision,
        report.main_baseline_revision.as_deref().map_or_else(
            || "not measured".to_owned(),
            |revision| format!("`{revision}`")
        )
    ));
    out.push_str("\n## Aggregate\n\n");
    out.push_str(
        "| Metric | Current | Heuristic baseline | Main baseline |\n|---|---:|---:|---:|\n",
    );
    let main = report.aggregate.main_baseline.as_ref();
    out.push_str(&format!(
        "| Must hit rate | {:.3} ({}/{}) | {:.3} ({}/{}) | {} |\n",
        report.aggregate.current.hit_rate,
        report.aggregate.current.must_hit,
        report.aggregate.current.must_total,
        report.aggregate.baseline.hit_rate,
        report.aggregate.baseline.must_hit,
        report.aggregate.baseline.must_total,
        main.map_or_else(
            || "n/a".into(),
            |metrics| format!(
                "{:.3} ({}/{})",
                metrics.hit_rate, metrics.must_hit, metrics.must_total
            )
        )
    ));
    out.push_str(&format!(
        "| Precision@{} | {:.3} ({}/{}) | {:.3} ({}/{}) | {} |\n",
        report.k,
        report.aggregate.current.precision_at_k,
        report.aggregate.current.precision_matches,
        report.aggregate.current.precision_k,
        report.aggregate.baseline.precision_at_k,
        report.aggregate.baseline.precision_matches,
        report.aggregate.baseline.precision_k,
        main.map_or_else(
            || "n/a".into(),
            |metrics| format!(
                "{:.3} ({}/{})",
                metrics.precision_at_k, metrics.precision_matches, metrics.precision_k
            )
        )
    ));
    out.push_str(&format!(
        "| Boundary quality | {} | {} | {} |\n",
        percent(report.aggregate.current.boundary_quality),
        percent(report.aggregate.baseline.boundary_quality),
        main.map_or_else(|| "n/a".into(), |metrics| percent(metrics.boundary_quality))
    ));
    out.push_str(&format!(
        "| Duration compliance | {:.3} | {:.3} | {} |\n",
        report.aggregate.current.duration_compliance,
        report.aggregate.baseline.duration_compliance,
        main.map_or_else(
            || "n/a".into(),
            |metrics| format!("{:.3}", metrics.duration_compliance)
        )
    ));
    out.push_str(&format!(
        "| Snap failures | {} | {} | {} |\n",
        report.aggregate.current.snap_failures,
        report.aggregate.baseline.snap_failures,
        main.map_or_else(|| "n/a".into(), |metrics| metrics.snap_failures.to_string())
    ));
    out.push_str(&format!(
        "| Cost (upper bound) | {} | {} | {} |\n",
        cost(report.aggregate.current_cost_micros),
        cost(report.aggregate.baseline_cost_micros),
        main.map_or_else(|| "n/a".into(), |metrics| cost(metrics.cost_micros))
    ));
    out.push_str(&format!(
        "| Wall time (ms) | {} | {} | {} |\n",
        report.aggregate.current.wall_ms,
        report.aggregate.baseline.wall_ms,
        main.map_or_else(|| "n/a".into(), |metrics| metrics.wall_ms.to_string())
    ));
    out.push_str("\n## Videos\n\n");
    out.push_str("| Video | Hit cur | Hit heuristic | Hit main | P@k cur | P@k heuristic | P@k main | Boundary cur | Boundary heuristic | Boundary main | Duration cur | Duration heuristic | Duration main | Suggestions | Cost USD | Wall ms |\n");
    out.push_str(
        "|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|\n",
    );
    for video in &report.videos {
        let main = video.main_baseline.as_ref();
        out.push_str(&format!(
            "| `{}` | {:.3} ({}/{}) | {:.3} ({}/{}) | {} | {:.3} | {:.3} | {} | {} | {} | {} | {:.3} | {:.3} | {} | {} | {} | {} |\n",
            video.label,
            video.current.hit_rate,
            video.current.must_hit,
            video.current.must_total,
            video.baseline.hit_rate,
            video.baseline.must_hit,
            video.baseline.must_total,
            main.map_or_else(
                || "n/a".into(),
                |metrics| format!(
                    "{:.3} ({}/{})",
                    metrics.hit_rate, metrics.must_hit, metrics.must_total
                )
            ),
            video.current.precision_at_k,
            video.baseline.precision_at_k,
            main.map_or_else(
                || "n/a".into(),
                |metrics| format!("{:.3}", metrics.precision_at_k)
            ),
            percent(video.current.boundary_quality),
            percent(video.baseline.boundary_quality),
            main.map_or_else(
                || "n/a".into(),
                |metrics| percent(metrics.boundary_quality)
            ),
            video.current.duration_compliance,
            video.baseline.duration_compliance,
            main.map_or_else(
                || "n/a".into(),
                |metrics| format!("{:.3}", metrics.duration_compliance)
            ),
            video.current.suggestions,
            cost(video.current.cost_micros),
            video.current.wall_ms
        ));
    }
    out.push_str("\n## Suggestion labels\n\n");
    out.push_str("| Video | Rank | Label | Source | Score | Title |\n");
    out.push_str("|---|---:|---|---|---:|---|\n");
    for video in &report.videos {
        for (rank, suggestion) in video.suggestions.iter().enumerate() {
            out.push_str(&format!(
                "| `{}` | {} | {} | {} | {:.3} | {} |\n",
                video.label,
                rank.saturating_add(1),
                label_name(suggestion.label),
                suggestion.source,
                suggestion.score,
                suggestion.title.replace(['|', '\n', '\r'], " ")
            ));
        }
    }
    out.push_str("\n## Notes\n\n");
    for note in &report.notes {
        out.push_str(&format!("- {note}\n"));
    }
    for video in &report.videos {
        if !video.notes.is_empty() {
            out.push_str(&format!(
                "- `{}`: {}\n",
                video.label,
                video.notes.join("; ")
            ));
        }
    }
    out
}

pub(super) fn percent(value: Option<f64>) -> String {
    value.map_or_else(|| "n/a".into(), |value| format!("{:.3}", value))
}

pub(super) fn micros(value: u64) -> f64 {
    value as f64 / 1_000_000.0
}

pub(super) fn sanitize(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for character in value.chars() {
        if character.is_ascii_alphanumeric() || character == '-' || character == '_' {
            out.push(character);
        } else {
            out.push('-');
        }
    }
    if out.is_empty() { "set".into() } else { out }
}

pub(super) fn file_stem(path: &Path) -> String {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("label")
        .to_owned()
}

pub(super) fn file_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("source")
        .to_owned()
}

pub(super) fn millis(duration: std::time::Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

pub(super) fn git_rev(root: &Path, reference: &str) -> String {
    std::process::Command::new("git")
        .args(["rev-parse", reference])
        .current_dir(root)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "unknown".into())
}

fn cost(value: Option<u64>) -> String {
    value.map_or_else(
        || "unknown".to_owned(),
        |value| format!("USD {:.6}", micros(value)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_keeps_report_names_safe() {
        assert_eq!(sanitize("real/set 1"), "real-set-1");
        assert_eq!(sanitize(""), "set");
    }
}

//! `cargo xtask clip-eval <set>` entry point.
use clip_eval::{Options, Provider};
use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("clip-eval: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let mut args = Vec::new();
    for argument in std::env::args_os().skip(1) {
        args.push(
            argument
                .into_string()
                .map_err(|_| "arguments must be valid Unicode".to_owned())?,
        );
    }
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        println!("{USAGE}");
        return Ok(());
    }
    let mut set = None;
    let mut labels = None;
    let mut provider = Provider::Heuristic;
    let mut live = false;
    let mut max_cost_micros = 1_000_000u64;
    let mut k = 5usize;
    let mut energy = true;
    let mut transcribe = false;
    let mut out = None;
    let mut baseline = None;
    let mut write_baseline = None;
    let mut baseline_revision = None;
    let mut index = 0;
    while index < args.len() {
        let arg = args.get(index).ok_or("missing argument")?;
        match arg.as_str() {
            "--labels" => {
                index += 1;
                labels = Some(PathBuf::from(
                    args.get(index).ok_or("--labels needs a path")?,
                ));
            }
            "--provider" => {
                index += 1;
                provider = Provider::parse(args.get(index).ok_or("--provider needs a value")?)
                    .map_err(|error| error.to_string())?;
            }
            "--live" => live = true,
            "--max-cost-usd" => {
                index += 1;
                let value: f64 = args
                    .get(index)
                    .ok_or("--max-cost-usd needs a value")?
                    .parse()
                    .map_err(|_| "--max-cost-usd must be a number".to_owned())?;
                if !value.is_finite() || value < 0.0 {
                    return Err("--max-cost-usd must be finite and nonnegative".into());
                }
                max_cost_micros = (value * 1_000_000.0).round() as u64;
            }
            "--k" => {
                index += 1;
                k = args
                    .get(index)
                    .ok_or("--k needs a value")?
                    .parse()
                    .map_err(|_| "--k must be an integer".to_owned())?;
                if k == 0 || k > 32 {
                    return Err("--k must be in 1..=32".into());
                }
            }
            "--out" => {
                index += 1;
                out = Some(PathBuf::from(args.get(index).ok_or("--out needs a path")?));
            }
            "--no-energy" => energy = false,
            "--transcribe" => transcribe = true,
            "--baseline" => {
                index += 1;
                baseline = Some(PathBuf::from(
                    args.get(index).ok_or("--baseline needs a path")?,
                ));
            }
            "--write-baseline" => {
                index += 1;
                write_baseline = Some(PathBuf::from(
                    args.get(index).ok_or("--write-baseline needs a path")?,
                ));
            }
            "--baseline-revision" => {
                index += 1;
                baseline_revision = Some(
                    args.get(index)
                        .ok_or("--baseline-revision needs a value")?
                        .to_owned(),
                );
            }
            value if value.starts_with('-') => {
                return Err(format!("unknown option: {value}"));
            }
            value => {
                if set.is_some() {
                    return Err("only one set may be evaluated at a time".into());
                }
                set = Some(value.to_owned());
            }
        }
        index += 1;
    }
    let set = set.ok_or_else(|| format!("missing set\n{USAGE}"))?;
    let repo_root = std::env::current_dir().map_err(|error| error.to_string())?;
    let out_dir = out.unwrap_or_else(|| repo_root.join("target/clip-eval"));
    let options = Options {
        set,
        labels,
        provider,
        live,
        max_cost_micros,
        k,
        out_dir,
        repo_root,
        energy,
        transcribe,
        baseline,
        write_baseline,
        baseline_revision,
    };
    let output = clip_eval::run(&options).map_err(|error| error.to_string())?;
    println!(
        "videos={} provider={} hit current={:.3} baseline={:.3} precision@{} current={:.3} baseline={:.3}",
        output.report.aggregate.videos,
        output.report.provider,
        output.report.aggregate.current.hit_rate,
        output.report.aggregate.baseline.hit_rate,
        output.report.k,
        output.report.aggregate.current.precision_at_k,
        output.report.aggregate.baseline.precision_at_k
    );
    println!("json: {}", output.json_path.display());
    println!("markdown: {}", output.markdown_path.display());
    Ok(())
}

const USAGE: &str = "usage: cargo xtask clip-eval <set> [--labels DIR] [--provider heuristic|gpt-6-luna] [--live] [--transcribe] [--baseline PATH] [--write-baseline PATH] [--baseline-revision SHA] [--max-cost-usd N] [--k N] [--out DIR] [--no-energy]";

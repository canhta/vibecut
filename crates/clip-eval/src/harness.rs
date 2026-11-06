//! Runs the fixed label set and writes its scored report.
mod report;
mod scoring;

pub use report::{Aggregate, RangeReport, Report, SuggestionReport, VideoReport};
use report::{file_name, file_stem, git_rev, micros, millis, write_report};
use scoring::{RunContext, estimate_provider, run_provider, score};

use crate::baseline::{Baseline, BaselineInput};
use crate::budget::Budget;
use crate::label::{LabelFile, ResolvedSettings, sha256_bytes, sha256_file, verify_hash};
use crate::metrics::Metrics;
use crate::transcript;
use crate::{Error, Result};
use ai::clips::{
    ClipInput, ClipSegment, ClipSettings, ClipSuggester, HeuristicClipSuggester, OPENAI_LUNA_MODEL,
    OPENAI_SETTINGS, OpenAiClipSuggester,
};
use ai::transport::{ApiKey, UreqTransport};
use ai::{ExecutionHooks, Location, ProviderConfig, Stage};
use captions::{LanguageProfile, LanguageRegistry, LanguageTag, NfcText, SegmentRules, Transcript};

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Provider {
    Heuristic,
    Gpt6Luna,
}

impl Provider {
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "heuristic" => Ok(Self::Heuristic),
            "gpt-6-luna" => Ok(Self::Gpt6Luna),
            other => Err(Error::Options(format!("unknown provider: {other}"))),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Heuristic => "heuristic",
            Self::Gpt6Luna => "gpt-6-luna",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Options {
    pub set: String,
    pub labels: Option<PathBuf>,
    pub provider: Provider,
    pub live: bool,
    pub max_cost_micros: u64,
    pub k: usize,
    pub out_dir: PathBuf,
    pub repo_root: PathBuf,
    pub energy: bool,
    pub transcribe: bool,
    pub baseline: Option<PathBuf>,
    pub write_baseline: Option<PathBuf>,
    pub baseline_revision: Option<String>,
    pub auto_baseline: bool,
}

impl Options {
    pub fn labels_dir(&self) -> PathBuf {
        if let Some(path) = &self.labels {
            return path.clone();
        }
        if self.set == "synthetic" {
            return self.repo_root.join("crates/clip-eval/fixtures/synthetic");
        }
        let direct = PathBuf::from(&self.set);
        if direct.is_dir() {
            return direct;
        }
        private_root(&self.repo_root)
            .join("clip-eval")
            .join(&self.set)
    }
}

/// Prefer the checkout's `.privates`; a linked worktree falls back to the main
/// checkout's private directory through the common git directory.
fn private_root(root: &Path) -> PathBuf {
    let local = root.join(".privates");
    if local.is_dir() {
        return local;
    }
    let common = std::process::Command::new("git")
        .args(["rev-parse", "--path-format=absolute", "--git-common-dir"])
        .current_dir(root)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|value| PathBuf::from(value.trim()));
    if let Some(git_dir) = common
        && let Some(main) = git_dir.parent()
    {
        let candidate = main.join(".privates");
        if candidate.is_dir() {
            return candidate;
        }
    }
    local
}

#[derive(Debug)]
pub struct RunOutput {
    pub report: Report,
    pub json_path: PathBuf,
    pub markdown_path: PathBuf,
}

struct NoHooks;
impl ExecutionHooks for NoHooks {
    fn cancelled(&self) -> bool {
        false
    }
    fn stage(&self, _: Stage) {}
}

pub fn run(options: &Options) -> Result<RunOutput> {
    let labels_dir = options.labels_dir();
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&labels_dir)
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                Error::LabelsMissing(labels_dir.display().to_string())
            } else {
                Error::Io(error)
            }
        })?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .filter(|path| {
            !path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with(".transcript.json") || name == "baseline.json")
        })
        .collect();
    paths.sort();
    if paths.is_empty() {
        return Err(Error::EmptySet(labels_dir.display().to_string()));
    }
    let stems: Vec<String> = paths.iter().map(|path| file_stem(path)).collect();

    let baseline_path = options.baseline.clone().or_else(|| {
        if !options.auto_baseline {
            return None;
        }
        let auto = labels_dir.join("baseline.json");
        auto.exists().then_some(auto)
    });
    let baseline = baseline_path.as_deref().map(Baseline::load).transpose()?;
    if let Some(baseline) = &baseline {
        baseline.validate_labels(&stems)?;
    }

    let (provider, provider_label) = make_provider(options)?;
    let mut budget = Budget::new(options.max_cost_micros);
    let mut videos = Vec::new();
    let mut aggregate = Aggregate {
        videos: 0,
        current: Metrics::default(),
        baseline: Metrics::default(),
        main_baseline: None,
        current_cost_micros: 0,
        baseline_cost_micros: 0,
        wall_ms: 0,
    };
    let mut notes = Vec::new();
    let mut baseline_inputs = std::collections::BTreeMap::new();
    let mut baseline_videos = std::collections::BTreeMap::new();
    for (path, stem) in paths.iter().zip(&stems) {
        let started = Instant::now();
        let (label, label_dir) = LabelFile::load(path)?;
        let source = label.source_path(&label_dir, &options.out_dir.join("synthetic"), stem)?;
        if label.synthetic.is_some() && !source.exists() {
            let duration = label
                .synthetic
                .as_ref()
                .map(|synthetic| synthetic.duration_seconds)
                .ok_or_else(|| Error::Options("synthetic duration".into()))?;
            crate::label::write_synthetic_wav(&source, duration)?;
        }
        let source_sha = verify_hash(&label, &source)?;
        let language = LanguageTag::parse(&label.language)?;
        let cached = transcript::load(
            &transcript::LoadContext {
                label: &label,
                label_dir: &label_dir,
                source_sha: &source_sha,
                out_dir: &options.out_dir,
                source: &source,
                language: &language,
                transcribe: options.transcribe,
            },
            &mut budget,
        )?;
        if cached.transcript.source_lang != language {
            return Err(Error::TranscriptLanguage {
                expected: label.language.clone(),
                actual: cached.transcript.source_lang.as_str().to_owned(),
            });
        }
        let registry = LanguageRegistry::standard()?;
        let profile = registry.profile(&language);
        let settings = label.settings.resolve().map_err(Error::Options)?;
        let input = build_input(&cached.transcript, profile, &settings, &language)?;
        let fingerprints = baseline_input(
            &source_sha,
            &cached,
            &settings,
            &label,
            &language,
            options.k,
            options.energy,
        )?;
        let main_metrics = if let Some(baseline) = &baseline {
            baseline.validate_input(stem, &fingerprints)?;
            Some(baseline.metrics(stem)?.clone())
        } else {
            None
        };
        let estimate = estimate_provider(provider.as_ref(), &input, &source_sha)?;
        eprintln!(
            "{}: provider {} estimated cost USD {:.6}; remaining USD {:.6} of cap USD {:.6}",
            stem,
            provider_label,
            micros(estimate),
            micros(budget.remaining()),
            micros(budget.limit())
        );

        let mut video_notes = Vec::new();
        video_notes.extend(cached.notes.iter().cloned());
        let current_started = Instant::now();
        let context = RunContext {
            provider: provider.as_ref(),
            input: &input,
            transcript: &cached.transcript,
            profile,
            settings: &settings,
            source_sha: &source_sha,
            source: &source,
            energy: options.energy,
        };
        let current = run_provider(&context, &mut video_notes, estimate, &mut budget)?;
        let current_wall = millis(current_started.elapsed());
        let current_metrics = score(
            &label.clips,
            &current,
            &cached.transcript,
            profile,
            &settings,
            options.k,
            estimate.saturating_add(cached.cost_micros),
            current_wall,
        )?;

        let (baseline_metrics, baseline_cost) = if options.provider == Provider::Heuristic {
            let mut metrics = current_metrics.clone();
            metrics.cost_micros = 0;
            (metrics, 0)
        } else {
            let heuristic = HeuristicClipSuggester::new()?;
            let baseline_started = Instant::now();
            let baseline_context = RunContext {
                provider: &heuristic,
                ..context
            };
            let baseline_run = run_provider(&baseline_context, &mut video_notes, 0, &mut budget)?;
            let baseline_wall = millis(baseline_started.elapsed());
            let metrics = score(
                &label.clips,
                &baseline_run,
                &cached.transcript,
                profile,
                &settings,
                options.k,
                0,
                baseline_wall,
            )?;
            (metrics, 0)
        };

        if options.write_baseline.is_some() {
            baseline_inputs.insert(stem.clone(), fingerprints);
            baseline_videos.insert(stem.clone(), baseline_metrics.clone());
        }

        aggregate.videos = aggregate.videos.saturating_add(1);
        aggregate.current.merge(&current_metrics);
        aggregate.baseline.merge(&baseline_metrics);
        aggregate.current_cost_micros = aggregate
            .current_cost_micros
            .saturating_add(current_metrics.cost_micros);
        aggregate.baseline_cost_micros =
            aggregate.baseline_cost_micros.saturating_add(baseline_cost);
        aggregate.wall_ms = aggregate.wall_ms.saturating_add(millis(started.elapsed()));

        videos.push(VideoReport {
            label: stem.clone(),
            source: file_name(&source),
            sha256: source_sha,
            language: label.language.clone(),
            transcript: cached.source.to_owned(),
            transcript_cache_hit: cached.cache_hit,
            current: current_metrics,
            baseline: baseline_metrics,
            main_baseline: main_metrics,
            suggestions: current.report,
            notes: video_notes,
        });
    }

    if baseline.is_some() {
        let mut main = Metrics::default();
        for video in &videos {
            if let Some(metrics) = &video.main_baseline {
                main.merge(metrics);
            }
        }
        aggregate.main_baseline = Some(main);
    }

    let revision = git_rev(&options.repo_root, "HEAD");
    if let Some(path) = &options.write_baseline {
        let baseline_revision = options
            .baseline_revision
            .clone()
            .unwrap_or_else(|| revision.clone());
        let written = Baseline {
            schema: 2,
            revision: baseline_revision,
            provider: "heuristic".into(),
            note: "Pinned 01 heuristic + 02 snapping baseline; regenerate with --write-baseline when pipeline logic changes.".into(),
            inputs: baseline_inputs,
            videos: baseline_videos,
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, serde_json::to_vec_pretty(&written)?)?;
        notes.push(format!(
            "wrote pinned heuristic baseline to {}",
            path.display()
        ));
    }

    notes.push(format!(
        "heuristic baseline is the 01 mode at evaluated revision {revision}"
    ));
    match &baseline {
        Some(baseline) => notes.push(format!(
            "main baseline loaded from {} at revision {}",
            baseline_path
                .as_deref()
                .map_or_else(|| "baseline.json".into(), |path| path.display().to_string()),
            baseline.revision
        )),
        None => notes.push(
            "main baseline not measured; pass --baseline PATH or place baseline.json next to the labels"
                .into(),
        ),
    }
    notes.push(
        "cost is the provider's per-invocation upper-bound estimate, not measured billing".into(),
    );
    let report = Report {
        schema: 1,
        set: options.set.clone(),
        provider: provider_label,
        live: options.live,
        k: options.k,
        revision: revision.clone(),
        baseline: "source:heuristic".into(),
        baseline_revision: revision,
        main_baseline_revision: baseline.as_ref().map(|baseline| baseline.revision.clone()),
        max_cost_micros: options.max_cost_micros,
        aggregate,
        videos,
        notes,
    };
    let (json_path, markdown_path) = write_report(&report, &options.out_dir, &options.set)?;
    Ok(RunOutput {
        report,
        json_path,
        markdown_path,
    })
}

fn make_provider(options: &Options) -> Result<(Box<dyn ClipSuggester>, String)> {
    match options.provider {
        Provider::Heuristic => Ok((Box::new(HeuristicClipSuggester::new()?), "heuristic".into())),
        Provider::Gpt6Luna => {
            if !options.live {
                return Err(Error::LiveRequired);
            }
            let key =
                std::env::var("OPENAI_API_KEY").map_err(|_| Error::MissingKey("OPENAI_API_KEY"))?;
            let key = ApiKey::new(key).map_err(|_| Error::MissingKey("OPENAI_API_KEY"))?;
            let config = ProviderConfig::new(
                "clips".into(),
                OPENAI_LUNA_MODEL.into(),
                OPENAI_SETTINGS.into(),
                Location::Remote {
                    backend: "openai".into(),
                    base_url: ai::openai::BASE_URL.into(),
                    auth_ref: "env:OPENAI_API_KEY".into(),
                },
            )?;
            let provider = OpenAiClipSuggester::new(config, key, Arc::new(UreqTransport::new()))?;
            Ok((Box::new(provider), "gpt-6-luna".into()))
        }
    }
}

fn build_input(
    transcript: &Transcript,
    profile: &LanguageProfile,
    settings: &ResolvedSettings,
    language: &LanguageTag,
) -> Result<ClipInput> {
    let rules = SegmentRules::from_profile(profile, settings.frame_rate)?;
    let segmentation = captions::segment_transcript(
        transcript,
        profile,
        &rules,
        &captions::LayoutHooks::default(),
    )?;
    let mut segments = Vec::with_capacity(segmentation.cues.len());
    for cue in &segmentation.cues {
        let text = cue
            .cue
            .texts
            .get(language)
            .map(|text| text.text.clone())
            .ok_or_else(|| Error::Options("cue has no source-language text".into()))?;
        segments.push(ClipSegment {
            id: cue.cue.id.clone(),
            text,
            range: cue.cue.range,
        });
    }
    let input = ClipInput {
        segments,
        settings: ClipSettings {
            count: settings.count,
            min_duration: settings.min_duration,
            max_duration: settings.max_duration,
            max_segments: settings.max_segments,
            min_segment_duration: settings.min_segment_duration,
            overlap_tolerance: settings.overlap_tolerance,
            language: language.clone(),
            keywords: settings
                .keywords
                .iter()
                .map(|keyword| NfcText::from(keyword.as_str()))
                .collect(),
        },
    };
    input.validate()?;
    Ok(input)
}

fn baseline_input(
    source_sha: &str,
    cached: &transcript::CachedTranscript,
    settings: &ResolvedSettings,
    label: &LabelFile,
    language: &LanguageTag,
    k: usize,
    energy: bool,
) -> Result<BaselineInput> {
    Ok(BaselineInput {
        source_sha256: source_sha.to_owned(),
        transcript_sha256: sha256_file(&cached.cache_path)?,
        settings_sha256: sha256_bytes(&serde_json::to_vec(settings)?),
        labels_sha256: sha256_bytes(&serde_json::to_vec(&label.clips)?),
        language: language.as_str().to_owned(),
        k,
        energy,
    })
}

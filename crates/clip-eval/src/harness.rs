//! Evaluation runner: labels in, ranked shorts out, metrics and report written.
use crate::baseline::{Baseline, BaselineInput};
use crate::budget::Budget;
use crate::label::{LabelFile, ResolvedSettings, sha256_bytes, sha256_file, verify_hash};
use crate::metrics::{Evaluation, Metrics, evaluate};
use crate::transcript;
use crate::{Error, Result};
use ai::clips::{
    ClipInput, ClipSegment, ClipSettings, ClipSource, ClipSuggester, HeuristicClipSuggester,
    MAX_WIRE, MODE, OPENAI_LUNA_MODEL, OPENAI_SETTINGS, OpenAiClipSuggester, suggest_clips,
};
use ai::transport::{ApiKey, UreqTransport};
use ai::{
    Consent, CostEstimate, ExecutionHooks, Location, Money, Operation, ProviderConfig, Request,
    SampleRange, Scope, Stage,
};
use captions::{LanguageProfile, LanguageRegistry, LanguageTag, NfcText, SegmentRules, Transcript};
use engine::clip_boundaries::{ClipBoundarySettings, snap_clip_boundaries};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;
use time::{SourceRange, Tick};

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

#[derive(Clone, Debug, Serialize)]
pub struct SuggestionReport {
    pub score: f64,
    pub source: String,
    pub title: String,
    pub segments: Vec<RangeReport>,
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
    pub current_cost_micros: u64,
    pub baseline_cost_micros: u64,
    pub wall_ms: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Report {
    pub schema: u32,
    pub set: String,
    pub provider: String,
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
        let current_metrics = evaluate(&Evaluation {
            labels: &label.clips,
            suggestions: &current.shorts,
            transcript: &cached.transcript,
            profile,
            settings: &settings,
            k: options.k,
            cost_micros: estimate.saturating_add(cached.cost_micros),
            wall_ms: current_wall,
            snap_failures: current.snap_failures,
        })?;

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
            let metrics = evaluate(&Evaluation {
                labels: &label.clips,
                suggestions: &baseline_run.shorts,
                transcript: &cached.transcript,
                profile,
                settings: &settings,
                k: options.k,
                cost_micros: 0,
                wall_ms: baseline_wall,
                snap_failures: baseline_run.snap_failures,
            })?;
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

struct ProviderRun {
    shorts: Vec<Vec<ai::clips::ClipSourceSegment>>,
    report: Vec<SuggestionReport>,
    snap_failures: usize,
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

fn scope_for(provider: &dyn ClipSuggester, window: &ClipInput, source_sha: &str) -> Result<Scope> {
    Ok(Scope {
        provider: provider.config().clone(),
        operation: Operation::SuggestClips,
        payload_fingerprint: format!("clip-eval:{source_sha}").into(),
        mode: MODE.into(),
        texts: vec![window.consent_text()?],
        source_language: window.settings.language.clone(),
        target_language: None,
        source_samples: SampleRange::new(0, 1_000)?,
        sample_rate: 1,
        max_bytes: MAX_WIRE,
    })
}

fn estimate_provider(
    provider: &dyn ClipSuggester,
    input: &ClipInput,
    source_sha: &str,
) -> Result<u64> {
    let windows = input.windows()?;
    let mut total = 0u64;
    for window in &windows {
        let scope = scope_for(provider, window, source_sha)?;
        total = total.saturating_add(estimate_micros(&provider.estimate(&scope))?);
    }
    Ok(total)
}

fn estimate_micros(estimate: &CostEstimate) -> Result<u64> {
    match estimate {
        CostEstimate::Free => Ok(0),
        CostEstimate::UpperBound(money) => Ok(money.micros()),
        CostEstimate::Unknown => Err(Error::UnknownCost),
    }
}

#[derive(Clone, Copy)]
struct RunContext<'a> {
    provider: &'a dyn ClipSuggester,
    input: &'a ClipInput,
    transcript: &'a Transcript,
    profile: &'a LanguageProfile,
    settings: &'a ResolvedSettings,
    source_sha: &'a str,
    source: &'a Path,
    energy: bool,
}

fn run_provider(
    context: &RunContext<'_>,
    notes: &mut Vec<String>,
    estimate: u64,
    budget: &mut Budget,
) -> Result<ProviderRun> {
    let RunContext {
        provider,
        input,
        transcript,
        profile,
        settings,
        source_sha,
        source,
        energy: energy_enabled,
    } = *context;
    budget.reserve(estimate)?;
    let windows = input.windows()?;
    let mut scopes = Vec::with_capacity(windows.len());
    for window in &windows {
        scopes.push(scope_for(provider, window, source_sha)?);
    }
    let mut consents = Vec::with_capacity(scopes.len());
    for scope in &scopes {
        if provider.config().is_remote() {
            let estimate = provider.estimate(scope);
            let micros = estimate_micros(&estimate)?;
            consents.push(Some(Consent::grant(
                scope.clone(),
                Money::new("USD", micros)?,
            )?));
        } else {
            consents.push(None);
        }
    }
    let requests: Vec<_> = scopes
        .iter()
        .zip(&consents)
        .map(|(scope, consent)| Request {
            scope,
            consent: consent.as_ref(),
        })
        .collect();
    let result = suggest_clips(provider, input, &requests, &NoHooks)?;
    let mut report = Vec::with_capacity(result.clips.len());
    for clip in &result.clips {
        report.push(SuggestionReport {
            score: clip.proposal.score,
            source: match clip.source {
                ClipSource::Llm => "llm".into(),
                ClipSource::Heuristic => "heuristic".into(),
            },
            title: clip.proposal.title.as_str().to_owned(),
            segments: clip
                .segments
                .iter()
                .map(|segment| RangeReport {
                    start: segment.range.start().0,
                    end: segment.range.end().0,
                })
                .collect(),
        });
    }

    let energy = if energy_enabled {
        match energy_windows(source, transcript) {
            Ok(energy) => energy,
            Err(error) => {
                notes.push(format!("energy unavailable: {error}"));
                Vec::new()
            }
        }
    } else {
        Vec::new()
    };
    let source_range = source_range(source, transcript, energy_enabled)?;
    let boundary = ClipBoundarySettings {
        source_range,
        frame_rate: settings.frame_rate,
        tolerance: settings.snap_tolerance,
        silence_rms: settings.silence_rms,
        min_duration: settings.min_duration,
        max_duration: settings.max_duration,
        min_segment_duration: settings.min_segment_duration,
        max_segments: settings.max_segments,
    };
    let hooks = captions::LayoutHooks::default();
    let mut shorts = Vec::with_capacity(result.clips.len());
    let mut snap_failures = 0usize;
    for clip in &result.clips {
        match snap_clip_boundaries(
            std::slice::from_ref(&clip.segments),
            transcript,
            profile,
            &hooks,
            &energy,
            &boundary,
        ) {
            Ok(mut snapped) => match snapped.pop() {
                Some(short) => shorts.push(short.segments),
                None => {
                    snap_failures = snap_failures.saturating_add(1);
                    shorts.push(clip.segments.clone());
                }
            },
            Err(error) => {
                snap_failures = snap_failures.saturating_add(1);
                notes.push(format!("snap refused: {error}"));
                shorts.push(clip.segments.clone());
            }
        }
    }
    Ok(ProviderRun {
        shorts,
        report,
        snap_failures,
    })
}

fn source_range(source: &Path, transcript: &Transcript, probe: bool) -> Result<SourceRange> {
    let end = transcript
        .segments
        .iter()
        .map(|segment| segment.range.end())
        .max()
        .unwrap_or(Tick::ZERO);
    let probed = if probe {
        media::probe(source)
            .ok()
            .and_then(|probe| probe.duration)
            .filter(|duration| *duration > end)
            .unwrap_or(end)
    } else {
        end
    };
    if probed <= Tick::ZERO {
        return Err(Error::Options("source duration is empty".into()));
    }
    Ok(SourceRange::new(Tick::ZERO, probed)?)
}

fn energy_windows(
    source: &Path,
    transcript: &Transcript,
) -> Result<Vec<media::SpeechEnergyWindow>> {
    let asset = media::SourceAsset::import(media::AssetId(1), source.to_path_buf())?;
    let range = source_range(source, transcript, true)?;
    Ok(media::analyze_speech_energy(
        &asset,
        range,
        Arc::new(std::sync::atomic::AtomicBool::new(false)),
    )?)
}

fn write_report(report: &Report, out_dir: &Path, set: &str) -> Result<(PathBuf, PathBuf)> {
    std::fs::create_dir_all(out_dir)?;
    let stem = format!("{}-{}", sanitize(set), report.provider);
    let json_path = out_dir.join(format!("{stem}.json"));
    let markdown_path = out_dir.join(format!("{stem}.md"));
    std::fs::write(&json_path, serde_json::to_vec_pretty(report)?)?;
    std::fs::write(&markdown_path, markdown(report))?;
    Ok((json_path, markdown_path))
}

fn markdown(report: &Report) -> String {
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
        "- Cost estimate: current USD {:.6} vs baseline USD {:.6}; wall {} ms.\n",
        micros(report.aggregate.current_cost_micros),
        micros(report.aggregate.baseline_cost_micros),
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
        "| Cost (USD, upper bound) | {:.6} | {:.6} | {} |\n",
        micros(report.aggregate.current_cost_micros),
        micros(report.aggregate.baseline_cost_micros),
        main.map_or_else(
            || "n/a".into(),
            |metrics| format!("{:.6}", micros(metrics.cost_micros))
        )
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
            "| `{}` | {:.3} ({}/{}) | {:.3} ({}/{}) | {} | {:.3} | {:.3} | {} | {} | {} | {} | {:.3} | {:.3} | {} | {} | {:.6} | {} |\n",
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
            micros(video.current.cost_micros),
            video.current.wall_ms
        ));
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

fn percent(value: Option<f64>) -> String {
    value.map_or_else(|| "n/a".into(), |value| format!("{:.3}", value))
}

fn micros(value: u64) -> f64 {
    value as f64 / 1_000_000.0
}

fn sanitize(value: &str) -> String {
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

fn file_stem(path: &Path) -> String {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("label")
        .to_owned()
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("source")
        .to_owned()
}

fn millis(duration: std::time::Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

fn git_rev(root: &Path, reference: &str) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synthetic_wav_hash_is_stable_and_verified() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("source.wav");
        crate::label::write_synthetic_wav(&path, 2.0).unwrap();
        let first = crate::label::sha256_file(&path).unwrap();
        crate::label::write_synthetic_wav(&path, 2.0).unwrap();
        assert_eq!(first, crate::label::sha256_file(&path).unwrap());
    }

    struct FakeProvider {
        config: ProviderConfig,
        estimate: u64,
    }

    impl ClipSuggester for FakeProvider {
        fn config(&self) -> &ProviderConfig {
            &self.config
        }
        fn capabilities(&self, _: &Scope) -> ai::Capabilities {
            ai::Capabilities {
                model: "fake-model".into(),
                mode: MODE.into(),
                operation: Operation::SuggestClips,
                languages: vec![LanguageTag::parse("en").unwrap()],
                target_languages: Vec::new(),
                timing: std::collections::BTreeSet::new(),
                diarization: ai::Support::Unsupported,
                live_audio_input: ai::Support::Unsupported,
                file_result_stream: ai::Support::Unsupported,
                max_upload_bytes: Some(MAX_WIRE),
                max_duration_micros: None,
                evidence: ai::Evidence::FixtureVerified,
                revision: "test".into(),
            }
        }
        fn estimate(&self, _: &Scope) -> CostEstimate {
            CostEstimate::UpperBound(Money::new("USD", self.estimate).unwrap())
        }
        fn source(&self) -> ClipSource {
            ClipSource::Llm
        }
        fn suggest(
            &self,
            _: &ClipInput,
            _: &Request<'_>,
            _: &dyn ExecutionHooks,
        ) -> std::result::Result<Vec<ai::clips::ClipProposal>, ai::Error> {
            Ok(Vec::new())
        }
    }

    #[test]
    fn second_video_is_refused_by_the_cumulative_run_budget() {
        use captions::{Fingerprint, NfcText, Segment, Word};
        let transcript = Transcript {
            source_lang: LanguageTag::parse("en").unwrap(),
            fingerprint: Fingerprint {
                provider: "fixture".into(),
                model: "fixture".into(),
                settings: "fixture".into(),
            },
            segments: vec![Segment {
                id: "s0".into(),
                range: SourceRange::new(Tick::ZERO, Tick(4 * time::TICKS_PER_SECOND)).unwrap(),
                text: NfcText::from("One."),
                words: vec![Word {
                    text: NfcText::from("One."),
                    range: Some(
                        SourceRange::new(Tick::ZERO, Tick(4 * time::TICKS_PER_SECOND)).unwrap(),
                    ),
                    speaker: None,
                    confidence: None,
                }],
                spans: None,
            }],
        };
        let settings = crate::label::Settings {
            count: 1,
            min_duration_seconds: 1.0,
            max_duration_seconds: 10.0,
            min_segment_duration_seconds: 1.0,
            ..crate::label::Settings::default()
        }
        .resolve()
        .unwrap();
        let registry = LanguageRegistry::standard().unwrap();
        let language = LanguageTag::parse("en").unwrap();
        let profile = registry.profile(&language);
        let input = build_input(&transcript, profile, &settings, &language).unwrap();
        let provider = FakeProvider {
            config: ProviderConfig::new(
                "fake".into(),
                "fake-model".into(),
                "fake-settings".into(),
                Location::Remote {
                    backend: "fake".into(),
                    base_url: "https://example.invalid".into(),
                    auth_ref: "test".into(),
                },
            )
            .unwrap(),
            estimate: 344_064,
        };
        let estimate = estimate_provider(&provider, &input, "sha").unwrap();
        assert_eq!(estimate, 344_064);
        let context = RunContext {
            provider: &provider,
            input: &input,
            transcript: &transcript,
            profile,
            settings: &settings,
            source_sha: "sha",
            source: Path::new("unused"),
            energy: false,
        };
        let mut budget = Budget::new(350_000);
        let mut notes = Vec::new();
        run_provider(&context, &mut notes, estimate, &mut budget).unwrap();
        assert!(matches!(
            run_provider(&context, &mut notes, estimate, &mut budget),
            Err(Error::CostCap {
                estimate: 688_128,
                cap: 350_000
            })
        ));
    }

    #[test]
    fn sanitize_keeps_report_names_safe() {
        assert_eq!(sanitize("real/set 1"), "real-set-1");
        assert_eq!(sanitize(""), "set");
    }
}

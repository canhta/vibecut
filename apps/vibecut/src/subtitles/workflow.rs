//! Composition: captions/engine own recognition, mapping, interchange and translation.
use super::{TaskEvents, Work, Workflow};
use captions::{CueTrack, Format};
use engine::{
    ByteBudget, Command, EditError, NewItem, Session,
    cache::ArtifactCache,
    jobs::{JobRunner, RunnerConfig, SystemClock},
    transcript::{Cancellation, Progress, Stage},
};
use std::{io::Read, sync::Arc, time::Duration};
use time::{SourceRange, Tick};

pub struct LocalWorkflow;
impl Workflow for LocalWorkflow {
    fn run(
        &self,
        work: Work,
        budget: ByteBudget,
        events: &TaskEvents,
        cancel: &Cancellation,
    ) -> engine::Result<Command> {
        if cancel.cancelled() {
            return Err(EditError::Cancelled);
        }
        match work {
            Work::Import {
                asset_id,
                path,
                language,
                start,
            } => {
                events.stage(Stage::Assemble);
                let format = match path
                    .extension()
                    .and_then(|s| s.to_str())
                    .map(str::to_ascii_lowercase)
                    .as_deref()
                {
                    Some("srt") => Format::Srt,
                    Some("vtt") => Format::WebVtt,
                    _ => return Err(EditError::Unsupported("import SRT or VTT")),
                };
                let mut bytes = Vec::new();
                std::fs::File::open(&path)?
                    .take(16 * 1024 * 1024 + 1)
                    .read_to_end(&mut bytes)?;
                let imported = captions::parse(&bytes, format, language)?;
                let end = imported
                    .track
                    .cues
                    .iter()
                    .map(|c| c.range.end())
                    .max()
                    .ok_or(EditError::Invalid("no imported cues"))?;
                let asset = media::SourceAsset::try_from(media::AssetData {
                    ephemeral: true,
                    id: asset_id,
                    path: std::fs::canonicalize(path)?,
                    content_hash: media::ContentHash::from_bytes(&bytes),
                    probe: media::SourceProbe {
                        format: match format {
                            Format::Srt => "srt",
                            Format::WebVtt => "webvtt",
                            Format::Txt => "txt",
                        }
                        .into(),
                        duration: Some(end),
                        streams: Vec::new(),
                    },
                })?;
                Ok(Command::Batch {
                    commands: vec![
                        Command::AddAsset { asset },
                        insert(
                            imported.track,
                            Some(asset_id),
                            SourceRange::new(Tick::ZERO, end)?,
                            start,
                        )?,
                    ],
                })
            }
            Work::Generate {
                asset,
                range,
                start,
                options,
            } => {
                events.stage(Stage::Extract);
                let credentials = engine::providers::Credentials::from_env();
                let root = engine::cache::CacheRoot::discover()?.models();
                let (mut cache, _) = ArtifactCache::open(&root, budget.clone(), 128)?;
                let provider = engine::providers::choose_transcription(
                    &options.transcription,
                    Some(&options.transcription_model),
                )?;
                let runtime = engine::providers::Runtime {
                    gipformer_runner: engine::providers::local_asr_runner(),
                    ..Default::default()
                };
                let providers = engine::providers::Providers::open(
                    provider,
                    None,
                    &mut cache,
                    &credentials,
                    &runtime,
                )?;
                let duration = asset
                    .probe()
                    .duration
                    .ok_or(EditError::Invalid("source duration unavailable"))?;
                // Auto-detect first; the detected tag then goes to every ASR
                // request explicitly, exactly like a user-picked language.
                let language = if options.source == "auto" {
                    let detected = providers.detect_language(&asset, duration, cancel)?;
                    if !super::SOURCE_LANGUAGES.contains(&detected.as_str()) {
                        return Err(engine::transcript::Error::LowConfidence.into());
                    }
                    detected
                } else {
                    captions::LanguageTag::parse(&options.source)?
                };
                let output = engine::transcript::transcribe(
                    &asset,
                    duration,
                    &language,
                    &providers,
                    Some(range),
                    &std::sync::Mutex::new(&mut cache),
                    events,
                    events,
                    cancel,
                    cancel,
                )?;
                let cues = captions::clip_track(
                    &CueTrack {
                        styles: Vec::new(),
                        cues: output.cues,
                    },
                    range,
                )?;
                if cancel.cancelled() {
                    return Err(EditError::Cancelled);
                }
                insert(cues, Some(asset.id()), range, start)
            }
            Work::Translate {
                project,
                item,
                ids,
                options,
                glossary,
                force,
            } => {
                let adapter = engine::transcript::SubtitleTranslator::open(
                    &engine::providers::choose_translation(
                        &options.translation,
                        Some(&options.translation_model),
                    )?,
                    &engine::providers::Credentials::from_env(),
                )?;
                let mut staging = Session::new((*project).clone(), 1)?;
                let mut runner = runner(budget)?;
                engine::transcript::translate_selected(
                    &mut staging,
                    &adapter,
                    &mut runner,
                    events,
                    events,
                    cancel,
                    &engine::transcript::TranslationSelection {
                        item,
                        ids,
                        target: captions::LanguageTag::parse(&options.target)?,
                        glossary,
                        force,
                    },
                    &mut engine::providers::Cost::Free,
                )?;
                let content = staging
                    .project()
                    .data()
                    .scene()
                    .timeline
                    .iter()
                    .flat_map(|t| &t.items)
                    .find(|i| i.id == item)
                    .ok_or(EditError::NoItem(item))?
                    .content
                    .clone();
                if cancel.cancelled() {
                    return Err(EditError::Cancelled);
                }
                Ok(Command::SetItemParam { item, content })
            }
        }
    }
}
fn runner(budget: ByteBudget) -> engine::Result<JobRunner> {
    JobRunner::new(
        RunnerConfig {
            concurrency: [2, 1, 1],
            queue: 8,
            retained: 16,
            network_attempts: 1,
            backoff: Duration::from_millis(10),
            progress_interval: Duration::from_millis(20),
        },
        budget,
        Arc::new(SystemClock::default()),
    )
}
fn insert(
    cues: CueTrack,
    asset: Option<media::AssetId>,
    range: SourceRange,
    start: Tick,
) -> engine::Result<Command> {
    if cues.cues.is_empty() || cues.cues.len() > 10_000 {
        return Err(EditError::Limit);
    }
    Ok(Command::InsertItem {
        placement: engine::Placement::Auto,
        item: NewItem {
            retime: Default::default(),
            animations: Default::default(),
            source_asset: asset,
            source_range: range,
            start,
            content: project::ItemParams::Caption(project::CaptionParams { cues }),
        },
    })
}

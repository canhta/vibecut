//! Composition: captions/engine own recognition, mapping, interchange and translation.
use super::{TaskEvents, Work, Workflow};
use captions::{CueTrack, Format};
use engine::{
    ByteBudget, Command, EditError, NewItem, Session,
    cache::ArtifactCache,
    jobs::{JobRunner, RunnerConfig, SystemClock},
    subtitles::{self, Cancellation, Progress, Stage},
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
                let credentials = credentials()?;
                let root = subtitles::default_cache_root()?;
                let (mut cache, _) = ArtifactCache::open(&root, budget.clone(), 128)?;
                let provider = subtitles::choose_transcription(
                    &options.transcription,
                    Some(&options.transcription_model),
                )?;
                let runtime = subtitles::Runtime {
                    gipformer_runner: subtitles::local_asr_runner(),
                    ..Default::default()
                };
                let providers =
                    subtitles::Providers::open(provider, None, &mut cache, &credentials, &runtime)?;
                let mut runner = runner(budget)?;
                let duration = asset
                    .probe()
                    .duration
                    .ok_or(EditError::Invalid("source duration unavailable"))?;
                let output = subtitles::transcribe(
                    &asset,
                    duration,
                    &captions::LanguageTag::parse(&options.source)?,
                    &providers,
                    &mut runner,
                    &mut cache,
                    events,
                    events,
                    cancel,
                )?;
                let mapping = captions::EditMapping::new(vec![(range, range.start())])?;
                let mut cues = mapping.map_cues(&CueTrack { cues: output.cues })?;
                for cue in &mut cues.cues {
                    cue.id = captions::CueFragmentId::from_id(&cue.id)?.source_id;
                }
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
                let adapter = subtitles::SubtitleTranslator::open(
                    &subtitles::choose_translation(
                        &options.translation,
                        Some(&options.translation_model),
                    )?,
                    &credentials()?,
                )?;
                let mut staging = Session::new((*project).clone(), 1)?;
                let mut runner = runner(budget)?;
                subtitles::translate_selected(
                    &mut staging,
                    &adapter,
                    &mut runner,
                    events,
                    events,
                    cancel,
                    &subtitles::TranslationSelection {
                        item,
                        ids,
                        target: captions::LanguageTag::parse(&options.target)?,
                        glossary,
                        force,
                    },
                    &mut subtitles::Cost::Free,
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
fn credentials() -> engine::Result<subtitles::Credentials> {
    subtitles::Credentials::new(
        std::env::var("OPENAI_API_KEY").ok(),
        std::env::var("ELEVENLABS_API_KEY").ok(),
        std::env::var("ANTHROPIC_API_KEY").ok(),
    )
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
            animations: Default::default(),
            source_asset: asset,
            source_range: range,
            start,
            content: project::ItemParams::Caption(project::CaptionParams { cues }),
        },
    })
}

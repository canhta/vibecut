//! SRT/VTT import: a local `Command` constructor, no provider and no registry job.
use captions::{CueTrack, Format};
use engine::{Command, NewItem, control::Error as EngineError, transcript::Cancellation};
use std::{io::Read, path::PathBuf};
use time::{SourceRange, Tick};

/// Parse the file into one batch: the ephemeral subtitle asset and its caption item.
pub(super) fn command(
    asset_id: media::AssetId,
    path: PathBuf,
    language: captions::LanguageTag,
    start: Tick,
    cancel: &Cancellation,
) -> engine::control::Result<Command> {
    if cancel.cancelled() {
        return Err(EngineError::Runtime(engine::RuntimeError::Cancelled));
    }
    let format = match path
        .extension()
        .and_then(|s| s.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("srt") => Format::Srt,
        Some("vtt") => Format::WebVtt,
        _ => {
            return Err(EngineError::Edit(engine::EditError::Unsupported(
                "import SRT or VTT",
            )));
        }
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
        .ok_or(EngineError::Edit(engine::EditError::Invalid(
            "no imported cues",
        )))?;
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
fn insert(
    cues: CueTrack,
    asset: Option<media::AssetId>,
    range: SourceRange,
    start: Tick,
) -> engine::control::Result<Command> {
    if cues.cues.is_empty() || cues.cues.len() > 10_000 {
        return Err(EngineError::Control(engine::control::ControlError::Limit));
    }
    Ok(Command::InsertItem {
        placement: engine::Placement::Auto,
        item: NewItem {
            retime: Default::default(),
            animations: Default::default(),
            source_asset: asset,
            source_range: range,
            start,
            content: project::ItemParams::Caption(project::CaptionParams {
                word_highlight: false,
                preset: None,
                overrides: captions::CaptionOverrides::default(),
                cues,
            }),
        },
    })
}

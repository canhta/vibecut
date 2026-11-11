//! Publishes subtitle workflow output.
use super::{Error as SubtitleError, Result as SubtitleResult};
use crate::cache::{ArtifactKind, CacheKey, KeyInputs};
use crate::jobs::JobRunner;
use crate::transcript::{Cancellation, Progress, wait_for_job};
use crate::{Command, Session};
use captions::LanguageTag;
use media::SourceAsset;
use project::{ItemParams, TrackId};
use std::path::{Path, PathBuf};
use time::{Tick, TimeRange};
pub(super) fn caption_track_id(session: &Session) -> crate::session::Result<TrackId> {
    session
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .find(|track| {
            track
                .items
                .iter()
                .any(|item| matches!(item.content, ItemParams::Caption(_)))
        })
        .map(|track| track.id)
        .ok_or(crate::session::EditError::Invalid("missing caption track"))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn export_subtitles(
    session: &Session,
    runner: &mut JobRunner,
    progress: &dyn Progress,
    asset: &SourceAsset,
    track: TrackId,
    languages: &[LanguageTag],
    path: &Path,
    overwrite: bool,
    cancel: &Cancellation,
) -> SubtitleResult<()> {
    cancel.checkpoint()?;
    let key = export_key(asset, "srt")?;
    let (id, token) = session.submit_export_command(
        runner,
        key,
        Command::ExportSubtitles {
            track,
            format: captions::Format::Srt,
            languages: languages.to_vec(),
            path: path.to_path_buf(),
            overwrite,
        },
    )?;
    let _ = wait_for_job(runner, id, &token, progress, cancel)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn burn_subtitles(
    session: &Session,
    runner: &mut JobRunner,
    progress: &dyn Progress,
    asset: &SourceAsset,
    duration: Tick,
    languages: &[LanguageTag],
    path: &Path,
    portrait: Option<bool>,
    overwrite: bool,
    cancel: &Cancellation,
) -> SubtitleResult<()> {
    cancel.checkpoint()?;
    let key = export_key(asset, "burn")?;
    let range = TimeRange::new(Tick::ZERO, duration)?;
    let (id, token) = session.submit_export_command(
        runner,
        key,
        Command::BurnSubtitles {
            range,
            languages: languages.to_vec(),
            style: render::CaptionStyle::default(),
            portrait: portrait.unwrap_or(false),
            path: path.to_path_buf(),
            overwrite,
        },
    )?;
    let _ = wait_for_job(runner, id, &token, progress, cancel)?;
    Ok(())
}

fn export_key(asset: &SourceAsset, label: &str) -> crate::runtime::Result<CacheKey> {
    let settings = format!("subtitle-cli-v1:{label}");
    CacheKey::new(KeyInputs {
        kind: ArtifactKind::Preview,
        source: asset.content_hash().0,
        provider: "vibecut-cli",
        model: "subtitle",
        version: "1",
        settings: settings.as_bytes(),
        transcript: [0; 32],
        cues: [0; 32],
        clip_range: [0; 32],
        translation: [0; 32],
    })
}

pub(super) fn ensure_distinct(input: &Path, output: &Path) -> SubtitleResult<()> {
    if resolve_path(input)? == resolve_path(output)? {
        return Err(SubtitleError::Edit(crate::session::EditError::Invalid(
            "output must not replace the input media",
        )));
    }
    Ok(())
}
pub(super) fn resolve_path(path: &Path) -> SubtitleResult<PathBuf> {
    match std::fs::canonicalize(path) {
        Ok(value) => Ok(value),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let name =
                path.file_name()
                    .ok_or(SubtitleError::Edit(crate::session::EditError::Invalid(
                        "output path has no file name",
                    )))?;
            let parent = path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            Ok(std::fs::canonicalize(parent)?.join(name))
        }
        Err(error) => Err(error.into()),
    }
}

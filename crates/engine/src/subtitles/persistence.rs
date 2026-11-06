//! Publishes subtitle workflow output.
use super::{
    Animations, ArtifactKind, CacheKey, Cancellation, CaptionParams, Command, Cue, CueTrack,
    EditError, ItemParams, JobRunner, KeyInputs, LanguageTag, NewItem, Outcome, Path, PathBuf,
    Progress, Project, ProjectId, Session, SourceAsset, SourceRange, Tick, TimeRange, TrackId,
    TrackKind, wait_for_job,
};
pub(crate) fn build_session(
    asset: &SourceAsset,
    cues: &[Cue],
    duration: Tick,
    with_video: bool,
    portrait: Option<bool>,
) -> Result<Session, EditError> {
    let mut data = Project::empty(ProjectId(1), "Subtitles".into())?.into_data();
    if with_video
        && let Some(video) = asset
            .probe()
            .streams
            .iter()
            .find(|stream| stream.kind == media::StreamKind::Video)
    {
        if let Some(info) = &video.video {
            data.canvas.width = info.width;
            data.canvas.height = info.height;
        }
        if let Some(rate) = video.frame_rate {
            data.canvas.fps = rate;
        }
    }
    if with_video && portrait == Some(false) {
        data.canvas.width = 1920;
        data.canvas.height = 1080;
    }
    let mut session = Session::new(Project::new(data)?, 16)?;
    session.apply(Command::AddAsset {
        asset: asset.clone(),
    })?;
    if with_video {
        let video = session.project().data().scene().timeline.main.id;
        let range = SourceRange::new(Tick::ZERO, duration)?;
        session.apply(Command::InsertItem {
            placement: crate::Placement::Track { track: video },
            item: NewItem {
                retime: Default::default(),
                animations: Animations::default(),
                source_asset: Some(asset.id()),
                source_range: range,
                start: Tick::ZERO,
                content: ItemParams::Video(project::VideoParams {
                    fades: Default::default(),
                    source_audio_enabled: true,
                    volume: 0.0,
                    muted: false,
                }),
            },
        })?;
    }
    let captions = track_id(session.apply(Command::AddTrack {
        placement: crate::TrackPlacement::Auto,
        name: "Captions".into(),
        kind: TrackKind::Text,
    })?)?;
    let range = SourceRange::new(Tick::ZERO, duration)?;
    session.apply(Command::InsertItem {
        placement: crate::Placement::Track { track: captions },
        item: NewItem {
            retime: Default::default(),
            animations: Animations::default(),
            source_asset: Some(asset.id()),
            source_range: range,
            start: Tick::ZERO,
            content: ItemParams::Caption(CaptionParams {
                cues: CueTrack {
                    cues: cues.to_vec(),
                },
            }),
        },
    })?;
    Ok(session)
}

fn track_id(outcome: Outcome) -> Result<TrackId, EditError> {
    outcome
        .created
        .into_iter()
        .find_map(|created| match created {
            crate::CreatedId::Track(id) => Some(id),
            _ => None,
        })
        .ok_or(EditError::Invalid("track was not created"))
}
pub(super) fn caption_track_id(session: &Session) -> Result<TrackId, EditError> {
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
        .ok_or(EditError::Invalid("missing caption track"))
}
pub(crate) fn caption_item_id(session: &Session) -> Result<project::ItemId, EditError> {
    session
        .project()
        .data()
        .scene()
        .timeline
        .iter()
        .filter(|track| track.kind == TrackKind::Text)
        .flat_map(|track| &track.items)
        .find(|item| matches!(item.content, ItemParams::Caption(_)))
        .map(|item| item.id)
        .ok_or(EditError::Invalid("missing caption item"))
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
) -> Result<(), EditError> {
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
) -> Result<(), EditError> {
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

fn export_key(asset: &SourceAsset, label: &str) -> Result<CacheKey, EditError> {
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

pub(super) fn ensure_distinct(input: &Path, output: &Path) -> Result<(), EditError> {
    if resolve_path(input)? == resolve_path(output)? {
        return Err(EditError::Invalid(
            "output must not replace the input media",
        ));
    }
    Ok(())
}
pub(super) fn resolve_path(path: &Path) -> Result<PathBuf, EditError> {
    match std::fs::canonicalize(path) {
        Ok(value) => Ok(value),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let name = path
                .file_name()
                .ok_or(EditError::Invalid("output path has no file name"))?;
            let parent = path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            Ok(std::fs::canonicalize(parent)?.join(name))
        }
        Err(error) => Err(error.into()),
    }
}

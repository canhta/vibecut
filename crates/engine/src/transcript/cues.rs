//! Builds a project session around a cue set.
use crate::session::EditError;
use crate::{Command, NewItem, Outcome, Session};
use captions::{Cue, CueTrack};
use media::SourceAsset;
use project::{Animations, CaptionParams, ItemParams, Project, ProjectId, TrackId, TrackKind};
use time::{SourceRange, Tick};
/// Build a session holding only `asset` and, with video, its source clip on the main track.
/// Burn-in takes the canvas from the video stream; `portrait` overrides only its orientation.
pub fn source_session(
    asset: &SourceAsset,
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
                    crop: None,
                }),
            },
        })?;
    }
    Ok(session)
}

/// Build a source-timed caption session through the edit registry.
/// Duration is in ticks; invalid source ranges or project geometry refuse atomically.
pub fn build_session(
    asset: &SourceAsset,
    cues: &[Cue],
    duration: Tick,
    with_video: bool,
    portrait: Option<bool>,
) -> Result<Session, EditError> {
    let mut session = source_session(asset, duration, with_video, portrait)?;
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
                word_highlight: false,
                cues: CueTrack {
                    styles: Vec::new(),
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

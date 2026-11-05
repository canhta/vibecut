//! Build a read-only observation project from synthetic media, through real commands.
use engine::{Command, CreatedId, NewItem, Session};
use project::{ItemParams, Project, ProjectId, TrackKind, VideoParams};
use std::path::PathBuf;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let source = PathBuf::from(args.next().ok_or("source path")?);
    let destination = PathBuf::from(args.next().ok_or("project path")?);
    let asset = media::SourceAsset::import(media::AssetId(1), source)?;
    let duration = asset.probe().duration.ok_or("source duration")?;
    let mut session = Session::new(
        Project::empty(ProjectId(1), "Preview observation".into())?,
        100,
    )?;
    session.apply(Command::AddAsset { asset })?;
    let outcome = session.apply(Command::AddTrack {
        placement: engine::TrackPlacement::Auto,
        name: "Video 1".into(),
        kind: TrackKind::Video,
    })?;
    let track = outcome
        .created
        .into_iter()
        .find_map(|id| {
            if let CreatedId::Track(id) = id {
                Some(id)
            } else {
                None
            }
        })
        .ok_or("track id")?;
    session.apply(Command::InsertItem {
        placement: engine::Placement::Track { track },
        item: NewItem {
            animations: Default::default(),
            source_asset: Some(media::AssetId(1)),
            source_range: time::SourceRange::new(time::Tick::ZERO, duration)?,
            start: time::Tick::ZERO,
            content: ItemParams::Video(VideoParams {
                fades: Default::default(),
                source_audio_enabled: true,
                volume: 0.0,
                muted: false,
            }),
        },
    })?;
    let mut writer = project::WriterLease::acquire(&destination)?;
    writer.save(session.project())?;
    Ok(())
}

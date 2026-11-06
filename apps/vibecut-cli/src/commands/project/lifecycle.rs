//! Adapts project lifecycle requests to leased storage.
use crate::args::CliError;
use crate::args::{one_path, utf8};
use crate::output::{Output, hex, json_path};
use crate::session::Context;
use engine::Session;
use project::{ProjectId, WriterLease};
use serde_json::{Value, json};
use std::ffi::OsString;
use std::io;
use std::path::PathBuf;
use time::FrameRate;

pub(crate) fn run(_context: &Context, args: Vec<OsString>) -> Result<Output, CliError> {
    let mut args = args.into_iter();
    let action = args
        .next()
        .ok_or_else(|| CliError::Usage("usage: vibecut-cli project new|info <path>".into()))?;
    let action = utf8(&action, "project action")?;
    let rest: Vec<OsString> = args.collect();
    match action {
        "new" => project_new(one_path(rest, "usage: vibecut-cli project new <path>")?),
        "info" => project_info(one_path(rest, "usage: vibecut-cli project info <path>")?),
        other => Err(CliError::Usage(format!(
            "unknown project action: {other}; expected new or info"
        ))),
    }
}

fn project_new(path: PathBuf) -> Result<Output, CliError> {
    let mut lease = WriterLease::acquire(&path)?;
    match lease.load() {
        Ok(_) => {
            return Err(CliError::Usage(format!(
                "project already exists: {}",
                path.display()
            )));
        }
        Err(project::Error::Io(error)) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(CliError::Project(error)),
    }
    let session = Session::create(
        engine::Command::CreateProject {
            id: ProjectId(0),
            name: "Untitled".into(),
        },
        1,
    )?;
    let receipt = lease.save(session.project())?;
    lease.save_metadata(session.metadata())?;
    let value = json!({
        "path": json_path(receipt.path())?,
        "content_hash": hex(&receipt.content_hash().0),
    });
    Ok(Output::new(
        value,
        format!("created {}", receipt.path().display()),
        false,
        0,
    ))
}

fn project_info(path: PathBuf) -> Result<Output, CliError> {
    let lease = WriterLease::acquire(&path)?;
    let loaded = lease.load()?;
    let metadata = lease.metadata();
    let data = loaded.project.data();
    let duration = loaded.project.duration();
    let tracks = data.scene().timeline.track_count();
    let items = data
        .scene()
        .timeline
        .iter()
        .map(|t| t.items.len())
        .sum::<usize>();
    let fps = data.canvas.fps;
    let value = json!({
        "path": json_path(&path)?,
        "id": data.id.0,
        "name": data.name.as_str(),
        "canvas": {
            "width": data.canvas.width,
            "height": data.canvas.height,
            "fps": fps_json(fps),
            "background": data.canvas.background,
        },
        "duration": {"ticks": duration.0, "seconds": duration.seconds()},
        "assets": data.assets.len(),
        "tracks": tracks,
        "items": items,
        "bookmarks": data.scene().bookmarks.len(),
        "schema_version": loaded.schema_version,
        "metadata": metadata,
    });
    let human = format!(
        "path: {}\nname: {}\ncanvas: {}x{} @ {} fps\nduration: {:.3} s ({} ticks)\nassets: {}, tracks: {}, items: {}, bookmarks: {}\nschema: {}",
        path.display(),
        data.name.as_str(),
        data.canvas.width,
        data.canvas.height,
        fps.label(),
        duration.seconds(),
        duration.0,
        data.assets.len(),
        tracks,
        items,
        data.scene().bookmarks.len(),
        loaded.schema_version,
    );
    Ok(Output::new(value, human, false, 0))
}

fn fps_json(rate: FrameRate) -> Value {
    json!({"num": rate.num(), "den": rate.den(), "fps": rate.as_f64()})
}

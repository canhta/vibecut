//! Adapts project lifecycle requests to leased storage.
use crate::args::CliError;
use crate::args::{one_path, utf8};
use crate::output::{Output, hex, json_path};
use crate::session::{
    Context, Edit, History, Lent, WritePolicy, cli_runner, run_command, run_leased,
};
use engine::Session;
use engine::control::ControlResult;
use project::{ProjectId, WriterLease};
use serde_json::{Value, json};
use std::ffi::OsString;
use std::io;
use std::path::PathBuf;
use time::FrameRate;

pub(crate) fn run(context: &Context, args: Vec<OsString>) -> Result<Output, CliError> {
    let mut args = args.into_iter();
    let action = args.next().ok_or_else(|| {
        CliError::Usage("usage: vibecut-cli project new|info|save-as <path> ...".into())
    })?;
    let action = utf8(&action, "project action")?;
    let rest: Vec<OsString> = args.collect();
    match action {
        "new" => project_new(one_path(rest, "usage: vibecut-cli project new <path>")?),
        "info" => project_info(one_path(rest, "usage: vibecut-cli project info <path>")?),
        "save-as" => {
            let replace = rest.iter().any(|arg| arg.as_os_str() == "--replace");
            let rest = rest
                .into_iter()
                .filter(|arg| arg.as_os_str() != "--replace")
                .collect();
            let (source, destination) = two_paths(
                rest,
                "usage: vibecut-cli project save-as <path> <new-path> [--replace]",
            )?;
            project_save_as(context, source, destination, replace)
        }
        other => Err(CliError::Usage(format!(
            "unknown project action: {other}; expected new, info or save-as"
        ))),
    }
}

/// Exactly two path arguments.
pub(super) fn two_paths(args: Vec<OsString>, usage: &str) -> Result<(PathBuf, PathBuf), CliError> {
    let mut args = args.into_iter().map(PathBuf::from);
    match (args.next(), args.next(), args.next()) {
        (Some(first), Some(second), None) => Ok((first, second)),
        _ => Err(CliError::Usage(usage.to_owned())),
    }
}

/// Publish a copy of `source` under a fresh identity through `project.save_as`.
/// The source is untouched. An existing destination is refused unless `--replace`
/// is given; the project's own file is refused either way.
fn project_save_as(
    context: &Context,
    source: PathBuf,
    destination: PathBuf,
    replace: bool,
) -> Result<Output, CliError> {
    let params = json!({"path": json_path(&destination)?, "replace": replace});
    published(
        run_on_project(context, &source, "project.save_as", params)?,
        "saved",
    )
}

/// Run one read-only registry command over the project under its writer lease.
/// The project is never rewritten: these commands publish elsewhere.
pub(super) fn run_on_project(
    context: &Context,
    project_file: &std::path::Path,
    command: &str,
    params: Value,
) -> Result<ControlResult, CliError> {
    let mut runner = cli_runner(context.budget()?)?;
    let protected = vec![
        project_file.to_path_buf(),
        project::metadata_path(project_file)?,
    ];
    let result = run_leased(
        project_file,
        History::Single,
        WritePolicy::Changed,
        |session| {
            session.protect_paths(protected)?;
            let value = run_command(session, &mut runner, Lent::default(), command, params, true)?;
            Ok(Edit {
                value,
                changed: false,
            })
        },
    );
    runner.shutdown();
    Ok(result?.value)
}

/// Present a `Published` result: the new path, identity and what needs relinking.
pub(super) fn published(result: ControlResult, verb: &str) -> Result<Output, CliError> {
    let ControlResult::Published {
        path,
        project_id,
        copied,
        missing,
        missing_luts,
    } = result
    else {
        return Err(CliError::Rejected("unexpected command result"));
    };
    let mut human = format!("{verb} {}", path.display());
    if !missing.is_empty() {
        human.push_str(&format!("; {} asset(s) need relinking", missing.len()));
    }
    if !missing_luts.is_empty() {
        human.push_str(&format!("; {} LUT(s) are missing", missing_luts.len()));
    }
    let value = json!({
        "path": json_path(&path)?,
        "id": project_id,
        "copied": copied,
        "missing": missing,
        "missing_luts": missing_luts,
    });
    Ok(Output::new(value, human, false, 0))
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
            fps: 30,
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

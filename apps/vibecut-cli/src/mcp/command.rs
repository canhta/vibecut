//! Runs the `mcp` CLI command over stdio or the attach proxy.
use crate::args::CliError;
use crate::output::Output;
use engine::Session;
use mcp::{Framed, Grants, Inbox};
use rmcp::ServiceExt;
use serde_json::Value;
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    time::Duration,
};

/// Maps the adapter's refusals onto the CLI's stable exit codes.
fn cli_error(error: mcp::Error) -> CliError {
    match error {
        mcp::Error::Rejected(reason) => CliError::Rejected(reason),
        mcp::Error::Io(error) => CliError::Io(error),
        mcp::Error::Engine(error) => CliError::Engine(error),
    }
}

pub(crate) fn run(
    context: &crate::session::Context,
    args: Vec<OsString>,
) -> Result<Output, CliError> {
    let mut project = None;
    let mut attach = false;
    let mut socket = None;
    let mut full = false;
    let mut grants = Grants::default();
    let mut save = true;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--attach") => attach = true,
            Some("--socket") => {
                socket = Some(PathBuf::from(
                    args.next().ok_or(CliError::Rejected("socket required"))?,
                ))
            }
            Some("--output-dir") => {
                grants = Grants::output(Path::new(
                    &args
                        .next()
                        .ok_or(CliError::Rejected("output directory required"))?,
                ))
                .map_err(cli_error)?
            }
            Some("--all-tools") => full = true,
            Some("--no-save") => save = false,
            Some(flag) if flag.starts_with('-') => {
                return Err(CliError::Rejected("unknown MCP flag"));
            }
            _ if project.is_none() => project = Some(PathBuf::from(arg)),
            _ => return Err(CliError::Rejected("one MCP project expected")),
        }
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    if attach {
        if project.is_some() || full || grants.output_granted() || !save {
            return Err(CliError::Rejected(
                "attach grants belong to the running app",
            ));
        }
        let socket = socket.ok_or(CliError::Rejected("attach requires --socket"))?;
        runtime.block_on(mcp::proxy(&socket))?;
    } else {
        if socket.is_some() {
            return Err(CliError::Rejected("headless MCP does not bind a socket"));
        }
        let path = project.ok_or(CliError::Rejected(
            "MCP requires project or --attach --socket",
        ))?;
        let mut lease = project::WriterLease::acquire(&path)?;
        let loaded = lease.load()?;
        crate::session::prepare_project_libraries(&loaded.project)?;
        let mut session = Session::new(loaded.project, 100)?;
        session.restore_metadata(lease.metadata())?;
        let mut saved_project = session.project().as_ref().clone();
        let mut saved_metadata = session.metadata().clone();
        let (mut handler, mut inbox) = Inbox::new(grants, context.budget()?).map_err(cli_error)?;
        let captions = crate::session::caption_access(context.budget()?);
        // One models-cache owner per process: vision reuses the captions handle.
        let shared = captions.cache().ok().cloned();
        inbox.attach_captions(std::sync::Arc::new(captions));
        inbox.attach_stickers(crate::session::sticker_lend());
        inbox.attach_vision(std::sync::Arc::new(crate::session::vision_access(
            context.budget()?,
            shared,
        )));
        if full {
            handler.all_tools();
        }
        let worker = std::thread::spawn(move || -> Result<(), CliError> {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(
                || -> Result<(), CliError> {
                    while !inbox.drained() {
                        inbox.poll(&mut session, |project, metadata| {
                            let _ = lease.save(project)?;
                            lease.save_metadata(metadata)?;
                            saved_project = project.clone();
                            saved_metadata = metadata.clone();
                            Ok(())
                        })?;
                        std::thread::sleep(Duration::from_millis(2));
                    }
                    if save {
                        let project_dirty = &saved_project != session.project().as_ref();
                        let metadata_dirty = &saved_metadata != session.metadata() || project_dirty;
                        crate::session::save_control_state(
                            &mut session,
                            &mut lease,
                            project_dirty,
                            metadata_dirty,
                        )?;
                    }
                    Ok(())
                },
            ));
            result.unwrap_or(Err(CliError::Rejected("MCP worker failed")))
        });
        let result = runtime.block_on(async {
            let service = tokio::time::timeout(
                Duration::from_secs(5),
                handler.serve(Framed::new(tokio::io::stdin(), tokio::io::stdout())),
            )
            .await
            .map_err(|_| CliError::Rejected("MCP initialization timed out"))?
            .map_err(|_| CliError::Rejected("MCP initialization failed"))?;
            service
                .waiting()
                .await
                .map_err(|_| CliError::Rejected("MCP service failed"))?;
            Ok::<_, CliError>(())
        });
        // Shut down SDK handler tasks before waiting for the last sender to drop.
        runtime.shutdown_timeout(Duration::from_secs(1));
        worker
            .join()
            .map_err(|_| CliError::Rejected("MCP worker failed"))??;
        result?;
    }
    Ok(Output::new(Value::Null, String::new(), true, 0))
}

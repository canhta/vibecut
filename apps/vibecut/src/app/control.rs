//! Attaches agent control to the live workspace, visible or background, through each window's own edit path.
use super::Workspace;
use crate::{Error, Result, preferences::Preferences};
use std::{
    ffi::OsString,
    path::PathBuf,
    time::{Duration, Instant},
};

/// Route desktop control by project id; focus is only the omitted-target default.
pub fn poll_workspace(workspace: &mut Workspace, inbox: &mut mcp::Inbox) -> Result<()> {
    inbox.retain_projects(|id| workspace.windows.contains_key(&id));
    for (id, window) in &workspace.windows {
        if let Some(editor) = &window.editor {
            inbox.register_project(*id, editor.playback.budget().clone())?;
        }
    }
    inbox.poll_projects(|target, inbox, value| {
        let id = target
            .or(workspace.focused_project())
            .ok_or(engine::control::Error::Control(
                engine::control::ControlError::SessionGone,
            ))?;
        let window = workspace
            .windows
            .get_mut(&id)
            .ok_or(engine::control::Error::Control(
                engine::control::ControlError::SessionGone,
            ))?;
        let editor = window
            .editor
            .as_deref_mut()
            .ok_or(engine::control::Error::Edit(engine::EditError::Invalid(
                "editor activation",
            )))?;
        editor
            .activate_with(None, |editor| {
                Ok(inbox.execute_project(&mut editor.session, value, |_, _| {
                    Err(engine::control::Error::Control(
                        engine::control::ControlError::Permission,
                    ))
                })?)
            })
            .map_err(|error| match error {
                Error::Engine(error) => error,
                _ => engine::control::Error::Edit(engine::EditError::Invalid("editor activation")),
            })
    })?;
    for window in workspace.windows.values_mut() {
        if let Some(editor) = &mut window.editor {
            inbox.observe(&mut editor.session);
        }
    }
    Ok(())
}

/// Run the real app workspace with no OS window, activation or audio device.
/// Syntax: `<project> <socket> <recovery-directory> <seconds>`; bounded to one hour.
pub fn background(arguments: Vec<OsString>) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};
        let main = objc2::MainThreadMarker::new()
            .ok_or(Error::Invalid("background control needs main thread"))?;
        let app = NSApplication::sharedApplication(main);
        if !app.setActivationPolicy(NSApplicationActivationPolicy::Accessory) {
            return Err(Error::Unsupported("background accessory activation policy"));
        }
    }
    if arguments.len() != 4 {
        return Err(Error::Invalid("background control arguments"));
    }
    let project = PathBuf::from(arguments.first().ok_or(Error::Invalid("project"))?);
    let endpoint = PathBuf::from(arguments.get(1).ok_or(Error::Invalid("socket"))?);
    let root = PathBuf::from(
        arguments
            .get(2)
            .ok_or(Error::Invalid("recovery directory"))?,
    );
    let seconds = arguments
        .get(3)
        .and_then(|arg| arg.to_str())
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|seconds| *seconds > 0 && *seconds <= 3600)
        .ok_or(Error::Invalid("background duration"))?;
    let mut workspace = Workspace::new(root, Preferences::default(), Some(project))?;
    let mut server = None;
    let deadline = Instant::now() + Duration::from_secs(seconds);
    while Instant::now() < deadline {
        workspace.tick();
        if workspace.error().is_some() {
            return Err(Error::Invalid("background workspace failed"));
        }
        if workspace.focused_editor().is_some() {
            if server.is_none() {
                let budget = workspace.control_budget();
                let media = workspace.media();
                server = Some(
                    mcp::Server::bind_with_media(&endpoint, Default::default(), budget, media)
                        .map_err(|_| Error::Invalid("control listener"))?,
                );
            }
            if let Some(server) = &mut server {
                poll_workspace(&mut workspace, &mut server.inbox)?;
            }
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    // Autosave/Workspace owns persistence; process shutdown joins its workers.
    Ok(())
}

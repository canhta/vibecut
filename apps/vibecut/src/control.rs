//! Background control uses the same Workspace and Session as the visible app.
use crate::{Error, Result, catalog::Preferences, workspace::Workspace};
use std::{
    ffi::OsString,
    path::PathBuf,
    time::{Duration, Instant},
};

/// Dispatch on the UI owner and refresh playback after a committed external edit.
pub fn poll(editor: &mut crate::Editor, inbox: &mut vibecut_cli::mcp::Inbox) -> Result<()> {
    let before = editor.session.revision();
    inbox.poll(&mut editor.session, |_, _| {
        Err(engine::EditError::Permission)
    })?;
    if before != editor.session.revision() {
        let project = editor.session.project().clone();
        let position = editor
            .playback
            .position()?
            .clamp(time::Tick::ZERO, project.duration());
        editor
            .playback
            .seek(position, editor.playback.playing(), project)?;
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
        if let Some(editor) = workspace.active_editor_mut() {
            if server.is_none() {
                server = Some(
                    vibecut_cli::mcp::Server::bind(
                        &endpoint,
                        Default::default(),
                        editor.playback.budget().clone(),
                    )
                    .map_err(|_| Error::Invalid("control listener"))?,
                );
            }
            if let Some(server) = &mut server {
                poll(editor, &mut server.inbox)?;
            }
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    // Autosave/Workspace owns persistence; process shutdown joins its workers.
    Ok(())
}

//! Desktop session adapter. The engine remains the only autosave/session owner.
use crate::{Error, Result};
use engine::{Command, Session, recovery::Autosave};
use std::ops::{Deref, DerefMut};

/// The editor's session. It derefs to the engine [`Session`] for every read and edit;
/// `Session::apply` refuses lifecycle commands (save, thumbnail), so the deref cannot
/// bypass the journal. Those commands go through [`EditorSession::autosave`], whose
/// `Autosave::apply` routes them.
pub enum EditorSession {
    Memory(Box<Session>),
    Persistent(Box<Autosave>),
}
impl Deref for EditorSession {
    type Target = Session;
    fn deref(&self) -> &Session {
        match self {
            Self::Memory(s) => s,
            Self::Persistent(s) => s.session(),
        }
    }
}
impl DerefMut for EditorSession {
    fn deref_mut(&mut self) -> &mut Session {
        match self {
            Self::Memory(s) => s,
            Self::Persistent(s) => s.session_mut(),
        }
    }
}
impl EditorSession {
    /// The one project-owned media folder (`<project>.media`) that library sounds,
    /// music and voice-over takes are written to; a project that has not been saved
    /// uses a temporary folder.
    pub fn media_directory(&self) -> std::path::PathBuf {
        if let Self::Persistent(autosave) = self
            && let Some(path) = autosave.saved_path()
        {
            let mut media = path.as_os_str().to_os_string();
            media.push(".media");
            return media.into();
        }
        std::env::temp_dir().join("vibecut-media")
    }
    pub fn autosave(&mut self) -> Option<&mut Autosave> {
        match self {
            Self::Persistent(s) => Some(s),
            Self::Memory(_) => None,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditorAction {
    Save,
    SaveAs,
    FocusHome,
    ExitProject,
    OpenHelp,
    OpenFeedback,
    ImportMedia,
    ImportLut,
    ExportSave,
    SnapshotSave,
}

/// Project-owned documentation and issue entry points; no hosted feedback service.
pub(crate) const HELP_URL: &str = "https://github.com/canhta/vibecut/blob/main/docs/README.md";
pub(crate) const FEEDBACK_URL: &str = "https://github.com/canhta/vibecut/issues/new/choose";

pub(crate) fn fresh(name: &str) -> Result<Session> {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| Error::Invalid("system time"))?;
    let id = u64::try_from(epoch.as_nanos())
        .map_err(|_| Error::Invalid("project identity"))?
        .wrapping_add(NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
    Ok(Session::create(
        Command::CreateProject {
            id: project::ProjectId(id),
            name: name.into(),
        },
        100,
    )?)
}

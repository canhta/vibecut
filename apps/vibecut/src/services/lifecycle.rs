//! Desktop session adapter. The engine remains the only autosave/session owner.
use crate::{Error, Result};
use engine::{Command, Session, recovery::Autosave};
use std::ops::{Deref, DerefMut};

/// Frame rate of a new project unless the Settings default says otherwise; Home and Shorts
/// read it here because they sit below the preferences.
pub const DEFAULT_PROJECT_FPS: u32 = 30;

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
    /// The saved project file, which `project::media_dir` places media beside;
    /// `None` for a memory session or a project that has not been saved.
    pub fn saved_path(&self) -> Option<&std::path::Path> {
        match self {
            Self::Persistent(autosave) => autosave.saved_path(),
            Self::Memory(_) => None,
        }
    }
    pub fn autosave(&mut self) -> Option<&mut Autosave> {
        match self {
            Self::Persistent(s) => Some(s),
            Self::Memory(_) => None,
        }
    }
}
/// A caption output the export dialog saves natively: one caption file, or the video burned
/// with the project's captions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptionOutput {
    File(captions::Format),
    Burned,
}

impl CaptionOutput {
    /// The file extension the native save dialog suggests.
    pub const fn extension(self) -> &'static str {
        match self {
            Self::File(captions::Format::Srt) => "srt",
            Self::File(captions::Format::WebVtt) => "vtt",
            Self::File(captions::Format::Txt) => "txt",
            Self::Burned => "mp4",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditorAction {
    Save,
    SaveAs,
    FocusHome,
    ExitProject,
    DeleteProject,
    OpenHelp,
    OpenFeedback,
    ImportMedia,
    ImportLut,
    ExportClips,
    ExportSave,
    ExportCaption(CaptionOutput),
    SnapshotSave,
    KeybindingsExport,
    KeybindingsImport {
        replace: bool,
    },
    /// Save one timeline clip's download (`export.clip`) to the destination the dialog picks.
    ClipDownload {
        item: project::ItemId,
        mode: engine::ClipExportMode,
    },
    /// The Add captions Import button: the window picks an SRT/VTT file for `at`.
    ImportSubtitles {
        at: time::Tick,
    },
    /// The font picker's Import action: the window picks a custom font file.
    FontUpload,
}

/// Project-owned documentation and issue entry points; no hosted feedback service.
pub(crate) const HELP_URL: &str = "https://github.com/canhta/vibecut/blob/main/docs/README.md";
pub(crate) const FEEDBACK_URL: &str = "https://github.com/canhta/vibecut/issues/new/choose";

pub(crate) fn fresh(name: &str, fps: u32) -> Result<Session> {
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
            fps,
        },
        100,
    )?)
}

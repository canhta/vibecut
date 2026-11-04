//! Desktop session adapter. The engine remains the only autosave/session owner.
use engine::{Session, recovery::Autosave};
use std::ops::{Deref, DerefMut};

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
    pub fn autosave(&mut self) -> Option<&mut Autosave> {
        match self {
            Self::Persistent(s) => Some(s),
            Self::Memory(_) => None,
        }
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum EditorAction {
    Save,
    SaveAs,
    Leave,
    ImportMedia,
}

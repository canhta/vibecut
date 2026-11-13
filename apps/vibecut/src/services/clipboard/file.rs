//! Services the OS clipboard's file list; the reader is injected so tests never touch the clipboard.
use std::path::PathBuf;

/// The app's read-only native boundary for files copied in the OS file manager.
pub struct FileClipboard {
    read: Box<dyn FnMut() -> Vec<PathBuf> + Send>,
}
impl Default for FileClipboard {
    /// No clipboard access: the reader returns nothing until the app wires the native one.
    fn default() -> Self {
        Self::new(Vec::new)
    }
}
impl FileClipboard {
    pub fn new(read: impl FnMut() -> Vec<PathBuf> + Send + 'static) -> Self {
        Self {
            read: Box::new(read),
        }
    }
    /// The platform clipboard's file list; an unavailable clipboard reads as no files.
    pub fn native() -> Self {
        Self::new(|| {
            arboard::Clipboard::new()
                .ok()
                .and_then(|mut clipboard| clipboard.get().file_list().ok())
                .unwrap_or_default()
        })
    }
    pub fn read(&mut self) -> Vec<PathBuf> {
        (self.read)()
    }
}

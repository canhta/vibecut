//! Services read-only access to the OS clipboard behind injectable readers.
mod file;
mod text;

pub use file::FileClipboard;
pub use text::TextClipboard;

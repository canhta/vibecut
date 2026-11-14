//! Built-in text and caption looks: closed ids with their shelves, names and style values.
mod caption;
mod text;

pub use caption::{CaptionLook, CaptionShelf};
pub use text::{
    TEXT_LIBRARY_LICENSE, TextLook, TextShelf, TextTemplate, is_commercial_license,
    search_text_looks, search_text_templates,
};

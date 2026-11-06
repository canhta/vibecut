//! Stores assets navigation drafts.
use super::{AssetsTab, BTreeMap, Model, Text};

#[derive(Default)]
pub(in crate::editor) struct State {
    pub(super) model: Model,
    pub(super) searches: BTreeMap<AssetsTab, String>,
    pub(super) filter: Filter,
    pub(super) transition_family: Option<&'static str>,
    pub(super) transition_previews: BTreeMap<&'static str, super::transition::Preview>,
    pub(in crate::editor) transition_drag: Option<(&'static str, std::sync::Arc<project::Project>)>,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub(super) enum Filter {
    #[default]
    All,
    Video,
    Audio,
    Image,
}
impl Filter {
    pub(super) const ALL: [Self; 4] = [Self::All, Self::Video, Self::Audio, Self::Image];
    pub(super) fn label(self) -> Text {
        match self {
            Self::All => Text::AssetsAll,
            Self::Video => Text::Video,
            Self::Audio => Text::Audio,
            Self::Image => Text::Image,
        }
    }
    pub(super) fn id(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Video => "video",
            Self::Audio => "audio",
            Self::Image => "image",
        }
    }
}

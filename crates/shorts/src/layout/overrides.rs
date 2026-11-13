//! Holds the layouts a reviewer pins for a short or one of its segments.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// One of the seven layouts the research observed in the reference editors.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Layout {
    /// The active speaker cropped to the target ratio.
    Fill,
    /// The whole frame, letterboxed with padding.
    Fit,
    /// Two people on screen together.
    Split,
    /// Three people on screen together.
    Three,
    /// Four people on screen together.
    Four,
    /// Screen content above, the speaker below.
    Screenshare,
    /// The speaker above (30 %), the content below (70 %).
    Gameplay,
}

impl Layout {
    /// Every layout in display order.
    pub const ALL: [Self; 7] = [
        Self::Fill,
        Self::Fit,
        Self::Split,
        Self::Three,
        Self::Four,
        Self::Screenshare,
        Self::Gameplay,
    ];

    /// The persisted spelling.
    pub const fn id(self) -> &'static str {
        match self {
            Self::Fill => "fill",
            Self::Fit => "fit",
            Self::Split => "split",
            Self::Three => "three",
            Self::Four => "four",
            Self::Screenshare => "screenshare",
            Self::Gameplay => "gameplay",
        }
    }
}

/// The reviewer's layout choices for one short. An absent choice leaves the
/// automatic per-shot choice in force; a segment choice beats the short's.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LayoutOverrides {
    /// Layout forced on every segment without its own choice.
    pub short: Option<Layout>,
    /// Layout forced on the segment at this play-order index.
    pub segments: BTreeMap<usize, Layout>,
    /// Keep the short's first visible speaker in the top slot of Split, Three and Four.
    pub pin_first_speaker: bool,
}

/// One reviewer edit of [`LayoutOverrides`]. Applying an edit returns the edit
/// that undoes it, so a history stores inverses and replays them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutEdit {
    /// Set or clear the short's layout.
    Short(Option<Layout>),
    /// Set or clear the layout of one segment.
    Segment {
        index: usize,
        layout: Option<Layout>,
    },
    /// Turn "pin first speaker" on or off.
    PinFirstSpeaker(bool),
}

impl LayoutOverrides {
    /// The layout forced on a segment: its own choice, else the short's.
    pub fn for_segment(&self, index: usize) -> Option<Layout> {
        self.segments.get(&index).copied().or(self.short)
    }

    /// Apply one edit and return its inverse.
    pub fn apply(&mut self, edit: LayoutEdit) -> LayoutEdit {
        match edit {
            LayoutEdit::Short(layout) => {
                LayoutEdit::Short(std::mem::replace(&mut self.short, layout))
            }
            LayoutEdit::Segment { index, layout } => {
                let before = match layout {
                    Some(layout) => self.segments.insert(index, layout),
                    None => self.segments.remove(&index),
                };
                LayoutEdit::Segment {
                    index,
                    layout: before,
                }
            }
            LayoutEdit::PinFirstSpeaker(on) => {
                LayoutEdit::PinFirstSpeaker(std::mem::replace(&mut self.pin_first_speaker, on))
            }
        }
    }
}

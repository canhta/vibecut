//! Timeline tracks, their kinds and ordering.
use super::item::Item;
use captions::NfcText;
use serde::{Deserialize, Serialize};

id!(TrackId);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TrackKind {
    Video,
    Audio,
    Text,
    Sticker,
    Effect,
    Filter,
    Adjustment,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Track {
    pub id: TrackId,
    pub name: NfcText,
    pub kind: TrackKind,
    pub muted: bool,
    pub hidden: bool,
    pub locked: bool,
    pub items: Vec<Item>,
}
/// Tracks are grouped in screen order, with one mandatory main video track.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Timeline {
    pub transitions: Vec<crate::Transition>,
    /// Topmost overlay first. Audio tracks belong in `audio` instead.
    pub overlays: Vec<Track>,
    pub main: Track,
    /// Main-track magnet preference; structural gap closing is an engine edit policy.
    pub magnet: bool,
    pub audio: Vec<Track>,
}
impl Default for Timeline {
    fn default() -> Self {
        Self {
            transitions: Vec::new(),
            overlays: Vec::new(),
            main: Track {
                id: TrackId(0),
                name: "Main Track".into(),
                kind: TrackKind::Video,
                muted: false,
                hidden: false,
                locked: false,
                items: Vec::new(),
            },
            audio: Vec::new(),
            magnet: true,
        }
    }
}
impl Timeline {
    /// Bottom-to-top compositing order; reverse for screen order.
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &Track> {
        self.audio
            .iter()
            .rev()
            .chain(std::iter::once(&self.main))
            .chain(self.overlays.iter().rev())
    }
    /// Mutable traversal in the same order as `iter`.
    pub fn iter_mut(&mut self) -> impl DoubleEndedIterator<Item = &mut Track> {
        self.audio
            .iter_mut()
            .rev()
            .chain(std::iter::once(&mut self.main))
            .chain(self.overlays.iter_mut().rev())
    }
    pub fn track_count(&self) -> usize {
        self.overlays.len() + 1 + self.audio.len()
    }
    /// Insert a secondary track at the highest position for its group.
    pub fn insert_track(&mut self, track: Track) {
        if track.kind == TrackKind::Audio {
            self.audio.insert(0, track);
        } else {
            self.overlays.insert(0, track);
        }
    }
}
impl TrackKind {
    pub fn can_mute(self) -> bool {
        matches!(self, Self::Video | Self::Audio)
    }
    pub fn can_hide(self) -> bool {
        self != Self::Audio
    }
}

//! Owns the open mask sub-panel, its shape choices and typed field drafts.
use crate::i18n::Text;
use project::{ItemId, Mask, MaskShape};
use std::collections::BTreeMap;

/// One choice in the reference's shape grid, in capture order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    None,
    Horizontal,
    Mirror,
    Circle,
    Rectangle,
    Heart,
    Stars,
}

impl Choice {
    pub const ALL: [Self; 7] = [
        Self::None,
        Self::Horizontal,
        Self::Mirror,
        Self::Circle,
        Self::Rectangle,
        Self::Heart,
        Self::Stars,
    ];

    pub const fn id(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Horizontal => "horizontal",
            Self::Mirror => "mirror",
            Self::Circle => "circle",
            Self::Rectangle => "rectangle",
            Self::Heart => "heart",
            Self::Stars => "stars",
        }
    }

    pub const fn label(self) -> Text {
        match self {
            Self::None => Text::DecorationNone,
            Self::Horizontal => Text::MaskHorizontal,
            Self::Mirror => Text::MaskMirror,
            Self::Circle => Text::MaskCircle,
            Self::Rectangle => Text::MaskRectangle,
            Self::Heart => Text::MaskHeart,
            Self::Stars => Text::MaskStars,
        }
    }

    /// The engine shape this capture tile shows; `None` removes the mask.
    pub fn shape(self) -> Option<MaskShape> {
        Some(match self {
            Self::None => return None,
            Self::Horizontal => MaskShape::Split,
            Self::Mirror => MaskShape::CinematicBars,
            Self::Circle => MaskShape::Ellipse,
            Self::Rectangle => MaskShape::Rectangle,
            Self::Heart => MaskShape::Heart,
            Self::Stars => MaskShape::Star,
        })
    }

    /// The tile that shows `shape`, or `None` for shapes the grid does not list.
    pub fn of(shape: &MaskShape) -> Option<Self> {
        match shape {
            MaskShape::Split => Some(Self::Horizontal),
            MaskShape::CinematicBars => Some(Self::Mirror),
            MaskShape::Ellipse => Some(Self::Circle),
            MaskShape::Rectangle => Some(Self::Rectangle),
            MaskShape::Heart => Some(Self::Heart),
            MaskShape::Star => Some(Self::Stars),
            _ => None,
        }
    }

    /// Size fields the reference shows for this shape: width, height.
    pub const fn size_fields(self) -> (bool, bool) {
        match self {
            Self::Horizontal => (false, false),
            Self::Mirror => (false, true),
            _ => (true, true),
        }
    }

    /// A fresh mask's rotation: the split's normal points right at zero, so
    /// the horizontal choice starts turned a quarter to keep the top half.
    pub const fn default_rotation(self) -> f64 {
        match self {
            Self::Horizontal => -90.0,
            _ => 0.0,
        }
    }
}

/// The mask a row opens: the first mask the item carries, if any.
pub fn current(item: &project::Item) -> Option<&Mask> {
    item.masks.first()
}

/// Transient sub-panel state; the panel itself lives on the properties area.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct State {
    open: bool,
    item: Option<ItemId>,
    drafts: BTreeMap<&'static str, String>,
    feather: Option<f64>,
}

impl State {
    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn item(&self) -> Option<ItemId> {
        self.item
    }

    pub fn open(&mut self, item: ItemId) {
        self.open = true;
        self.item = Some(item);
        self.drafts.clear();
        self.feather = None;
    }

    pub fn close(&mut self) {
        self.open = false;
        self.item = None;
        self.drafts.clear();
        self.feather = None;
    }

    /// The feather value under the pointer while its rail drags; it previews
    /// the thumb and commits once on release.
    pub fn feather_preview(&self) -> Option<f64> {
        self.feather
    }

    pub fn set_feather_preview(&mut self, value: Option<f64>) {
        self.feather = value;
    }

    pub fn draft(&self, key: &'static str) -> Option<&str> {
        self.drafts.get(key).map(String::as_str)
    }

    pub fn set_draft(&mut self, key: &'static str, text: String) {
        self.drafts.insert(key, text);
    }

    pub fn clear_draft(&mut self, key: &'static str) {
        self.drafts.remove(key);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::{Choice, current};
    use project::{Mask, MaskShape};

    #[test]
    fn the_capture_tiles_round_trip_through_the_engine_shapes() {
        for choice in Choice::ALL {
            let Some(shape) = choice.shape() else {
                assert_eq!(choice, Choice::None);
                assert_eq!(Choice::of(&MaskShape::Ellipse), Some(Choice::Circle));
                continue;
            };
            assert_eq!(Choice::of(&shape), Some(choice), "{choice:?}");
        }
        assert_eq!(Choice::of(&MaskShape::Diamond), None);
        assert_eq!(
            Choice::of(&MaskShape::Path {
                path: Default::default()
            }),
            None
        );
    }

    #[test]
    fn only_the_split_shape_starts_turned_and_size_fields_follow_the_capture() {
        assert_eq!(Choice::Horizontal.default_rotation(), -90.0);
        for choice in Choice::ALL.into_iter().filter(|c| *c != Choice::Horizontal) {
            assert_eq!(choice.default_rotation(), 0.0, "{choice:?}");
        }
        assert_eq!(Choice::Horizontal.size_fields(), (false, false));
        assert_eq!(Choice::Mirror.size_fields(), (false, true));
        assert_eq!(Choice::Rectangle.size_fields(), (true, true));
    }

    #[test]
    fn the_row_reads_the_first_mask_and_closing_drops_drafts() {
        let mut item = project::Item {
            hidden: false,
            blend_mode: Default::default(),
            retime: Default::default(),
            masks: vec![Mask::new("mask1", MaskShape::Heart)],
            effects: Vec::new(),
            animations: Default::default(),
            id: project::ItemId(1),
            source_asset: None,
            source_range: time::SourceRange::new(
                time::Tick::ZERO,
                time::FrameRate::FPS_30.tick_of(1).unwrap(),
            )
            .unwrap(),
            start: time::Tick::ZERO,
            content: project::ItemParams::Overlay(project::OverlayParams {
                color: project::Color([0; 4]),
            }),
        };
        assert_eq!(current(&item).map(|mask| mask.id.as_str()), Some("mask1"));
        item.masks.clear();
        assert!(current(&item).is_none());

        let mut state = super::State::default();
        state.open(project::ItemId(1));
        state.set_draft("feather", "10".into());
        state.close();
        assert!(!state.is_open());
        assert!(state.draft("feather").is_none());
    }
}

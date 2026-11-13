//! Presents the selection scopes nested inside the session's item selection: items, keyframes or mask points.
use crate::egui::{Pos2, Rect};
use project::{AnimationPath, ColorComponent, ItemId, TransitionId};
use std::collections::BTreeSet;
use time::Tick;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionScope {
    None,
    Items,
    Keyframes,
    MaskPoints,
    /// One timeline transition; it replaces the item selection.
    Transition,
}
/// Identity includes the channel: simultaneous keys on distinct paths stay distinct.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyframeSelection {
    pub item: ItemId,
    pub path: AnimationPath,
    pub component: Option<ColorComponent>,
    pub time: Tick,
}
impl Ord for KeyframeSelection {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        let component = |c: Option<ColorComponent>| {
            c.map(|c| match c {
                ColorComponent::R => 0,
                ColorComponent::G => 1,
                ColorComponent::B => 2,
                ColorComponent::A => 3,
            })
        };
        (self.item, &self.path, component(self.component), self.time).cmp(&(
            other.item,
            &other.path,
            component(other.component),
            other.time,
        ))
    }
}
impl PartialOrd for KeyframeSelection {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct MaskPointSelection {
    pub item: ItemId,
    /// Stable `Mask::id`, never the vector index; reorder cannot retarget it.
    pub mask: String,
    pub point: usize,
}
/// Nested selections are owned by the app; future lanes/handles use these same sets.
#[derive(Clone, Debug, Default)]
pub struct SelectionScopes {
    pub mask_points: BTreeSet<MaskPointSelection>,
    pub keyframes: BTreeSet<KeyframeSelection>,
    /// The selected transition; never coexists with selected items.
    pub transition: Option<TransitionId>,
}
impl SelectionScopes {
    pub fn active(&self, items: &BTreeSet<ItemId>) -> SelectionScope {
        if !self.mask_points.is_empty() {
            SelectionScope::MaskPoints
        } else if !self.keyframes.is_empty() {
            SelectionScope::Keyframes
        } else if !items.is_empty() {
            SelectionScope::Items
        } else if self.transition.is_some() {
            SelectionScope::Transition
        } else {
            SelectionScope::None
        }
    }
    /// Pop the most specific scope. Returns true when that scope is the item
    /// selection itself, which the caller clears through the session.
    pub fn clear_specific(&mut self, items: &BTreeSet<ItemId>) -> bool {
        match self.active(items) {
            SelectionScope::MaskPoints => self.mask_points.clear(),
            SelectionScope::Keyframes => self.keyframes.clear(),
            SelectionScope::Items => return true,
            SelectionScope::Transition => self.transition = None,
            SelectionScope::None => {}
        }
        false
    }
}

pub(crate) fn intersections<T: Copy + Ord>(
    start: Pos2,
    end: Pos2,
    items: impl Iterator<Item = (T, Rect)>,
) -> BTreeSet<T> {
    let marquee = Rect::from_two_pos(start, end);
    items
        .filter(|(_, rect)| rect.intersects(marquee))
        .map(|(id, _)| id)
        .collect()
}

/// Ordered list surfaces opt into Shift ranges; timeline clips pass no order.
pub(crate) fn click<T: Copy + Ord>(
    selected: &mut BTreeSet<T>,
    anchor: &mut Option<T>,
    id: T,
    toggle: bool,
    range: Option<&[T]>,
) {
    if let Some(order) = range
        && let Some(from) = anchor.and_then(|a| order.iter().position(|v| *v == a))
        && let Some(to) = order.iter().position(|v| *v == id)
        && let Some(span) = order.get(from.min(to)..=from.max(to))
    {
        *selected = span.iter().copied().collect();
        return;
    }
    if toggle {
        if !selected.remove(&id) {
            selected.insert(id);
        }
    } else {
        selected.clear();
        selected.insert(id);
    }
    *anchor = Some(id);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn click_toggle_and_ordered_ranges_keep_surface_semantics() {
        let mut selected = BTreeSet::new();
        let mut anchor = None;
        for (id, toggle, range, expected) in [
            (2, false, false, vec![2]),
            (4, true, false, vec![2, 4]),
            (4, true, false, vec![2]),
            (1, false, false, vec![1]),
            (3, false, true, vec![1, 2, 3]),
            (2, false, true, vec![1, 2]),
            (4, true, false, vec![1, 2, 4]),
        ] {
            click(
                &mut selected,
                &mut anchor,
                id,
                toggle,
                range.then_some(&[1, 2, 3, 4]),
            );
            assert_eq!(selected, expected.into_iter().collect());
        }
    }
    #[test]
    #[allow(clippy::expect_used, reason = "known animation path fixture")]
    fn escape_pops_specific_scopes_before_items() {
        let mut items = [ItemId(1)].into_iter().collect();
        let mut scopes = SelectionScopes::default();
        scopes.mask_points.insert(MaskPointSelection {
            item: ItemId(1),
            mask: "mask1".into(),
            point: 0,
        });
        for path in ["opacity", "transform.scaleX"] {
            scopes.keyframes.insert(KeyframeSelection {
                item: ItemId(1),
                path: AnimationPath::new(path).expect("known path"),
                component: None,
                time: Tick::ZERO,
            });
        }
        assert_eq!(scopes.keyframes.len(), 2);
        for expected in [
            SelectionScope::MaskPoints,
            SelectionScope::Keyframes,
            SelectionScope::Items,
            SelectionScope::None,
        ] {
            assert_eq!(scopes.active(&items), expected);
            if scopes.clear_specific(&items) {
                items.clear();
            }
        }
    }
    #[test]
    fn marquee_intersects_in_both_directions_including_edges() {
        let items = [
            (
                ItemId(1),
                Rect::from_min_max(Pos2::new(10., 10.), Pos2::new(20., 20.)),
            ),
            (
                ItemId(2),
                Rect::from_min_max(Pos2::new(30., 10.), Pos2::new(40., 20.)),
            ),
        ];
        for (start, end, expected) in [
            ((5., 5.), (10., 10.), vec![ItemId(1)]),
            ((25., 25.), (15., 15.), vec![ItemId(1)]),
            ((21., 21.), (29., 29.), vec![]),
        ] {
            assert_eq!(
                intersections(
                    Pos2::new(start.0, start.1),
                    Pos2::new(end.0, end.1),
                    items.into_iter()
                ),
                expected.into_iter().collect()
            );
        }
    }
}

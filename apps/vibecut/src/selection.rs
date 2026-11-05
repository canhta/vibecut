//! Transient nested scopes and shared inspector values; project edits stay in engine.
use crate::egui::{Pos2, Rect};
use project::{AnimationPath, ColorComponent, ItemId, ItemParams, Project};
use std::collections::BTreeSet;
use time::Tick;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionScope {
    None,
    Items,
    Keyframes,
    MaskPoints,
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct MaskPointSelection {
    pub item: ItemId,
    pub mask: usize,
    pub point: usize,
}
/// Nested selections are owned by the app; future lanes/handles use these same sets.
#[derive(Clone, Debug, Default)]
pub struct SelectionScopes {
    pub mask_points: BTreeSet<MaskPointSelection>,
    pub keyframes: BTreeSet<KeyframeSelection>,
}
impl SelectionScopes {
    pub fn active(&self, items: &BTreeSet<ItemId>) -> SelectionScope {
        if !self.mask_points.is_empty() {
            SelectionScope::MaskPoints
        } else if !self.keyframes.is_empty() {
            SelectionScope::Keyframes
        } else if !items.is_empty() {
            SelectionScope::Items
        } else {
            SelectionScope::None
        }
    }
    pub fn clear_specific(&mut self, items: &mut BTreeSet<ItemId>) {
        match self.active(items) {
            SelectionScope::MaskPoints => self.mask_points.clear(),
            SelectionScope::Keyframes => self.keyframes.clear(),
            SelectionScope::Items => items.clear(),
            SelectionScope::None => {}
        }
    }
}

pub(crate) fn intersections(
    start: Pos2,
    end: Pos2,
    items: impl Iterator<Item = (ItemId, Rect)>,
) -> BTreeSet<ItemId> {
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

#[derive(Clone, Debug, PartialEq)]
pub struct SharedField {
    pub id: &'static str,
    /// None means mixed values; the UI shows a dash, never a representative value.
    pub value: Option<String>,
    pub compatible: usize,
}
/// Aggregate only compatible items. An edit never copies unrelated fields.
pub fn shared_fields(project: &Project, selection: &BTreeSet<ItemId>) -> Vec<SharedField> {
    let items: Vec<_> = project
        .data()
        .scene()
        .timeline
        .iter()
        .flat_map(|t| &t.items)
        .filter(|i| selection.contains(&i.id))
        .collect();
    [
        "blendMode",
        "opacity",
        "audio_gain",
        "audio_muted",
        "text_size",
        "text_color",
    ]
    .into_iter()
    .filter_map(|id| {
        let values: Vec<_> = items
            .iter()
            .filter_map(|i| match (id, &i.content) {
                ("blendMode", content) if !matches!(content, ItemParams::Audio(_)) => {
                    Some(i.blend_mode.id().into())
                }
                ("opacity", content) if !matches!(content, ItemParams::Audio(_)) => {
                    let path = AnimationPath::new("opacity").ok()?;
                    let value = i.animations.value_at(&path, Tick::ZERO).ok().flatten();
                    Some(match value {
                        Some(project::ParamValue::Scalar(v)) => v.clamp(0.0, 1.0).to_string(),
                        _ => "1".into(),
                    })
                }
                ("audio_gain", ItemParams::Audio(p)) => Some(if p.gain_db == 0.0 {
                    "0".to_owned()
                } else {
                    p.gain_db.to_string()
                }),
                ("audio_muted", ItemParams::Audio(p)) => Some(p.muted.to_string()),
                ("text_size", ItemParams::Text(p)) => Some(p.font_size.to_string()),
                ("text_color", ItemParams::Text(p)) => {
                    let [r, g, b, a] = p.color.0;
                    Some(format!("#{r:02X}{g:02X}{b:02X}{a:02X}"))
                }
                _ => None,
            })
            .collect();
        let first = values.first()?;
        Some(SharedField {
            id,
            value: values.iter().all(|v| v == first).then(|| first.clone()),
            compatible: values.len(),
        })
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[allow(clippy::unwrap_used, reason = "validated fixture assertions")]
    fn shared_fields_distinguish_mixed_values_from_equal_values() {
        use project::{Color, Item, ProjectId, TextParams, Track, TrackId, TrackKind};
        let mut data = Project::empty(ProjectId(1), "".into()).unwrap().into_data();
        data.scene_mut().timeline.insert_track(Track {
            id: TrackId(1),
            name: "Text".into(),
            kind: TrackKind::Text,
            muted: false,
            hidden: false,
            locked: false,
            items: (0..2)
                .map(|n| Item {
                    blend_mode: Default::default(),
                    masks: Vec::new(),
                    effects: vec![],
                    animations: Default::default(),
                    id: ItemId(n + 10),
                    source_asset: None,
                    source_range: time::SourceRange::new(Tick::ZERO, Tick(100)).unwrap(),
                    start: Tick(n as i64 * 100),
                    content: ItemParams::Text(TextParams {
                        content: "Keep".into(),
                        font_size: 24.0 + n as f32,
                        color: Color([10, 20, 30, 255]),
                    }),
                })
                .collect(),
        });
        let project = Project::new(data).unwrap();
        assert!(shared_fields(&project, &BTreeSet::new()).is_empty());
        for (ids, expected_size, count) in [
            (vec![ItemId(10)], Some("24"), 1),
            (vec![ItemId(10), ItemId(11)], None, 2),
        ] {
            let fields = shared_fields(&project, &ids.into_iter().collect());
            assert_eq!(
                fields,
                vec![
                    SharedField {
                        id: "blendMode",
                        value: Some("normal".into()),
                        compatible: count
                    },
                    SharedField {
                        id: "opacity",
                        value: Some("1".into()),
                        compatible: count
                    },
                    SharedField {
                        id: "text_size",
                        value: expected_size.map(str::to_owned),
                        compatible: count
                    },
                    SharedField {
                        id: "text_color",
                        value: Some("#0A141EFF".into()),
                        compatible: count
                    }
                ]
            );
        }
    }
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
            mask: 0,
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
            scopes.clear_specific(&mut items);
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

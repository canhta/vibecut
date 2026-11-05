use crate::{Error, Item, ItemId, ItemParams, ProjectData, Result, Timeline, TrackId, TrackKind};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use time::Tick;

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(transparent)]
pub struct TransitionId(pub u64);

/// Placement relative to the shared cut; Custom retains an explicitly moved span.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TransitionAlignment {
    Start,
    Center,
    End,
    Custom,
}

/// The admitted catalog is shared by command validation and rendering.
#[derive(Clone, Copy, Debug)]
pub struct TransitionDefinition {
    pub kind: &'static str,
    pub name: &'static str,
    pub family: &'static str,
    /// None is a cross dissolve; Some is an opaque linear-light dip colour.
    pub dip: Option<[f32; 4]>,
}
pub const TRANSITION_DEFINITIONS: &[TransitionDefinition] = &[
    TransitionDefinition {
        kind: "cross_dissolve",
        name: "Cross Dissolve",
        family: "Dissolve",
        dip: None,
    },
    TransitionDefinition {
        kind: "dip_to_black",
        name: "Dip to Black",
        family: "Dissolve",
        dip: Some([0., 0., 0., 1.]),
    },
    TransitionDefinition {
        kind: "dip_to_white",
        name: "Dip to White",
        family: "Dissolve",
        dip: Some([1.; 4]),
    },
];
/// Look up an admitted kind; unsupported names return a typed error capped at 64 characters.
pub fn transition_definition(kind: &str) -> Result<&'static TransitionDefinition> {
    TRANSITION_DEFINITIONS
        .iter()
        .find(|d| d.kind == kind)
        .ok_or_else(|| Error::UnsupportedTransition(kind.chars().take(64).collect()))
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
/// A video-only timeline span joining two adjacent clips over their shared cut.
pub struct Transition {
    pub id: TransitionId,
    pub track: TrackId,
    pub from: ItemId,
    pub to: ItemId,
    /// Earlier outgoing pieces, in timeline order; `from` remains at the shared cut.
    pub from_pieces: Vec<ItemId>,
    /// Later incoming pieces, in timeline order; `to` remains at the shared cut.
    pub to_pieces: Vec<ItemId>,
    pub kind: String,
    pub start: Tick,
    pub duration: Tick,
    pub alignment: TransitionAlignment,
}
impl Transition {
    /// Resolve both complete sides. Split pieces must remain contiguous and on
    /// the transition's track; edits that break a side invalidate the transition.
    fn sides<'a>(&self, timeline: &'a Timeline) -> Result<(&'a [Item], &'a [Item])> {
        self.clips(timeline)?;
        let track = timeline
            .iter()
            .find(|t| t.id == self.track)
            .ok_or(Error::Invalid("transition track"))?;
        let from = track
            .items
            .iter()
            .position(|i| i.id == self.from)
            .ok_or(Error::Invalid("transition outgoing clip"))?;
        let first = from
            .checked_sub(self.from_pieces.len())
            .ok_or(Error::Invalid("transition pieces"))?;
        let to = from
            .checked_add(1)
            .ok_or(Error::Invalid("transition pieces"))?;
        let end = to
            .checked_add(self.to_pieces.len())
            .and_then(|n| n.checked_add(1))
            .ok_or(Error::Invalid("transition pieces"))?;
        let a = track
            .items
            .get(first..to)
            .ok_or(Error::Invalid("transition pieces"))?;
        let b = track
            .items
            .get(to..end)
            .ok_or(Error::Invalid("transition pieces"))?;
        let valid = |items: &[Item]| {
            items
                .iter()
                .all(|i| matches!(i.content, ItemParams::Video(_) | ItemParams::Overlay(_)))
                && items.windows(2).all(|p| match (p.first(), p.get(1)) {
                    (Some(a), Some(b)) => a.end().ok() == Some(b.start),
                    _ => false,
                })
        };
        if !valid(a)
            || !valid(b)
            || !a
                .iter()
                .map(|i| i.id)
                .eq(self.from_pieces.iter().copied().chain([self.from]))
            || !b
                .iter()
                .map(|i| i.id)
                .eq(std::iter::once(self.to).chain(self.to_pieces.iter().copied()))
        {
            return Err(Error::Invalid("transition piece continuity"));
        }
        Ok((a, b))
    }
    /// The piece that continues each side at `at`, including its source handles.
    pub fn contributors<'a>(
        &self,
        timeline: &'a Timeline,
        at: Tick,
    ) -> Result<(&'a Item, &'a Item)> {
        let (a, b) = self.sides(timeline)?;
        let select = |items: &'a [Item]| -> Result<&'a Item> {
            items
                .iter()
                .rev()
                .find(|i| i.start <= at)
                .or_else(|| items.first())
                .ok_or(Error::Invalid("transition side"))
        };
        Ok((select(a)?, select(b)?))
    }

    /// Extend a side's identity across a non-destructive split.
    pub fn split_piece(&mut self, left: ItemId, right: ItemId) {
        if self.from == left {
            self.from_pieces.push(left);
            self.from = right;
        } else if let Some(n) = self.from_pieces.iter().position(|i| *i == left) {
            self.from_pieces.insert(n + 1, right);
        }
        if self.to == left {
            self.to_pieces.insert(0, right);
        } else if let Some(n) = self.to_pieces.iter().position(|i| *i == left) {
            self.to_pieces.insert(n + 1, right);
        }
    }
    /// Exclusive output end in ticks; overflow returns Invalid.
    pub fn end(&self) -> Result<Tick> {
        self.start.add_tick(self.duration)
    }
    pub fn contains(&self, at: Tick) -> bool {
        self.start <= at && self.end().is_ok_and(|end| at < end)
    }
    /// Resolve adjacent touching video/overlay clips on the named video track.
    pub fn clips<'a>(&self, timeline: &'a Timeline) -> Result<(&'a Item, &'a Item)> {
        let track = timeline
            .iter()
            .find(|t| t.id == self.track && t.kind == TrackKind::Video)
            .ok_or(Error::Invalid("transition track"))?;
        let pair = track
            .items
            .windows(2)
            .find(|p| {
                p.first().is_some_and(|i| i.id == self.from)
                    && p.get(1).is_some_and(|i| i.id == self.to)
            })
            .ok_or(Error::Invalid("transition adjacent clips"))?;
        let a = pair
            .first()
            .ok_or(Error::Invalid("transition outgoing clip"))?;
        let b = pair
            .get(1)
            .ok_or(Error::Invalid("transition incoming clip"))?;
        if a.end()? != b.start
            || ![a, b]
                .iter()
                .all(|i| matches!(i.content, ItemParams::Video(_) | ItemParams::Overlay(_)))
        {
            return Err(Error::Invalid("transition cut"));
        }
        Ok((a, b))
    }
    /// Earliest start, shared cut and latest end, in output ticks. Source handles
    /// bound extensions; unknown source duration refuses an outgoing extension.
    pub fn bounds(&self, timeline: &Timeline, data: &ProjectData) -> Result<(Tick, Tick, Tick)> {
        let (outgoing, incoming) = self.sides(timeline)?;
        let a = outgoing.last().ok_or(Error::Invalid("transition side"))?;
        let b = incoming.first().ok_or(Error::Invalid("transition side"))?;
        let cut = b.start;
        let mut lo = outgoing
            .first()
            .ok_or(Error::Invalid("transition side"))?
            .start;
        let mut hi = incoming
            .last()
            .ok_or(Error::Invalid("transition side"))?
            .end()?;
        if b.source_asset.is_some() {
            lo = lo.max(cut.sub_tick(b.retime.speed.output_duration(b.source_range.start())?)?);
        }
        if let Some(id) = a.source_asset {
            let available = data
                .assets
                .iter()
                .find(|a| a.id() == id)
                .and_then(|a| a.probe().duration)
                .map(|d| d.sub_tick(a.source_range.end()))
                .transpose()?
                .unwrap_or(Tick::ZERO);
            hi = hi.min(cut.add_tick(a.retime.speed.output_duration(available)?)?);
        }
        Ok((lo, cut, hi))
    }
    /// Clamp a positive requested duration, following alignment; a custom span
    /// resizes about its current centre. A zero-handle cut returns Invalid.
    pub fn set_duration(&mut self, duration: Tick, bounds: (Tick, Tick, Tick)) -> Result<()> {
        if duration <= Tick::ZERO {
            return Err(Error::Invalid("transition duration"));
        }
        let (lo, cut, hi) = bounds;
        let left = cut.sub_tick(lo)?.0;
        let right = hi.sub_tick(cut)?.0;
        let max = match self.alignment {
            TransitionAlignment::Start => right,
            TransitionAlignment::End => left,
            TransitionAlignment::Center => left
                .saturating_mul(2)
                .saturating_add(1)
                .min(right.saturating_mul(2)),
            TransitionAlignment::Custom => hi.sub_tick(lo)?.0,
        };
        let d = duration.0.min(max);
        if d <= 0 {
            return Err(Error::Invalid("transition handles"));
        }
        self.start = match self.alignment {
            TransitionAlignment::Start => cut,
            TransitionAlignment::End => cut.sub_tick(Tick(d))?,
            TransitionAlignment::Center => cut.sub_tick(Tick(d / 2))?,
            TransitionAlignment::Custom => {
                let center = self.start.add_tick(Tick(self.duration.0 / 2))?;
                let desired = center.sub_tick(Tick(d / 2))?;
                desired
                    .max(lo.max(cut.sub_tick(Tick(d))?))
                    .min(cut.min(hi.sub_tick(Tick(d))?))
            }
        };
        self.duration = Tick(d);
        Ok(())
    }
    pub fn validate(&self, timeline: &Timeline, data: &ProjectData) -> Result<()> {
        transition_definition(&self.kind)?;
        let (lo, cut, hi) = self.bounds(timeline, data)?;
        let end = self.end()?;
        if self.duration <= Tick::ZERO
            || self.start < lo
            || self.start > cut
            || end < cut
            || end > hi
        {
            return Err(Error::Invalid("transition span or handles"));
        }
        let aligned = match self.alignment {
            TransitionAlignment::Start => self.start == cut,
            TransitionAlignment::End => end == cut,
            TransitionAlignment::Center => self.start.add_tick(Tick(self.duration.0 / 2))? == cut,
            TransitionAlignment::Custom => true,
        };
        if !aligned {
            return Err(Error::Invalid("transition alignment"));
        }
        Ok(())
    }
}

impl Timeline {
    /// Items on visible tracks that can contribute pixels to `[from, to)`: those
    /// whose own extent overlaps it, plus both clips of every transition whose
    /// span overlaps it, because they render from their handles beyond that extent.
    pub fn rendered_items(&self, from: Tick, to: Tick) -> Vec<&Item> {
        let mut joined = BTreeSet::new();
        for tr in self
            .transitions
            .iter()
            .filter(|tr| tr.start < to && tr.end().is_ok_and(|end| end > from))
        {
            let Ok((a, b)) = tr.sides(self) else { continue };
            let Ok(end) = tr.end() else { continue };
            for side in [a, b] {
                for (n, item) in side.iter().enumerate() {
                    let lo = if n == 0 { tr.start } else { item.start };
                    let hi = if n + 1 == side.len() {
                        end
                    } else {
                        item.end().unwrap_or(item.start)
                    };
                    if lo.max(tr.start) < to && hi.min(end) > from {
                        joined.insert(item.id);
                    }
                }
            }
        }
        self.iter()
            .filter(|t| !t.hidden)
            .flat_map(|t| &t.items)
            .filter(|i| {
                (i.start < to && i.end().is_ok_and(|end| end > from)) || joined.contains(&i.id)
            })
            .collect()
    }
}

trait CheckedTick {
    fn add_tick(self, rhs: Tick) -> Result<Tick>;
    fn sub_tick(self, rhs: Tick) -> Result<Tick>;
}
impl CheckedTick for Tick {
    fn add_tick(self, rhs: Tick) -> Result<Tick> {
        self.checked_add(rhs)
            .ok_or(Error::Invalid("transition time overflow"))
    }
    fn sub_tick(self, rhs: Tick) -> Result<Tick> {
        self.checked_sub(rhs)
            .ok_or(Error::Invalid("transition time overflow"))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]
    use super::*;
    use crate::{Color, ItemParams, OverlayParams, Project, ProjectId, TransitionAlignment::*};
    use time::SourceRange;
    fn data() -> ProjectData {
        let mut d = Project::empty(ProjectId(0), "".into()).unwrap().into_data();
        d.scene_mut().timeline.main.items = [(1, 0), (2, 100)]
            .into_iter()
            .map(|(id, start)| Item {
                retime: Default::default(),
                id: ItemId(id),
                source_asset: None,
                source_range: SourceRange::new(Tick(10), Tick(110)).unwrap(),
                start: Tick(start),
                content: ItemParams::Overlay(OverlayParams {
                    color: Color([255; 4]),
                }),
                animations: Default::default(),
                effects: vec![],
                masks: vec![],
                blend_mode: Default::default(),
            })
            .collect();
        d
    }
    fn transition() -> Transition {
        Transition {
            from_pieces: vec![],
            to_pieces: vec![],
            id: TransitionId(1),
            track: TrackId(0),
            from: ItemId(1),
            to: ItemId(2),
            kind: "cross_dissolve".into(),
            start: Tick(80),
            duration: Tick(40),
            alignment: Center,
        }
    }
    #[test]
    fn alignment_bounds_clamping_and_hostile_model_spans() {
        let d = data();
        for (alignment, start, duration) in [
            (Start, 100, 100),
            (Center, 0, 200),
            (End, 0, 100),
            (Custom, 0, 200),
        ] {
            let mut tr = transition();
            tr.alignment = alignment;
            tr.set_duration(Tick(i64::MAX), tr.bounds(&d.scene().timeline, &d).unwrap())
                .unwrap();
            assert_eq!((tr.start, tr.duration), (Tick(start), Tick(duration)));
            tr.validate(&d.scene().timeline, &d).unwrap();
        }
        for (start, duration, alignment) in [
            (-1, 40, Custom),
            (80, 0, Custom),
            (80, -1, Custom),
            (110, 1, Custom),
            (i64::MAX, i64::MAX, Custom),
            (80, 40, Start),
            (80, 40, End),
            (80, 41, Center),
        ] {
            let mut tr = transition();
            tr.start = Tick(start);
            tr.duration = Tick(duration);
            tr.alignment = alignment;
            // An odd centered span uses floor(duration/2) before the cut.
            if (start, duration) == (80, 41) {
                tr.validate(&d.scene().timeline, &d).unwrap();
            } else {
                assert!(tr.validate(&d.scene().timeline, &d).is_err());
            }
        }
        let mut d = d;
        d.scene_mut().timeline.transitions = vec![transition(), transition()];
        assert!(Project::new(d.clone()).is_err());
        d.scene_mut().timeline.transitions[1].id = TransitionId(2);
        assert!(Project::new(d.clone()).is_err());
        // Nonoverlapping end-at-cut and start-at-cut spans still name the same
        // cut: allowing both would jump from B back to A at their shared edge.
        d.scene_mut().timeline.transitions[0].start = Tick(60);
        d.scene_mut().timeline.transitions[0].alignment = End;
        d.scene_mut().timeline.transitions[1].start = Tick(100);
        d.scene_mut().timeline.transitions[1].alignment = Start;
        assert!(matches!(
            Project::new(d.clone()),
            Err(Error::Invalid("duplicate transition cut"))
        ));
        d.scene_mut().timeline.main.items.pop();
        assert!(Project::new(d).is_err());
        let mut d = data();
        let mut third = d.scene().timeline.main.items[1].clone();
        third.id = ItemId(3);
        third.start = Tick(200);
        d.scene_mut().timeline.main.items.push(third);
        let mut a = transition();
        a.start = Tick(20);
        a.duration = Tick(160);
        let mut b = transition();
        b.id = TransitionId(2);
        b.from = ItemId(2);
        b.to = ItemId(3);
        b.start = Tick(150);
        b.duration = Tick(50);
        b.alignment = End;
        d.scene_mut().timeline.transitions = vec![a, b];
        assert!(matches!(
            Project::new(d),
            Err(Error::Invalid("overlapping transitions"))
        ));
    }
    #[test]
    fn rendered_items_include_both_clips_of_an_overlapping_span() {
        let mut d = data();
        d.scene_mut().timeline.transitions = vec![transition()]; // span 80..120 over the cut at 100
        let ids = |d: &ProjectData, from: i64, to: i64| -> Vec<u64> {
            d.scene()
                .timeline
                .rendered_items(Tick(from), Tick(to))
                .iter()
                .map(|i| i.id.0)
                .collect()
        };
        assert_eq!(ids(&d, 70, 80), [1]); // span start is exclusive of an earlier range
        assert_eq!(ids(&d, 79, 81), [1, 2]); // incoming clip renders before its own start
        assert_eq!(ids(&d, 119, 120), [1, 2]); // outgoing clip renders past its own end
        assert_eq!(ids(&d, 120, 130), [2]);
        let mut split = d.clone();
        let mut right = split.scene().timeline.main.items[0].clone();
        right.id = ItemId(3);
        right.start = Tick(90);
        right.source_range = SourceRange::new(Tick(100), Tick(110)).unwrap();
        split.scene_mut().timeline.main.items[0].source_range =
            SourceRange::new(Tick(10), Tick(100)).unwrap();
        split.scene_mut().timeline.main.items.insert(1, right);
        split.scene_mut().timeline.transitions[0].from = ItemId(3);
        split.scene_mut().timeline.transitions[0].from_pieces = vec![ItemId(1)];
        Project::new(split.clone()).unwrap();
        for (from, to, expected) in [
            (79, 81, vec![1, 2]),
            (89, 91, vec![1, 3, 2]),
            (100, 110, vec![3, 2]),
            (119, 120, vec![3, 2]),
        ] {
            assert_eq!(
                ids(&split, from, to),
                expected,
                "only pieces that contribute to this range"
            );
        }
        d.scene_mut().timeline.main.hidden = true;
        assert!(ids(&d, 79, 121).is_empty());
    }
    #[test]
    fn media_handles_bound_both_sides_and_zero_handles_refuse() {
        let mut d = data();
        d.assets.push(
            media::SourceAsset::try_from(media::AssetData {
                ephemeral: false,
                id: media::AssetId(1),
                path: std::env::temp_dir().join("transition-handles"),
                content_hash: media::ContentHash([0; 32]),
                probe: media::SourceProbe {
                    format: "synthetic".into(),
                    duration: Some(Tick(117)),
                    streams: vec![media::SourceStream {
                        index: 0,
                        kind: media::StreamKind::Video,
                        codec: "rgba".into(),
                        time_base: media::TimeBase::new(1, 30).unwrap(),
                        start: None,
                        duration: Some(Tick(117)),
                        frame_rate: Some(time::FrameRate::FPS_30),
                        frame_count: None,
                        variable_frame_rate: None,
                        video: Some(media::VideoInfo {
                            width: 4,
                            height: 4,
                            color: media::ColorInfo::default(),
                            bit_depth: Some(8),
                        }),
                        audio: None,
                    }],
                },
            })
            .unwrap(),
        );
        for i in &mut d.scene_mut().timeline.main.items {
            i.source_asset = Some(media::AssetId(1));
            i.content = ItemParams::Video(crate::VideoParams {
                source_audio_enabled: false,
                volume: 0.,
                muted: false,
                fades: Default::default(),
            });
        }
        for (speed, cut, expected) in [
            (0.5, 200, (180, 200, 214)),
            (2.0, 50, (45, 50, 53)),
            (5.0, 20, (18, 20, 21)),
        ] {
            let mut retimed = d.clone();
            for i in &mut retimed.scene_mut().timeline.main.items {
                i.retime.speed = time::Speed::from_f64(speed).unwrap();
            }
            retimed.scene_mut().timeline.main.items[1].start = Tick(cut);
            assert_eq!(
                transition()
                    .bounds(&retimed.scene().timeline, &retimed)
                    .unwrap(),
                (Tick(expected.0), Tick(expected.1), Tick(expected.2)),
            );
        }
        let mut tr = transition();
        let bounds = tr.bounds(&d.scene().timeline, &d).unwrap();
        assert_eq!(bounds, (Tick(90), Tick(100), Tick(107)));
        for (align, start, duration) in [(Start, 100, 7), (Center, 93, 14), (End, 90, 10)] {
            tr.alignment = align;
            tr.set_duration(Tick(100), bounds).unwrap();
            assert_eq!((tr.start, tr.duration), (Tick(start), Tick(duration)));
        }
        d.scene_mut().timeline.main.items[1].source_range =
            SourceRange::new(Tick(0), Tick(100)).unwrap();
        tr.alignment = End;
        assert!(
            tr.set_duration(Tick(20), tr.bounds(&d.scene().timeline, &d).unwrap())
                .is_err()
        );
    }
}

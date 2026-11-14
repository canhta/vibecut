//! Compound clips: items whose source is a nested timeline of their members.
use super::item::{Item, ItemId, ItemParams, VideoParams};
use super::track::{Timeline, Track, TrackId, TrackKind};
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use time::{SourceRange, Tick};

/// Deepest nesting of compound clips inside compound clips, the outermost counting as one.
pub const MAX_COMPOUND_DEPTH: usize = 8;

/// A compound clip's source and its own clip settings.
///
/// The nested timeline is the source: its members in source ticks, rendered into one isolated
/// frame and one mix. The item's `source_range` and `retime` window that source like a video
/// clip's; `clip` holds the same picture and sound settings a video clip has (volume, mute,
/// fades, cleanup, crop, fit, background). Settings that read source analysis (stabilize,
/// background removal, matte strokes, flicker, noise reduction) stay off. Member, track and
/// transition ids are unique across the whole project, so releasing the members keeps them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CompoundParams {
    pub timeline: Timeline,
    pub clip: VideoParams,
}

impl CompoundParams {
    /// The clip settings a new compound clip starts with: a plain video clip with its sound on.
    pub fn plain_clip() -> VideoParams {
        VideoParams {
            source_audio_enabled: true,
            ..VideoParams::plain()
        }
    }
}

impl Item {
    /// A compound clip at output `start` owning `timeline`: its window is the whole nested
    /// content, `[0, end of the last member)`, at normal speed with no look of its own.
    ///
    /// # Errors
    ///
    /// `Invalid` when the nested timeline holds no member.
    pub fn compound(id: ItemId, start: Tick, timeline: Timeline) -> Result<Self> {
        let end = content_end(&timeline)?;
        if end <= Tick::ZERO {
            return Err(Error::Invalid("empty compound clip"));
        }
        Ok(Self {
            blend_mode: crate::BlendMode::Normal,
            masks: Vec::new(),
            effects: Vec::new(),
            animations: crate::Animations::default(),
            id,
            group: None,
            hidden: false,
            source_asset: None,
            source_range: SourceRange::new(Tick::ZERO, end)?,
            start,
            retime: crate::Retime::default(),
            content: ItemParams::Compound(Box::new(CompoundParams {
                timeline,
                clip: CompoundParams::plain_clip(),
            })),
        })
    }
    /// Whether this compound clip carries edits of its own that releasing its members would
    /// lose: a window other than the whole content, a retime, a look, or changed clip settings.
    /// `false` for every other item kind.
    pub fn compound_has_own_edits(&self) -> Result<bool> {
        let ItemParams::Compound(params) = &self.content else {
            return Ok(false);
        };
        let whole = SourceRange::new(Tick::ZERO, content_end(&params.timeline)?)?;
        Ok(self.source_range != whole
            || self.retime != crate::Retime::default()
            || !self.effects.is_empty()
            || !self.masks.is_empty()
            || self.animations.iter().next().is_some()
            || self.blend_mode != crate::BlendMode::Normal
            || params.clip != CompoundParams::plain_clip())
    }
}

impl Timeline {
    /// This timeline, then depth first every timeline its compound clips own.
    pub fn timelines(&self) -> Vec<&Timeline> {
        let mut out = Vec::new();
        let mut stack = vec![self];
        while let Some(timeline) = stack.pop() {
            out.push(timeline);
            for item in timeline.iter().flat_map(|track| &track.items) {
                if let ItemParams::Compound(params) = &item.content {
                    stack.push(&params.timeline);
                }
            }
        }
        out
    }
    /// Every track of this timeline and of the compound clips nested in it; whole-project
    /// readers (asset use, ids, corrections, fonts) walk these instead of `iter`.
    pub fn all_tracks(&self) -> Vec<&Track> {
        self.timelines()
            .into_iter()
            .flat_map(|timeline| timeline.iter())
            .collect()
    }
    /// End of the last item, in this timeline's own ticks; zero when it is empty.
    ///
    /// # Errors
    ///
    /// An item end that overflows.
    pub fn content_end(&self) -> Result<Tick> {
        content_end(self)
    }
}

fn content_end(timeline: &Timeline) -> Result<Tick> {
    let mut end = Tick::ZERO;
    for item in timeline.iter().flat_map(|track| &track.items) {
        end = end.max(item.end()?);
    }
    Ok(end)
}

/// Deepest compound nesting under `timeline`; a timeline without compound clips is 0.
fn depth(timeline: &Timeline) -> usize {
    let mut deepest = 0;
    let mut stack = vec![(timeline, 0_usize)];
    while let Some((timeline, level)) = stack.pop() {
        deepest = deepest.max(level);
        if level > MAX_COMPOUND_DEPTH {
            break;
        }
        for item in timeline.iter().flat_map(|track| &track.items) {
            if let ItemParams::Compound(params) = &item.content {
                stack.push((&params.timeline, level.saturating_add(1)));
            }
        }
    }
    deepest
}

/// Nesting bound of one scene timeline.
pub(super) fn validate_depth(timeline: &Timeline) -> Result<()> {
    if depth(timeline) > MAX_COMPOUND_DEPTH {
        return Err(Error::TooLarge("compound nesting"));
    }
    Ok(())
}

/// The rules a compound clip item adds to ordinary item validation: no media reference, a
/// window inside the nested content, clip settings that need no source analysis, and no
/// group inside it.
pub(super) fn validate_item(item: &Item, params: &CompoundParams) -> Result<()> {
    if item.source_asset.is_some() {
        return Err(Error::Invalid("compound clip references media"));
    }
    let end = content_end(&params.timeline)?;
    if end <= Tick::ZERO
        || item.source_range.end() > end
        || item
            .retime
            .window
            .is_some_and(|w| w.source_range.end() > end)
    {
        return Err(Error::Invalid("compound clip window"));
    }
    let clip = &params.clip;
    if clip.stabilize.is_some()
        || clip.background_removal.is_some()
        || !clip.matte_strokes.is_empty()
        || clip.remove_flicker
        || clip.reduce_noise
    {
        return Err(Error::Invalid("compound clip source analysis"));
    }
    if params
        .timeline
        .all_tracks()
        .iter()
        .flat_map(|track| &track.items)
        .any(|member| member.group.is_some())
    {
        return Err(Error::Invalid("group inside a compound clip"));
    }
    Ok(())
}

/// Put compound clip `id` of `timeline` back as its members, at their output times.
///
/// A member at nested time `t` lands at `start - source_range.start + t`: the members keep
/// their own timing, so a caller must refuse a clip with its own edits
/// ([`Item::compound_has_own_edits`]) instead of losing them. The members of the lowest nested
/// visual track that holds any join the compound's own track when that track is a video track
/// with the same hidden and muted state; every other nested track that holds a member becomes a
/// track of `timeline`, visual ones directly above the compound's track and audio ones at the
/// top of the audio group, in their nested order. A hidden compound hides its visual members
/// and a hidden or muted host track passes that state to the tracks it releases. Nested
/// transitions follow their clips. Every id is kept.
///
/// # Errors
///
/// `Invalid` when `id` is not a compound clip of `timeline`; time overflow.
pub fn release_compound(timeline: &mut Timeline, id: ItemId) -> Result<()> {
    let (host_id, host_hidden, host_muted) = timeline
        .iter()
        .find(|track| {
            track
                .items
                .iter()
                .any(|item| item.id == id && matches!(item.content, ItemParams::Compound(_)))
        })
        .map(|track| (track.id, track.hidden, track.muted))
        .ok_or(Error::Invalid("compound clip not found"))?;
    let host_track = timeline
        .iter_mut()
        .find(|track| track.id == host_id)
        .ok_or(Error::Invalid("compound clip not found"))?;
    let index = host_track
        .items
        .iter()
        .position(|item| item.id == id)
        .ok_or(Error::Invalid("compound clip not found"))?;
    let compound = host_track.items.remove(index);
    let ItemParams::Compound(params) = compound.content else {
        return Err(Error::Invalid("compound clip not found"));
    };
    let mut nested = params.timeline;
    let shift = |at: Tick| {
        at.checked_add(compound.start)
            .and_then(|at| at.checked_sub(compound.source_range.start()))
            .ok_or(Error::Invalid("compound member time"))
    };
    for track in nested.iter_mut() {
        track.hidden |= host_hidden;
        track.muted |= host_muted && track.kind.can_mute();
        for item in &mut track.items {
            item.start = shift(item.start)?;
            if compound.hidden && track.kind != TrackKind::Audio {
                item.hidden = true;
            }
        }
    }
    for transition in &mut nested.transitions {
        transition.start = shift(transition.start)?;
    }
    // Nested visual tracks bottom to top; the first is the merge candidate.
    let mut visual: Vec<Track> = std::iter::once(nested.main)
        .chain(nested.overlays.into_iter().rev())
        .filter(|track| !track.items.is_empty())
        .collect();
    let host_video = timeline
        .iter()
        .find(|track| track.id == host_id)
        .is_some_and(|track| track.kind == TrackKind::Video);
    let merge = visual.first().is_some_and(|track| {
        host_video
            && track.kind == TrackKind::Video
            && track.hidden == host_hidden
            && track.muted == host_muted
    });
    let mut renamed = BTreeMap::new();
    if merge && !visual.is_empty() {
        let lowest = visual.remove(0);
        renamed.insert(lowest.id, host_id);
        let host_track = timeline
            .iter_mut()
            .find(|track| track.id == host_id)
            .ok_or(Error::Invalid("compound clip not found"))?;
        host_track.items.extend(lowest.items);
        host_track.items.sort_by_key(|item| item.start);
    }
    // `overlays` is topmost first: the released tracks go directly above the host.
    let at = if timeline.main.id == host_id {
        timeline.overlays.len()
    } else {
        timeline
            .overlays
            .iter()
            .position(|track| track.id == host_id)
            .ok_or(Error::Invalid("compound clip not found"))?
    };
    for track in visual {
        timeline.overlays.insert(at, track);
    }
    let audio: Vec<Track> = nested
        .audio
        .into_iter()
        .filter(|track| !track.items.is_empty())
        .collect();
    for (n, track) in audio.into_iter().enumerate() {
        timeline.audio.insert(n, track);
    }
    for mut transition in nested.transitions {
        if let Some(track) = renamed.get(&transition.track) {
            transition.track = *track;
        }
        timeline.transitions.push(transition);
    }
    Ok(())
}

impl CompoundParams {
    /// Give every nested track, member and transition, at any depth, a fresh id from `fresh`,
    /// so a copy or a split piece of a compound clip can live beside the original.
    ///
    /// # Errors
    ///
    /// The first error `fresh` returns; ids already replaced stay replaced.
    pub fn renumber<E>(
        &mut self,
        fresh: &mut impl FnMut() -> std::result::Result<u64, E>,
    ) -> std::result::Result<(), E> {
        let mut stack = vec![&mut self.timeline];
        while let Some(timeline) = stack.pop() {
            let mut tracks = BTreeMap::new();
            let mut items = BTreeMap::new();
            for track in timeline.iter_mut() {
                let id = TrackId(fresh()?);
                tracks.insert(track.id, id);
                track.id = id;
                for item in &mut track.items {
                    let id = ItemId(fresh()?);
                    items.insert(item.id, id);
                    item.id = id;
                }
            }
            for transition in &mut timeline.transitions {
                transition.id = crate::TransitionId(fresh()?);
                let rename = |id: &mut ItemId| {
                    if let Some(new) = items.get(id) {
                        *id = *new;
                    }
                };
                rename(&mut transition.from);
                rename(&mut transition.to);
                transition.from_pieces.iter_mut().for_each(rename);
                transition.to_pieces.iter_mut().for_each(rename);
                if let Some(new) = tracks.get(&transition.track) {
                    transition.track = *new;
                }
            }
            for item in timeline.iter_mut().flat_map(|track| track.items.iter_mut()) {
                if let ItemParams::Compound(params) = &mut item.content {
                    stack.push(&mut params.timeline);
                }
            }
        }
        Ok(())
    }
}

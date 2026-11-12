//! Transfers bounded snapshots through the in-app clipboard.
use crate::KeyframeRef;
use crate::session::{EditError, Result};
use project::{AnimationPath, Channel, ColorComponent, Param, ProjectData};
use serde::{Deserialize, Serialize, ser::SerializeSeq};
use std::collections::{BTreeMap, BTreeSet};
use time::Tick;

pub const MAX_CLIPBOARD_KEYS: usize = 4096;
pub const MAX_CLIPBOARD_PATHS: usize = 64;
pub const MAX_CLIPBOARD_BYTES: usize = 1 << 20;
pub const MAX_CLIPBOARD_ITEMS: usize = 1024;
pub const MAX_ITEM_CLIPBOARD_BYTES: usize = 4 << 20;

#[derive(Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct CopiedParam {
    path: AnimationPath,
    param: Param,
}
#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct Payload {
    #[schemars(range(min = 1, max = 1))]
    version: u8,
    #[schemars(range(min = -9007199254740991i64, max = 9007199254740991i64))]
    origin: Tick,
    #[schemars(length(min = 1, max = 64))]
    #[serde(deserialize_with = "entries")]
    params: Vec<CopiedParam>,
}
fn entries<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<Vec<CopiedParam>, D::Error> {
    struct Entries;
    impl<'de> serde::de::Visitor<'de> for Entries {
        type Value = Vec<CopiedParam>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("at most 64 copied parameters")
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(
            self,
            mut a: A,
        ) -> std::result::Result<Self::Value, A::Error> {
            let mut result = Vec::new();
            while let Some(entry) = a.next_element()? {
                if result.len() == MAX_CLIPBOARD_PATHS {
                    return Err(serde::de::Error::custom("clipboard path limit"));
                }
                result.push(entry);
            }
            Ok(result)
        }
    }
    d.deserialize_seq(Entries)
}
/// Version-one immutable snapshot; at most 64 paths, 4096 keys and 1 MiB compact JSON.
/// Times and handle deltas fit exact JSON integers. Construction refuses invalid Params,
/// unordered paths, empty selections, a nonminimal origin and overflowing offsets.
#[derive(Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(try_from = "Payload")]
pub struct KeyframeClipboard {
    #[schemars(range(min = 1, max = 1))]
    version: u8,
    #[schemars(range(min = -9007199254740991i64, max = 9007199254740991i64))]
    origin: Tick,
    #[schemars(length(min = 1, max = 64))]
    params: Vec<CopiedParam>,
}
impl std::fmt::Debug for KeyframeClipboard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeyframeClipboard")
            .field("paths", &self.params.len())
            .finish_non_exhaustive()
    }
}
impl TryFrom<Payload> for KeyframeClipboard {
    type Error = EditError;
    fn try_from(value: Payload) -> Result<Self> {
        if value.version != 1 {
            return Err(EditError::Control(
                crate::session::wire::ControlError::InvalidParams,
            ));
        }
        Self::new(
            value.origin,
            value
                .params
                .into_iter()
                .map(|p| (p.path, p.param))
                .collect(),
        )
    }
}
/// Stored sources share the Session snapshot; explicit payloads travel only over control IO.
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum KeyframePasteSource {
    Stored,
    Payload { payload: KeyframeClipboard },
}
fn channels(param: &Param) -> &[Channel] {
    match param {
        Param::Value(c) => std::slice::from_ref(c),
        Param::Color(c) => c.as_ref(),
    }
}
fn channels_mut(param: &mut Param) -> &mut [Channel] {
    match param {
        Param::Value(c) => std::slice::from_mut(c),
        Param::Color(c) => c.as_mut(),
    }
}
fn exact(time: Tick) -> Result<Tick> {
    if !(-super::wire::MAX_SAFE_INTEGER..=super::wire::MAX_SAFE_INTEGER).contains(&time.0) {
        return Err(EditError::Control(
            crate::session::wire::ControlError::InvalidParams,
        ));
    }
    Ok(time)
}
// A projection owns only bounded reference metadata. Source Params are already validated
// by Project; removing keys preserves their value, curve, Color and path invariants.
struct Selection<'a> {
    origin: Tick,
    paths: BTreeMap<&'a AnimationPath, &'a Param>,
    keys: BTreeSet<(&'a AnimationPath, Option<usize>, Tick)>,
}
impl<'a> Selection<'a> {
    fn admit(data: &'a ProjectData, refs: &'a [KeyframeRef]) -> Result<Option<Self>> {
        if refs.len() > MAX_CLIPBOARD_KEYS {
            return Err(EditError::Control(
                crate::session::wire::ControlError::Limit,
            ));
        }
        let Some(first) = refs.first() else {
            return Ok(None);
        };
        let source = data
            .scene()
            .timeline
            .iter()
            .flat_map(|t| &t.items)
            .find(|i| i.id == first.item)
            .ok_or(EditError::NoItem(first.item))?;
        let mut selected = Self {
            origin: first.time,
            paths: BTreeMap::new(),
            keys: BTreeSet::new(),
        };
        for r in refs {
            if r.item != first.item {
                return Err(EditError::Invalid("copy requires one source item"));
            }
            let component = r.component.map(|c| match c {
                ColorComponent::R => 0,
                ColorComponent::G => 1,
                ColorComponent::B => 2,
                ColorComponent::A => 3,
            });
            if !selected.keys.insert((&r.path, component, r.time)) {
                return Err(EditError::Control(
                    crate::session::wire::ControlError::InvalidParams,
                ));
            }
            let param = source
                .animations
                .get(&r.path)
                .ok_or(EditError::Invalid("missing animated parameter"))?;
            let channel = param.channel(r.component)?;
            let index = channel
                .keys()
                .binary_search_by_key(&r.time, |k| k.time)
                .map_err(|_| EditError::Invalid("stale keyframe selection"))?;
            let key = channel
                .keys()
                .get(index)
                .ok_or(EditError::Invalid("stale keyframe selection"))?;
            exact(key.time)?;
            for handle in [key.incoming, key.outgoing].into_iter().flatten() {
                exact(handle.dt)?;
            }
            selected.paths.insert(&r.path, param);
            if selected.paths.len() > MAX_CLIPBOARD_PATHS {
                return Err(EditError::Control(
                    crate::session::wire::ControlError::Limit,
                ));
            }
            selected.origin = selected.origin.min(r.time);
        }
        for (_, _, time) in &selected.keys {
            exact(time.checked_sub(selected.origin).ok_or(EditError::Control(
                crate::session::wire::ControlError::InvalidParams,
            ))?)?;
        }
        // Stream borrowed bases and selected keys through the real JSON escaping rules.
        // No text/key/base is cloned, and no serialized payload buffer is allocated.
        compact_size(&selected)?;
        Ok(Some(selected))
    }
}
struct SelectedKeys<'a> {
    selection: &'a Selection<'a>,
    path: &'a AnimationPath,
    component: Option<usize>,
    channel: &'a Channel,
}
impl Serialize for SelectedKeys<'_> {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(None)?;
        for key in self.channel.keys() {
            if self
                .selection
                .keys
                .contains(&(self.path, self.component, key.time))
            {
                seq.serialize_element(&KeyProjection::from(key))?;
            }
        }
        seq.end()
    }
}
// NfcText's ordinary Serialize uses `into = String`, which clones its owned
// string. Borrow its normalized str here so even one hostile base is never copied.
struct ValueProjection<'a>(&'a project::ParamValue);
impl Serialize for ValueProjection<'_> {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        if let project::ParamValue::Text(text) = self.0 {
            use serde::ser::SerializeStruct;
            let mut value = serializer.serialize_struct("ParamValue", 2)?;
            value.serialize_field("kind", "text")?;
            value.serialize_field("value", text.as_str())?;
            value.end()
        } else {
            self.0.serialize(serializer)
        }
    }
}
#[derive(Serialize)]
struct KeyProjection<'a> {
    time: Tick,
    value: ValueProjection<'a>,
    segment: project::Segment,
    tangent: project::TangentMode,
    incoming: Option<project::CurveHandle>,
    outgoing: Option<project::CurveHandle>,
}
impl<'a> From<&'a project::Keyframe> for KeyProjection<'a> {
    fn from(key: &'a project::Keyframe) -> Self {
        Self {
            time: key.time,
            value: ValueProjection(&key.value),
            segment: key.segment,
            tangent: key.tangent,
            incoming: key.incoming,
            outgoing: key.outgoing,
        }
    }
}
#[derive(Serialize)]
struct ChannelProjection<'a> {
    value: ValueProjection<'a>,
    keys: SelectedKeys<'a>,
    before: project::Extrapolation,
    after: project::Extrapolation,
}
impl<'a> ChannelProjection<'a> {
    fn new(
        selection: &'a Selection<'a>,
        path: &'a AnimationPath,
        channel: &'a Channel,
        component: Option<usize>,
    ) -> Self {
        let (before, after) = channel.extrapolation();
        Self {
            value: ValueProjection(channel.base()),
            keys: SelectedKeys {
                selection,
                path,
                component,
                channel,
            },
            before,
            after,
        }
    }
}
#[derive(Serialize)]
#[serde(tag = "kind", content = "channels", rename_all = "snake_case")]
enum ParamProjection<'a> {
    Value(ChannelProjection<'a>),
    Color([ChannelProjection<'a>; 4]),
}
struct SelectedParams<'a>(&'a Selection<'a>);
impl Serialize for SelectedParams<'_> {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Entry<'a> {
            path: &'a AnimationPath,
            param: ParamProjection<'a>,
        }
        let mut seq = serializer.serialize_seq(Some(self.0.paths.len()))?;
        for (path, param) in &self.0.paths {
            let param = match param {
                Param::Value(c) => {
                    ParamProjection::Value(ChannelProjection::new(self.0, path, c, None))
                }
                Param::Color(c) => {
                    let channels: &[project::Channel; 4] = c.as_ref();
                    let [c0, c1, c2, c3] = channels;
                    ParamProjection::Color([
                        ChannelProjection::new(self.0, path, c0, Some(0)),
                        ChannelProjection::new(self.0, path, c1, Some(1)),
                        ChannelProjection::new(self.0, path, c2, Some(2)),
                        ChannelProjection::new(self.0, path, c3, Some(3)),
                    ])
                }
            };
            seq.serialize_element(&Entry { path, param })?;
        }
        seq.end()
    }
}
impl Serialize for Selection<'_> {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut value = serializer.serialize_struct("KeyframeClipboard", 3)?;
        value.serialize_field("version", &1u8)?;
        value.serialize_field("origin", &self.origin)?;
        value.serialize_field("params", &SelectedParams(self))?;
        value.end()
    }
}
fn compact_size(value: &impl Serialize) -> Result<usize> {
    bounded_size(value, MAX_CLIPBOARD_BYTES)
}
fn bounded_size(value: &impl Serialize, limit: usize) -> Result<usize> {
    struct Counter {
        bytes: usize,
        limit: usize,
    }
    impl std::io::Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > self.limit - self.bytes {
                return Err(std::io::Error::other("clipboard byte limit"));
            }
            self.bytes += bytes.len();
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut counter = Counter { bytes: 0, limit };
    serde_json::to_writer(&mut counter, value)
        .map_err(|_| EditError::Control(crate::session::wire::ControlError::Limit))?;
    Ok(counter.bytes)
}
impl KeyframeClipboard {
    pub fn new(origin: Tick, params: Vec<(AnimationPath, Param)>) -> Result<Self> {
        if params.len() > MAX_CLIPBOARD_PATHS {
            return Err(EditError::Control(
                crate::session::wire::ControlError::Limit,
            ));
        }
        exact(origin)?;
        let mut previous = None;
        let mut minimum = None;
        let mut count = 0usize;
        for (path, param) in &params {
            if previous.is_some_and(|p| p >= path) {
                return Err(EditError::Control(
                    crate::session::wire::ControlError::InvalidParams,
                ));
            }
            previous = Some(path);
            let mut participating = false;
            for c in channels(param) {
                count = count.checked_add(c.keys().len()).ok_or(EditError::Control(
                    crate::session::wire::ControlError::Limit,
                ))?;
                if count > MAX_CLIPBOARD_KEYS {
                    return Err(EditError::Control(
                        crate::session::wire::ControlError::Limit,
                    ));
                }
                for key in c.keys() {
                    participating = true;
                    exact(key.time)?;
                    let offset = exact(key.time.checked_sub(origin).ok_or(EditError::Control(
                        crate::session::wire::ControlError::InvalidParams,
                    ))?)?;
                    if offset < Tick::ZERO {
                        return Err(EditError::Control(
                            crate::session::wire::ControlError::InvalidParams,
                        ));
                    }
                    for handle in [key.incoming, key.outgoing].into_iter().flatten() {
                        exact(handle.dt)?;
                    }
                    minimum = Some(minimum.map_or(key.time, |t: Tick| t.min(key.time)));
                }
            }
            if !participating {
                return Err(EditError::Control(
                    crate::session::wire::ControlError::InvalidParams,
                ));
            }
        }
        if minimum != Some(origin) {
            return Err(EditError::Control(
                crate::session::wire::ControlError::InvalidParams,
            ));
        }
        let value = Self {
            version: 1,
            origin,
            params: params
                .into_iter()
                .map(|(path, param)| CopiedParam { path, param })
                .collect(),
        };
        value.compact_bytes()?;
        // The ordinary animation owner validates typed Color and path/value combinations too.
        let mut admitted = project::Animations::default();
        for p in &value.params {
            admitted.set(p.path.clone(), p.param.clone())?;
        }
        Ok(value)
    }
    pub(super) fn same_bytes(&self, other: &Self) -> Result<bool> {
        Ok(super::wire::bounded_json(self, MAX_CLIPBOARD_BYTES)?
            == super::wire::bounded_json(other, MAX_CLIPBOARD_BYTES)?)
    }
    pub(super) fn compact_bytes(&self) -> Result<usize> {
        compact_size(self)
    }
    pub(super) fn can_capture(data: &ProjectData, refs: &[KeyframeRef]) -> bool {
        Selection::admit(data, refs).is_ok()
    }
    pub(super) fn capture(data: &ProjectData, refs: &[KeyframeRef]) -> Result<Option<Self>> {
        let Some(selected) = Selection::admit(data, refs)? else {
            return Ok(None);
        };
        let Selection {
            origin,
            paths,
            keys: unique,
        } = selected;
        let mut params = Vec::new();
        for (path, param) in paths {
            let copy = |c: &Channel, selector| -> Result<Channel> {
                let keys = c
                    .keys()
                    .iter()
                    .filter(|k| unique.contains(&(path, selector, k.time)))
                    .cloned()
                    .collect();
                let (before, after) = c.extrapolation();
                Ok(Channel::new(c.base().clone(), keys, before, after)?)
            };
            let param = match param {
                Param::Value(c) => Param::Value(copy(c, None)?),
                Param::Color(c) => {
                    let [r, g, b, a] = c.as_ref();
                    Param::Color(Box::new([
                        copy(r, Some(0))?,
                        copy(g, Some(1))?,
                        copy(b, Some(2))?,
                        copy(a, Some(3))?,
                    ]))
                }
            };
            params.push((path.clone(), param));
        }
        Self::new(origin, params).map(Some)
    }
    pub(super) fn admit_paste(&self, target: &project::Item, at: Tick) -> Result<Tick> {
        if at < Tick::ZERO {
            return Err(EditError::Control(
                crate::session::wire::ControlError::InvalidParams,
            ));
        }
        exact(at)?;
        let anchor = exact(at.checked_sub(target.start).ok_or(EditError::Control(
            crate::session::wire::ControlError::InvalidParams,
        ))?)?;
        for p in &self.params {
            let destination = target
                .animations
                .get(&p.path)
                .ok_or(project::Error::Invalid("missing animated parameter"))?;
            if std::mem::discriminant(destination) != std::mem::discriminant(&p.param) {
                return Err(EditError::Invalid("clipboard parameter type"));
            }
            for (source, dest) in channels(&p.param).iter().zip(channels(destination)) {
                if source.keys().is_empty() {
                    continue;
                }
                if std::mem::discriminant(source.base()) != std::mem::discriminant(dest.base()) {
                    return Err(EditError::Invalid("clipboard value type"));
                }
                let mut count = dest.keys().len();
                for key in source.keys() {
                    let time = self.pasted_time(anchor, key.time)?;
                    if dest.keys().binary_search_by_key(&time, |k| k.time).is_err() {
                        count = count.checked_add(1).ok_or(EditError::Control(
                            crate::session::wire::ControlError::Limit,
                        ))?;
                    }
                }
                if count > project::MAX_CHANNEL_KEYS {
                    return Err(project::Error::TooLarge("channel keys").into());
                }
            }
        }
        Ok(anchor)
    }
    fn pasted_time(&self, anchor: Tick, time: Tick) -> Result<Tick> {
        let offset = time.checked_sub(self.origin).ok_or(EditError::Control(
            crate::session::wire::ControlError::InvalidParams,
        ))?;
        exact(anchor.checked_add(offset).ok_or(EditError::Control(
            crate::session::wire::ControlError::InvalidParams,
        ))?)
    }
    pub(super) fn paste(&self, target: &mut project::Item, at: Tick) -> Result<()> {
        // Availability and the edit share borrowed admission before any target key clone.
        let anchor = self.admit_paste(target, at)?;
        for p in &self.params {
            let destination = target.animations.get_mut(&p.path)?;
            for (source, dest) in channels(&p.param).iter().zip(channels_mut(destination)) {
                if source.keys().is_empty() {
                    continue;
                }
                let mut keys: BTreeMap<_, _> =
                    dest.keys().iter().map(|k| (k.time, k.clone())).collect();
                for key in source.keys() {
                    let time = self.pasted_time(anchor, key.time)?;
                    let mut key = key.clone();
                    key.time = time;
                    keys.insert(time, key);
                }
                let (before, after) = source.extrapolation();
                *dest = Channel::new(
                    source.base().clone(),
                    keys.into_values().collect(),
                    before,
                    after,
                )?;
            }
        }
        Ok(())
    }
}

/// One copied item snapshot with its source track identity and group-relative start.
#[derive(Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct CopiedItem {
    #[serde(rename = "trackKind")]
    kind: project::TrackKind,
    /// Zero-based position among tracks of `kind` in top-to-bottom screen order.
    #[serde(rename = "trackIndex")]
    track: u32,
    /// A full item snapshot with a zero placeholder id and group-relative start.
    item: project::Item,
}
#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct ItemPayload {
    #[schemars(range(min = 1, max = 1))]
    version: u8,
    #[schemars(length(min = 1, max = 1024))]
    items: Vec<CopiedItem>,
    #[schemars(range(min = 0, max = 9007199254740991i64))]
    span: Tick,
}
/// Version-one immutable item snapshot; at most 1024 items and 4 MiB compact JSON.
/// Each item keeps its content, source window, retime, effects, masks and animation
/// parameters with a group-relative start; the payload never carries a source id.
#[derive(Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(try_from = "ItemPayload")]
pub struct ItemClipboard {
    #[schemars(range(min = 1, max = 1))]
    version: u8,
    #[schemars(length(min = 1, max = 1024))]
    items: Vec<CopiedItem>,
    #[schemars(range(min = 0, max = 9007199254740991i64))]
    span: Tick,
}
impl std::fmt::Debug for ItemClipboard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ItemClipboard")
            .field("items", &self.items.len())
            .field("span", &self.span)
            .finish()
    }
}
impl TryFrom<ItemPayload> for ItemClipboard {
    type Error = EditError;
    fn try_from(value: ItemPayload) -> Result<Self> {
        if value.version != 1 {
            return Err(EditError::Control(
                crate::session::wire::ControlError::InvalidParams,
            ));
        }
        let clipboard = Self::assemble(value.items)?;
        if clipboard.span != value.span {
            return Err(EditError::Control(
                crate::session::wire::ControlError::InvalidParams,
            ));
        }
        Ok(clipboard)
    }
}
/// Stored sources share the Session snapshot; explicit payloads travel only over control IO.
#[derive(Clone, Debug, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ItemPasteSource {
    Stored,
    Payload { payload: ItemClipboard },
}
/// Tracks of one kind in top-to-bottom screen order: overlays, main, then audio.
fn kind_tracks(timeline: &project::Timeline, kind: project::TrackKind) -> Vec<&project::Track> {
    let mut tracks: Vec<&project::Track> = timeline
        .overlays
        .iter()
        .filter(|track| track.kind == kind)
        .collect();
    if timeline.main.kind == kind {
        tracks.push(&timeline.main);
    }
    tracks.extend(timeline.audio.iter().filter(|track| track.kind == kind));
    tracks
}
/// Borrowed admission of one copy selection: item count, identity, kinds and
/// exact group offsets, without cloning an item.
struct ItemSelection<'a> {
    origin: Tick,
    entries: Vec<(&'a project::Track, &'a project::Item)>,
}
impl<'a> ItemSelection<'a> {
    fn admit(data: &'a ProjectData, ids: &[project::ItemId]) -> Result<Option<Self>> {
        if ids.len() > MAX_CLIPBOARD_ITEMS {
            return Err(EditError::Control(
                crate::session::wire::ControlError::Limit,
            ));
        }
        if ids.is_empty() {
            return Ok(None);
        }
        let mut seen = BTreeSet::new();
        let mut entries = Vec::with_capacity(ids.len());
        let mut origin = None;
        for id in ids {
            if !seen.insert(*id) {
                return Err(EditError::Invalid("duplicate clipboard item"));
            }
            let (track, item) = data
                .scene()
                .timeline
                .iter()
                .find_map(|track| {
                    track
                        .items
                        .iter()
                        .find(|item| item.id == *id)
                        .map(|item| (track, item))
                })
                .ok_or(EditError::NoItem(*id))?;
            if track.kind != item.content.kind() {
                return Err(EditError::Invalid("clipboard item track kind"));
            }
            exact(item.start)?;
            exact(
                item.start
                    .checked_add(item.duration()?)
                    .ok_or(EditError::Control(
                        crate::session::wire::ControlError::InvalidParams,
                    ))?,
            )?;
            origin = Some(origin.map_or(item.start, |time: Tick| time.min(item.start)));
            entries.push((track, item));
        }
        let Some(origin) = origin else {
            return Ok(None);
        };
        for (_, item) in &entries {
            exact(item.start.checked_sub(origin).ok_or(EditError::Control(
                crate::session::wire::ControlError::InvalidParams,
            ))?)?;
        }
        Ok(Some(Self { origin, entries }))
    }
}
/// One admitted paste destination: resolved track, absolute start and end.
pub(super) struct Destination<'a> {
    pub(super) track: project::TrackId,
    pub(super) start: Tick,
    pub(super) end: Tick,
    entry: &'a CopiedItem,
}
impl Destination<'_> {
    pub(super) fn item(&self, id: project::ItemId) -> project::Item {
        let mut item = self.entry.item.clone();
        item.id = id;
        item.start = self.start;
        item
    }
}
impl ItemClipboard {
    fn assemble(items: Vec<CopiedItem>) -> Result<Self> {
        if items.is_empty() {
            return Err(EditError::Control(
                crate::session::wire::ControlError::InvalidParams,
            ));
        }
        if items.len() > MAX_CLIPBOARD_ITEMS {
            return Err(EditError::Control(
                crate::session::wire::ControlError::Limit,
            ));
        }
        let mut minimum = None;
        let mut span = Tick::ZERO;
        for entry in &items {
            if entry.item.id != project::ItemId(0) || entry.item.start < Tick::ZERO {
                return Err(EditError::Control(
                    crate::session::wire::ControlError::InvalidParams,
                ));
            }
            exact(entry.item.start)?;
            let end = exact(entry.item.start.checked_add(entry.item.duration()?).ok_or(
                EditError::Control(crate::session::wire::ControlError::InvalidParams),
            )?)?;
            span = span.max(end);
            minimum =
                Some(minimum.map_or(entry.item.start, |time: Tick| time.min(entry.item.start)));
        }
        if minimum != Some(Tick::ZERO) {
            return Err(EditError::Control(
                crate::session::wire::ControlError::InvalidParams,
            ));
        }
        let value = Self {
            version: 1,
            items,
            span,
        };
        value.compact_bytes()?;
        Ok(value)
    }
    pub(super) fn same_bytes(&self, other: &Self) -> Result<bool> {
        Ok(bounded_size(self, MAX_ITEM_CLIPBOARD_BYTES)?
            == bounded_size(other, MAX_ITEM_CLIPBOARD_BYTES)?)
    }
    pub(super) fn compact_bytes(&self) -> Result<usize> {
        bounded_size(self, MAX_ITEM_CLIPBOARD_BYTES)
    }
    pub(super) fn span(&self) -> Tick {
        self.span
    }
    pub(super) fn can_capture(data: &ProjectData, ids: &[project::ItemId]) -> bool {
        ItemSelection::admit(data, ids).is_ok()
    }
    pub(super) fn capture(data: &ProjectData, ids: &[project::ItemId]) -> Result<Option<Self>> {
        let Some(selection) = ItemSelection::admit(data, ids)? else {
            return Ok(None);
        };
        let timeline = &data.scene().timeline;
        let mut items = Vec::with_capacity(selection.entries.len());
        for (track, item) in selection.entries {
            let index = kind_tracks(timeline, track.kind)
                .iter()
                .position(|candidate| candidate.id == track.id)
                .ok_or(EditError::Invalid("clipboard track"))?;
            let mut copy = item.clone();
            copy.id = project::ItemId(0);
            copy.start = item
                .start
                .checked_sub(selection.origin)
                .ok_or(EditError::Control(
                    crate::session::wire::ControlError::InvalidParams,
                ))?;
            items.push(CopiedItem {
                kind: track.kind,
                track: u32::try_from(index)
                    .map_err(|_| EditError::Control(crate::session::wire::ControlError::Limit))?,
                item: copy,
            });
        }
        Self::assemble(items).map(Some)
    }
    pub(super) fn can_paste(&self, data: &ProjectData, at: Tick) -> bool {
        self.destinations(data, at).is_ok()
    }
    /// Resolve every copied item to its relative track, or the last track of that
    /// kind, before any edit. A locked target or a kind with no track refuses.
    pub(super) fn destinations(
        &self,
        data: &ProjectData,
        at: Tick,
    ) -> Result<Vec<Destination<'_>>> {
        if at < Tick::ZERO {
            return Err(EditError::Control(
                crate::session::wire::ControlError::InvalidParams,
            ));
        }
        exact(at)?;
        let timeline = &data.scene().timeline;
        let mut destinations = Vec::with_capacity(self.items.len());
        for entry in &self.items {
            if entry.item.id != project::ItemId(0) {
                return Err(EditError::Control(
                    crate::session::wire::ControlError::InvalidParams,
                ));
            }
            if entry.item.content.kind() != entry.kind {
                return Err(EditError::Invalid("clipboard item kind"));
            }
            let tracks = kind_tracks(timeline, entry.kind);
            let index = usize::try_from(entry.track).map_err(|_| {
                EditError::Control(crate::session::wire::ControlError::InvalidParams)
            })?;
            let target = tracks
                .get(index)
                .or_else(|| tracks.last())
                .ok_or(EditError::Invalid("clipboard track kind"))?;
            if target.locked {
                return Err(EditError::Locked(target.id));
            }
            if let Some(asset) = entry.item.source_asset
                && data.assets.iter().all(|candidate| candidate.id() != asset)
            {
                return Err(EditError::NoAsset(asset));
            }
            let start = exact(at.checked_add(entry.item.start).ok_or(EditError::Control(
                crate::session::wire::ControlError::InvalidParams,
            ))?)?;
            let end = exact(start.checked_add(entry.item.duration()?).ok_or(
                EditError::Control(crate::session::wire::ControlError::InvalidParams),
            )?)?;
            destinations.push(Destination {
                track: target.id,
                start,
                end,
                entry,
            });
        }
        Ok(destinations)
    }
}

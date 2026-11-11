//! Transfers bounded snapshots of selected item-local animation keys.
use crate::{EditError, KeyframeRef, Result};
use project::{AnimationPath, Channel, ColorComponent, Param, ProjectData};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use time::Tick;

pub const MAX_CLIPBOARD_KEYS: usize = 4096;
pub const MAX_CLIPBOARD_PATHS: usize = 64;
pub const MAX_CLIPBOARD_BYTES: usize = 1 << 20;

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
            return Err(EditError::InvalidParams);
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
        return Err(EditError::InvalidParams);
    }
    Ok(time)
}
impl KeyframeClipboard {
    pub fn new(origin: Tick, params: Vec<(AnimationPath, Param)>) -> Result<Self> {
        if params.len() > MAX_CLIPBOARD_PATHS {
            return Err(EditError::Limit);
        }
        exact(origin)?;
        let mut previous = None;
        let mut minimum = None;
        let mut count = 0usize;
        for (path, param) in &params {
            if previous.is_some_and(|p| p >= path) {
                return Err(EditError::InvalidParams);
            }
            previous = Some(path);
            let mut participating = false;
            for c in channels(param) {
                count = count.checked_add(c.keys().len()).ok_or(EditError::Limit)?;
                if count > MAX_CLIPBOARD_KEYS {
                    return Err(EditError::Limit);
                }
                for key in c.keys() {
                    participating = true;
                    exact(key.time)?;
                    let offset = exact(
                        key.time
                            .checked_sub(origin)
                            .ok_or(EditError::InvalidParams)?,
                    )?;
                    if offset < Tick::ZERO {
                        return Err(EditError::InvalidParams);
                    }
                    for handle in [key.incoming, key.outgoing].into_iter().flatten() {
                        exact(handle.dt)?;
                    }
                    minimum = Some(minimum.map_or(key.time, |t: Tick| t.min(key.time)));
                }
            }
            if !participating {
                return Err(EditError::InvalidParams);
            }
        }
        if minimum != Some(origin) {
            return Err(EditError::InvalidParams);
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
        Ok(super::wire::bounded_json(self, MAX_CLIPBOARD_BYTES)?.len())
    }
    pub(super) fn capture(data: &ProjectData, refs: &[KeyframeRef]) -> Result<Option<Self>> {
        if refs.len() > MAX_CLIPBOARD_KEYS {
            return Err(EditError::Limit);
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
        let mut unique = BTreeSet::new();
        let mut paths = BTreeMap::new();
        let mut origin = first.time;
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
            if !unique.insert((&r.path, component, r.time)) {
                return Err(EditError::InvalidParams);
            }
            let param = source
                .animations
                .get(&r.path)
                .ok_or(EditError::Invalid("missing animated parameter"))?;
            let channel = param.channel(r.component)?;
            if channel
                .keys()
                .binary_search_by_key(&r.time, |k| k.time)
                .is_err()
            {
                return Err(EditError::Invalid("stale keyframe selection"));
            }
            paths.insert(r.path.clone(), param);
            if paths.len() > MAX_CLIPBOARD_PATHS {
                return Err(EditError::Limit);
            }
            origin = origin.min(r.time);
        }
        let mut params = Vec::new();
        for (path, param) in paths {
            let copy = |c: &Channel, selector| -> Result<Channel> {
                let keys = c
                    .keys()
                    .iter()
                    .filter(|k| unique.contains(&(&path, selector, k.time)))
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
            params.push((path, param));
        }
        Self::new(origin, params).map(Some)
    }
    pub(super) fn paste(&self, target: &mut project::Item, at: Tick) -> Result<()> {
        if at < Tick::ZERO {
            return Err(EditError::InvalidParams);
        }
        exact(at)?;
        let anchor = exact(
            at.checked_sub(target.start)
                .ok_or(EditError::InvalidParams)?,
        )?;
        for p in &self.params {
            let destination = target.animations.get_mut(&p.path)?;
            if std::mem::discriminant(destination) != std::mem::discriminant(&p.param) {
                return Err(EditError::Invalid("clipboard parameter type"));
            }
            for (source, dest) in channels(&p.param).iter().zip(channels_mut(destination)) {
                if source.keys().is_empty() {
                    continue;
                }
                if std::mem::discriminant(source.base()) != std::mem::discriminant(dest.base()) {
                    return Err(EditError::Invalid("clipboard value type"));
                }
                let mut keys: BTreeMap<_, _> =
                    dest.keys().iter().map(|k| (k.time, k.clone())).collect();
                for k in source.keys() {
                    let offset = k
                        .time
                        .checked_sub(self.origin)
                        .ok_or(EditError::InvalidParams)?;
                    let mut key = k.clone();
                    key.time = exact(anchor.checked_add(offset).ok_or(EditError::InvalidParams)?)?;
                    keys.insert(key.time, key);
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

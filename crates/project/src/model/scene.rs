//! Scenes, scene selection and bookmarks.
use super::settings::Color;
use super::track::Timeline;
use crate::{Error, Result};
use captions::NfcText;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use time::Tick;

id!(BookmarkId, SceneId);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Bookmark {
    pub id: BookmarkId,
    pub start: Tick,
    pub duration: Tick,
    pub note: NfcText,
    pub color: Color,
}
/// One independently editable timeline; canvas and assets remain project-wide.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Scene {
    pub id: SceneId,
    pub name: NfcText,
    pub is_main: bool,
    pub timeline: Timeline,
    pub bookmarks: Vec<Bookmark>,
}
impl Scene {
    /// End of content in output ticks; bookmarks do not extend the scene.
    pub fn duration(&self) -> Tick {
        self.timeline
            .iter()
            .flat_map(|t| &t.items)
            .filter_map(|i| i.end().ok())
            .max()
            .unwrap_or(Tick::ZERO)
    }
}
/// Explicit export selection; output UI and adapters choose output paths separately.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(
    tag = "scope",
    content = "scenes",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum SceneScope {
    Active,
    All,
    Selected(Vec<SceneId>),
}
/// Nonempty scene array with exactly one main scene. Main is never deletable.
#[derive(Clone, Debug, PartialEq, Deserialize, schemars::JsonSchema)]
#[serde(try_from = "Vec<Scene>")]
#[schemars(with = "Vec<Scene>")]
pub struct Scenes {
    pub(super) main: Scene,
    pub(super) others: Vec<Scene>,
}
impl TryFrom<Vec<Scene>> for Scenes {
    type Error = Error;
    fn try_from(mut scenes: Vec<Scene>) -> Result<Self> {
        if scenes.len() > 256 {
            return Err(Error::TooLarge("scenes"));
        }
        if scenes.iter().filter(|s| s.is_main).count() != 1 {
            return Err(Error::Invalid("exactly one main scene required"));
        }
        let index = scenes
            .iter()
            .position(|s| s.is_main)
            .ok_or(Error::Invalid("main scene"))?;
        let main = scenes.remove(index);
        Ok(Self {
            main,
            others: scenes,
        })
    }
}
impl Serialize for Scenes {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        serializer.collect_seq(self.iter())
    }
}
impl Default for Scenes {
    fn default() -> Self {
        Self {
            main: Scene {
                id: SceneId(0),
                name: "Main scene".into(),
                is_main: true,
                timeline: Timeline::default(),
                bookmarks: Vec::new(),
            },
            others: Vec::new(),
        }
    }
}
impl Scenes {
    pub fn iter(&self) -> impl Iterator<Item = &Scene> {
        std::iter::once(&self.main).chain(&self.others)
    }
    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut Scene> {
        std::iter::once(&mut self.main).chain(&mut self.others)
    }
    pub fn main(&self) -> &Scene {
        &self.main
    }
    pub fn get(&self, id: SceneId) -> Option<&Scene> {
        self.iter().find(|s| s.id == id)
    }
    pub fn get_mut(&mut self, id: SceneId) -> Option<&mut Scene> {
        self.iter_mut().find(|s| s.id == id)
    }
    pub fn insert(&mut self, scene: Scene) -> Result<()> {
        if scene.is_main || self.get(scene.id).is_some() {
            return Err(Error::Invalid("scene identity"));
        }
        if self.others.len() >= 255 {
            return Err(Error::TooLarge("scenes"));
        }
        self.others.push(scene);
        Ok(())
    }
    pub fn delete(&mut self, ids: &BTreeSet<SceneId>) -> Result<()> {
        if ids.contains(&self.main.id) {
            return Err(Error::Invalid("main scene cannot be deleted"));
        }
        if ids.iter().any(|id| self.get(*id).is_none()) {
            return Err(Error::Invalid("scene not found"));
        }
        self.others.retain(|s| !ids.contains(&s.id));
        Ok(())
    }
}

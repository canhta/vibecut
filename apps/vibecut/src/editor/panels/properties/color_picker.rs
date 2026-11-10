//! Maps properties colour drafts to project commands.
use crate::services::color_picker::Picker;
use crate::{Error, Result};
use project::{BookmarkId, Color, ItemId};

pub(super) struct Draft {
    pub(super) target: ColorTarget,
    pub(super) picker: Picker,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ColorTarget {
    Text(ItemId),
    Background(ItemId),
    GraphicFill(ItemId),
    GraphicStroke(ItemId),
    MaskStroke {
        item: ItemId,
        mask: String,
    },
    /// The chroma key's packed key colour on one effect instance.
    EffectKeyColor {
        item: ItemId,
        effect: String,
    },
    Bookmark(BookmarkId),
    Canvas,
}
impl ColorTarget {
    pub(crate) fn item(&self) -> Option<ItemId> {
        match self {
            Self::Text(id)
            | Self::Background(id)
            | Self::GraphicFill(id)
            | Self::GraphicStroke(id) => Some(*id),
            Self::MaskStroke { item, .. } | Self::EffectKeyColor { item, .. } => Some(*item),
            Self::Bookmark(_) | Self::Canvas => None,
        }
    }
    pub(crate) fn color(&self, project: &project::Project) -> Result<Color> {
        if *self == Self::Canvas {
            return Ok(project.data().canvas.background);
        }
        if let Self::Bookmark(bookmark) = self {
            return project
                .data()
                .scene()
                .bookmarks
                .iter()
                .find(|candidate| candidate.id == *bookmark)
                .map(|candidate| candidate.color)
                .ok_or(Error::Invalid("bookmark colour target"));
        }
        let item = project
            .data()
            .scene()
            .timeline
            .iter()
            .flat_map(|t| &t.items)
            .find(|i| Some(i.id) == self.item())
            .ok_or(Error::Invalid("colour target"))?;
        match (self, &item.content) {
            (Self::Text(_), project::ItemParams::Text(p)) => Ok(p.color),
            (Self::Background(_), project::ItemParams::Text(p)) => {
                Ok(p.background.as_ref().map_or(Color([0; 4]), |b| b.color))
            }
            (Self::GraphicFill(_), project::ItemParams::Overlay(p)) => Ok(p.color),
            (Self::GraphicStroke(_), project::ItemParams::Overlay(_)) => Ok(render::color::at(
                &item.animations,
                &project::AnimationPath::new("params.strokeColor")?,
                time::Tick::ZERO,
                Color([0; 4]),
            )?),
            (Self::MaskStroke { mask, .. }, _) => item
                .masks
                .iter()
                .find(|m| m.id == *mask)
                .map(|m| m.stroke.color)
                .ok_or(Error::Invalid("mask colour target")),
            (Self::EffectKeyColor { effect, .. }, _) => item
                .effects
                .iter()
                .find(|candidate| candidate.id == *effect)
                .and_then(|candidate| candidate.value("key_color")?.color())
                .ok_or(Error::Invalid("chroma key colour target")),
            _ => Err(Error::Invalid("colour target")),
        }
    }
    pub(crate) fn command(
        &self,
        project: &project::Project,
        color: Color,
    ) -> Result<engine::Command> {
        if *self == Self::Canvas {
            return Ok(engine::Command::SetBackground { color });
        }
        if let Self::Bookmark(bookmark) = self {
            return Ok(engine::Command::UpdateBookmark {
                bookmark: *bookmark,
                note: None,
                color: Some(color),
                duration: None,
            });
        }
        if let Self::MaskStroke { item, mask } = self {
            let mut value = project
                .data()
                .scene()
                .timeline
                .iter()
                .flat_map(|t| &t.items)
                .find(|i| i.id == *item)
                .and_then(|i| i.masks.iter().find(|m| m.id == *mask))
                .cloned()
                .ok_or(Error::Invalid("mask colour target"))?;
            value.stroke.color = color;
            return Ok(engine::Command::UpdateMask {
                item: *item,
                mask: value,
            });
        }
        if let Self::EffectKeyColor { item, effect } = self {
            let [r, g, b, _] = color.0;
            let packed = (u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b)) as f64;
            return Ok(engine::Command::UpdateEffectParam {
                item: *item,
                effect: effect.clone(),
                key: "key_color".into(),
                value: packed,
            });
        }
        let item = self.item().ok_or(Error::Invalid("colour target"))?;
        let patch = match self {
            Self::Text(_) => engine::ItemParamPatch::TextColor(color),
            Self::Background(_) => engine::ItemParamPatch::TextBackgroundColor(color),
            Self::GraphicFill(_) => engine::ItemParamPatch::GraphicFill(color),
            Self::GraphicStroke(_) => engine::ItemParamPatch::GraphicStroke(color),
            _ => return Err(Error::Invalid("colour target")),
        };
        Ok(engine::Command::SetItemField { item, patch })
    }
}

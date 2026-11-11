//! Maps properties colour drafts to project commands.
use crate::services::color_picker::Picker;
use crate::{Error, Result, egui};
use project::{AnimationPath, BookmarkId, Color, ColorComponent, ItemId, Param, ParamValue};
use time::Tick;

const RGB: [ColorComponent; 3] = [ColorComponent::R, ColorComponent::G, ColorComponent::B];

pub(super) struct Draft {
    pub(super) target: ColorTarget,
    pub(super) picker: Picker,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ColorTarget {
    Text(ItemId),
    Background(ItemId),
    GraphicFill(ItemId),
    GraphicColor {
        item: ItemId,
        stroke: bool,
        time: Tick,
    },
    GraphicStroke(ItemId),
    MaskStroke {
        item: ItemId,
        mask: String,
    },
    /// One effect colour at the captured real item-local time.
    EffectColor {
        item: ItemId,
        effect: String,
        key: String,
        time: Tick,
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
            Self::GraphicColor { item, .. }
            | Self::MaskStroke { item, .. }
            | Self::EffectColor { item, .. } => Some(*item),
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
            (Self::GraphicColor { stroke, time, .. }, project::ItemParams::Graphic(p)) => {
                let (fill, outline, ..) = p.appearance();
                Ok(render::color::at(
                    &item.animations,
                    &AnimationPath::new(if *stroke {
                        "params.stroke"
                    } else {
                        "params.fill"
                    })?,
                    *time,
                    if *stroke { outline } else { fill },
                )?)
            }
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
            (
                Self::EffectColor {
                    effect, key, time, ..
                },
                _,
            ) => {
                let fallback = item
                    .effects
                    .iter()
                    .find(|candidate| candidate.id == *effect)
                    .and_then(|candidate| candidate.value(key)?.color())
                    .ok_or(Error::Invalid("effect colour target"))?;
                if key == "key_color" {
                    return Ok(fallback);
                }
                let path = AnimationPath::new(format!("effects.{effect}.params.{key}"))?;
                if item
                    .animations
                    .get(&path)
                    .is_some_and(|p| !matches!(p, Param::Color(_)))
                {
                    return Err(Error::Invalid("effect colour animation"));
                }
                let mut color = render::color::at(&item.animations, &path, *time, fallback)?;
                color.0[3] = 255;
                Ok(color)
            }
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
        if let Self::GraphicColor { item, time, .. } = self {
            let (path, mut param) = self.graphic_animation(project)?;
            let keyed = self.rgb_keys(project, false);
            if keyed && !self.inside(project) {
                return Err(Error::Invalid("graphic colour time"));
            }
            for (index, component) in [
                ColorComponent::R,
                ColorComponent::G,
                ColorComponent::B,
                ColorComponent::A,
            ]
            .into_iter()
            .enumerate()
            {
                let byte = *color.0.get(index).ok_or(Error::Invalid("RGBA component"))?;
                let value = ParamValue::Scalar(if component == ColorComponent::A {
                    f64::from(byte) / 255.
                } else {
                    f64::from(egui::ecolor::linear_f32_from_gamma_u8(byte))
                });
                let channel = param.channel_mut(Some(component))?;
                if keyed {
                    let mut key = channel
                        .keys()
                        .iter()
                        .find(|k| k.time == *time)
                        .cloned()
                        .unwrap_or_else(|| project::Keyframe::new(*time, value.clone()));
                    key.value = value;
                    channel.put(key)?;
                } else {
                    let (before, after) = channel.extrapolation();
                    *channel = project::Channel::new(value, vec![], before, after)?;
                }
            }
            return Ok(engine::Command::SetAnimatedParam {
                item: *item,
                path,
                param,
            });
        }
        if let Self::EffectColor {
            item,
            effect,
            key,
            time,
        } = self
        {
            if key == "key_color" {
                let [r, g, b, _] = color.0;
                return Ok(engine::Command::UpdateEffectParam {
                    item: *item,
                    effect: effect.clone(),
                    key: key.clone(),
                    value: project::EffectParamUpdate::Scalar(f64::from(
                        u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b),
                    )),
                });
            }
            let (path, mut param) = self.effect_animation(project)?;
            let keyed = self.rgb_keys(project, false);
            if keyed && !self.inside(project) {
                return Err(Error::Invalid("effect colour time"));
            }
            for (component, byte) in RGB.into_iter().zip(color.0) {
                let value =
                    ParamValue::Scalar(f64::from(egui::ecolor::linear_f32_from_gamma_u8(byte)));
                let channel = param.channel_mut(Some(component))?;
                if keyed {
                    let mut key = channel
                        .keys()
                        .iter()
                        .find(|k| k.time == *time)
                        .cloned()
                        .unwrap_or_else(|| project::Keyframe::new(*time, value.clone()));
                    key.value = value;
                    channel.put(key)?;
                } else {
                    let (before, after) = channel.extrapolation();
                    *channel = project::Channel::new(value, vec![], before, after)?;
                }
            }
            return Ok(engine::Command::SetAnimatedParam {
                item: *item,
                path,
                param,
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
    fn graphic_animation(&self, project: &project::Project) -> Result<(AnimationPath, Param)> {
        let Self::GraphicColor { stroke, .. } = self else {
            return Err(Error::Invalid("graphic colour target"));
        };
        let item = self.effect_item(project)?;
        let project::ItemParams::Graphic(p) = &item.content else {
            return Err(Error::Invalid("graphic colour target"));
        };
        let (fill, outline, ..) = p.appearance();
        let path = AnimationPath::new(if *stroke {
            "params.stroke"
        } else {
            "params.fill"
        })?;
        let param = match item.animations.get(&path) {
            Some(param @ Param::Color(_)) => param.clone(),
            Some(_) => return Err(Error::Invalid("graphic colour animation")),
            None => {
                let [r, g, b, a] = if *stroke { outline.0 } else { fill.0 };
                let [r, g, b] =
                    [r, g, b].map(|v| f64::from(egui::ecolor::linear_f32_from_gamma_u8(v)));
                Param::new(ParamValue::Color([r, g, b, f64::from(a) / 255.]))?
            }
        };
        Ok((path, param))
    }
    fn effect_item<'a>(&self, project: &'a project::Project) -> Result<&'a project::Item> {
        project
            .data()
            .scene()
            .timeline
            .iter()
            .flat_map(|t| &t.items)
            .find(|i| Some(i.id) == self.item())
            .ok_or(Error::Invalid("effect colour target"))
    }
    fn effect_animation(&self, project: &project::Project) -> Result<(AnimationPath, Param)> {
        let Self::EffectColor { effect, key, .. } = self else {
            return Err(Error::Invalid("effect colour target"));
        };
        let item = self.effect_item(project)?;
        let instance = item
            .effects
            .iter()
            .find(|e| e.id == *effect)
            .ok_or(Error::Invalid("effect colour target"))?;
        let definition = instance.definition()?;
        if !definition.params.iter().any(|p| {
            p.id == key && p.animatable && matches!(p.kind, project::EffectParamKind::Color)
        }) {
            return Err(Error::Invalid("effect colour parameter"));
        }
        let path = AnimationPath::new(format!("effects.{effect}.params.{key}"))?;
        let param = match item.animations.get(&path) {
            Some(Param::Color(_)) => item
                .animations
                .get(&path)
                .cloned()
                .ok_or(Error::Invalid("effect colour animation"))?,
            Some(_) => return Err(Error::Invalid("effect colour animation")),
            None => {
                let color = instance
                    .value(key)
                    .and_then(|v| v.color())
                    .ok_or(Error::Invalid("effect colour parameter"))?;
                let [r, g, b, a] = color.0;
                let [r, g, b] =
                    [r, g, b].map(|v| f64::from(egui::ecolor::linear_f32_from_gamma_u8(v)));
                Param::new(ParamValue::Color([r, g, b, f64::from(a) / 255.0]))?
            }
        };
        Ok((path, param))
    }
    pub(super) fn rgb_keys(&self, project: &project::Project, at_tick: bool) -> bool {
        if let Self::GraphicColor { time, .. } = self {
            return self.graphic_animation(project).is_ok_and(|(_, param)| {
                [
                    ColorComponent::R,
                    ColorComponent::G,
                    ColorComponent::B,
                    ColorComponent::A,
                ]
                .into_iter()
                .any(|c| {
                    param.channel(Some(c)).is_ok_and(|channel| {
                        channel.keys().iter().any(|k| !at_tick || k.time == *time)
                    })
                })
            });
        }
        let Self::EffectColor {
            effect, key, time, ..
        } = self
        else {
            return false;
        };
        let Ok(item) = self.effect_item(project) else {
            return false;
        };
        let Ok(path) = AnimationPath::new(format!("effects.{effect}.params.{key}")) else {
            return false;
        };
        item.animations.get(&path).is_some_and(|param| {
            RGB.into_iter().any(|c| {
                param
                    .channel(Some(c))
                    .is_ok_and(|channel| channel.keys().iter().any(|k| !at_tick || k.time == *time))
            })
        })
    }
    pub(super) fn inside(&self, project: &project::Project) -> bool {
        if let Self::GraphicColor { time, .. } = self {
            return self
                .effect_item(project)
                .is_ok_and(|i| i.duration().is_ok_and(|d| Tick::ZERO <= *time && *time < d));
        }
        let Self::EffectColor { time, .. } = self else {
            return false;
        };
        self.effect_item(project).is_ok_and(|i| {
            i.duration()
                .is_ok_and(|duration| Tick::ZERO <= *time && *time <= duration)
        })
    }
    pub(super) fn toggle_rgb(&self, project: &project::Project) -> Result<engine::Command> {
        if let Self::GraphicColor { item, time, .. } = self {
            if !self.inside(project) {
                return Err(Error::Invalid("graphic colour time"));
            }
            let (path, mut param) = self.graphic_animation(project)?;
            let ParamValue::Color(values) = param.value_at(*time)? else {
                return Err(Error::Invalid("graphic colour animation"));
            };
            let components = [
                ColorComponent::R,
                ColorComponent::G,
                ColorComponent::B,
                ColorComponent::A,
            ];
            let remove = components.into_iter().all(|c| {
                param
                    .channel(Some(c))
                    .is_ok_and(|ch| ch.keys().iter().any(|k| k.time == *time))
            });
            for (c, value) in components.into_iter().zip(values) {
                let ch = param.channel_mut(Some(c))?;
                let exists = ch.keys().iter().any(|k| k.time == *time);
                if remove {
                    ch.remove(*time)?;
                } else if !exists {
                    ch.put(project::Keyframe::new(*time, ParamValue::Scalar(value)))?;
                }
            }
            return Ok(engine::Command::SetAnimatedParam {
                item: *item,
                path,
                param,
            });
        }
        let Self::EffectColor { item, time, .. } = self else {
            return Err(Error::Invalid("effect colour target"));
        };
        if !self.inside(project) {
            return Err(Error::Invalid("effect colour time"));
        }
        let (path, mut param) = self.effect_animation(project)?;
        let ParamValue::Color(values) = param.value_at(*time)? else {
            return Err(Error::Invalid("effect colour animation"));
        };
        let remove = self.rgb_keys(project, true);
        for (component, value) in RGB.into_iter().zip(values) {
            let channel = param.channel_mut(Some(component))?;
            if remove {
                if channel.keys().iter().any(|k| k.time == *time) {
                    channel.remove(*time)?;
                }
            } else {
                channel.put(project::Keyframe::new(*time, ParamValue::Scalar(value)))?;
            }
        }
        Ok(engine::Command::SetAnimatedParam {
            item: *item,
            path,
            param,
        })
    }
}

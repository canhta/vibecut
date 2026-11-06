//! Adapts property edits to engine patches.

use crate::editor::properties::{
    Editor, ElementKind, Error, ItemId, ItemParamPatch, Result, element_kind, single,
};

impl Editor {
    /// The inspector and automation use the same registered atomic field edit.
    pub fn set_shared_field(&mut self, field: &str, value: &str) -> Result<()> {
        let patch = item_patch(field, value)?;
        self.session.set_selection(self.selection.clone())?;
        self.session.execute(
            "timeline.set_item_param_many",
            serde_json::json!({"items":self.selection,"patch":patch}),
        )?;
        self.sync_preview();
        Ok(())
    }
    /// One element's committed field edit, through the singular registered command.
    pub fn set_item_field(&mut self, item: ItemId, field: &str, value: &str) -> Result<()> {
        if field == "speed" {
            let speed = value
                .trim()
                .parse::<f64>()
                .map_err(|_| Error::Invalid("speed"))?;
            let (_, maintain_pitch, _) =
                crate::editor::properties::read::retime_of(self.session.project(), item)
                    .ok_or(Error::Invalid("speed"))?;
            return self.set_speed(item, speed, maintain_pitch);
        }
        let patch = item_patch(field, value)?;
        self.session.set_selection(self.selection.clone())?;
        self.session.execute(
            "timeline.set_item_param",
            serde_json::json!({"item":item,"patch":patch}),
        )?;
        self.sync_preview();
        Ok(())
    }
}

fn item_patch(field: &str, value: &str) -> Result<ItemParamPatch> {
    if let Some(transform) = project::TransformField::ALL
        .into_iter()
        .find(|f| f.path() == field)
    {
        let value = value
            .trim()
            .parse::<f64>()
            .map_err(|_| Error::Invalid("transform value"))?;
        let value = transform.clamp(value)?;
        use project::TransformField as F;
        return Ok(match transform {
            F::PositionX => ItemParamPatch::PositionX(value),
            F::PositionY => ItemParamPatch::PositionY(value),
            F::ScaleX => ItemParamPatch::ScaleX(value),
            F::ScaleY => ItemParamPatch::ScaleY(value),
            F::Rotate => ItemParamPatch::Rotate(value),
        });
    }
    Ok(match field {
        "blendMode" => ItemParamPatch::BlendMode(
            serde_json::from_value(serde_json::json!(value))
                .map_err(|_| Error::Invalid("blend mode"))?,
        ),
        "opacity" => ItemParamPatch::Opacity(
            value
                .trim()
                .parse()
                .map_err(|_| Error::Invalid("opacity"))?,
        ),
        "audio_gain" => ItemParamPatch::AudioGain(
            value
                .trim()
                .parse()
                .map_err(|_| Error::Invalid("audio gain"))?,
        ),
        "audio_muted" => {
            let muted = match value.trim() {
                "true" | "1" | "muted" => true,
                "false" | "0" | "unmuted" => false,
                _ => return Err(Error::Invalid("audio mute")),
            };
            ItemParamPatch::AudioMuted(muted)
        }
        "text_content" => ItemParamPatch::TextContent(value.into()),
        "text_font" => {
            let font = value.trim();
            if font.is_empty() || font.len() > 256 {
                return Err(Error::Invalid("font family"));
            }
            ItemParamPatch::TextFont(font.into())
        }
        "text_size" => ItemParamPatch::TextSize(
            value
                .trim()
                .parse()
                .map_err(|_| Error::Invalid("text size"))?,
        ),
        "text_color" => ItemParamPatch::TextColor(parse_color(value)?),
        "text_background" => {
            let value = value.trim();
            ItemParamPatch::TextBackground(if value.eq_ignore_ascii_case("none") {
                None
            } else {
                Some(project::TextBackground {
                    color: parse_color(value)?,
                    ..Default::default()
                })
            })
        }
        "text_background_color" => ItemParamPatch::TextBackgroundColor(parse_color(value)?),
        "text_position_x" => ItemParamPatch::TextPositionX(
            parse_percent(value).map_err(|_| Error::Invalid("horizontal position"))?,
        ),
        "text_position_y" => ItemParamPatch::TextPositionY(
            parse_percent(value).map_err(|_| Error::Invalid("vertical position"))?,
        ),
        _ => return Err(Error::Invalid("shared field")),
    })
}
fn parse_color(value: &str) -> Result<project::Color> {
    Ok(render::color::parse_hex(value)?)
}
fn parse_percent(value: &str) -> Result<f32> {
    let value: f32 = value
        .trim()
        .parse()
        .map_err(|_| Error::Invalid("percentage"))?;
    if !value.is_finite() || !(0.0..=100.0).contains(&value) {
        return Err(Error::Invalid("percentage"));
    }
    Ok(value)
}

impl Editor {
    pub(in crate::editor) fn selected_item_id(&self) -> Option<ItemId> {
        single(&self.selection)
    }
    pub(in crate::editor) fn element_kind(&self, id: ItemId) -> Option<(ElementKind, bool)> {
        element_kind(self.session.project(), id)
    }
}

//! Applies effect choices to the editor context.
use crate::editor::assets::{Arc, Editor, Result, Text, egui};

impl Editor {
    /// Attach the native effect adapter to playback and thumbnail jobs, sharing
    /// playback's byte budget. Failure leaves the thumbnail composition unchanged.
    pub fn set_effects(
        &mut self,
        effects: Arc<dyn render::effects::EffectCompositor>,
    ) -> Result<()> {
        let budget = self.playback.budget().clone();
        let cache =
            engine::effect_thumbnails::EffectThumbnails::new(effects.clone(), budget.clone());
        let previews = crate::effect_previews::EffectPreviews::new(cache, budget)?;
        self.playback.set_effects(effects)?;
        self.effect_previews = Some(previews);
        self.effect_textures.clear();
        Ok(())
    }
    pub(in crate::editor) fn add_effect_from_selection(&mut self, kind: &str) {
        match self.selected_item_id() {
            Some(item) => self.add_effect_to_item(item, kind),
            None => {
                let text = self.tr(Text::EffectSelectClip).to_owned();
                self.set_toast(&text, None);
            }
        }
    }
    pub(in crate::editor) fn finish_effect_drag(&mut self, ctx: &egui::Context) {
        let Some(kind) = self.effect_drag.clone() else {
            return;
        };
        if !ctx.input(|input| input.pointer.any_released()) {
            return;
        }
        self.effect_drag = None;
        let position = ctx.input(|input| {
            input
                .pointer
                .interact_pos()
                .or_else(|| input.pointer.latest_pos())
        });
        let target = position.and_then(|position| {
            self.timeline
                .item_rects()
                .iter()
                .find(|(_, rect)| rect.contains(position))
                .map(|(item, _)| *item)
        });
        if let Some(item) = target {
            self.add_effect_to_item(item, &kind);
        } else {
            let text = self.tr(Text::EffectDropMiss).to_owned();
            self.set_toast(&text, None);
        }
    }
}

impl Editor {
    pub(in crate::editor) fn effect_name(
        &self,
        definition: &project::EffectDefinition,
    ) -> &'static str {
        match definition.kind {
            "blur" => self.tr(Text::EffectBlur),
            "color_grade" => self.tr(Text::EffectColorGrade),
            _ => definition.name,
        }
    }
}

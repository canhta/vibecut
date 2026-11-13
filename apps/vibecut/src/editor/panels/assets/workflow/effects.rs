//! Applies effect choices from the Effects tab.
use super::super::effects::{EffectPreviews, preview_instance};
use super::super::{Arc, Text, egui};
use super::State;
use crate::editor::{Desk, Result, context::single};

impl State {
    /// Attach the native effect adapter to playback and thumbnail jobs, sharing
    /// playback's byte budget. Failure leaves the thumbnail composition unchanged.
    pub(in crate::editor) fn set_effects(
        &mut self,
        desk: &mut Desk<'_>,
        effects: Arc<dyn render::effects::EffectCompositor>,
    ) -> Result<()> {
        let budget = desk.editor().playback.budget().clone();
        let cache =
            engine::effect_thumbnails::EffectThumbnails::new(effects.clone(), budget.clone());
        let previews = EffectPreviews::new(cache, budget)?;
        desk.editor().playback.set_effects(effects)?;
        self.effect_previews = Some(previews);
        Ok(())
    }

    pub(in crate::editor) fn add_effect_from_selection(&mut self, desk: &mut Desk<'_>, kind: &str) {
        let selected = single(desk.view().selection());
        match selected {
            Some(item) => desk.editor().add_effect_to_item(item, kind),
            None => {
                let text =
                    crate::i18n::text(desk.editor().preferences.locale, Text::EffectSelectClip);
                desk.toast(text, None);
            }
        }
    }

    /// Finish the tile drag in the same frame the pointer is released; only a
    /// clip under the pointer accepts the effect.
    pub(in crate::editor) fn finish_effect_drag(
        &mut self,
        desk: &mut Desk<'_>,
        ctx: &egui::Context,
    ) {
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
        let rects = desk.editor().timeline.item_rects().to_vec();
        let target = position.and_then(|position| {
            rects
                .iter()
                .find(|(_, rect)| rect.contains(position))
                .map(|(item, _)| *item)
        });
        if let Some(item) = target {
            desk.editor().add_effect_to_item(item, &kind);
        } else {
            let text = crate::i18n::text(desk.editor().preferences.locale, Text::EffectDropMiss);
            desk.toast(text, None);
        }
    }

    /// Replace one failed tile's static request; a late old job cannot present.
    pub(in crate::editor) fn retry_effect(&mut self, kind: &str) {
        if let Some(definition) = project::effect_definitions()
            .iter()
            .find(|definition| definition.id == kind)
            && let Some(params) = preview_instance(definition)
            && let Some(previews) = &mut self.effect_previews
        {
            previews.retry(kind, &params, &project::Animations::default());
        }
    }
}

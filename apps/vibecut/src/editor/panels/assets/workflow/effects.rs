//! Applies choices from the tabs that add effect stacks.
use super::super::effects::{EffectPreviews, preview_instance};
use super::super::{Arc, CreatedId, TICKS_PER_SECOND, Text, Tick, egui};
use super::State;
use crate::editor::{Port, Request, Result, context::single};
use std::collections::BTreeSet;

/// A filter clip is three seconds long, as the reference's is.
const FILTER_DURATION: Tick = Tick(3 * TICKS_PER_SECOND);

impl State {
    /// Insert one filter clip on its own effect lane at the playhead at full Intensity as one
    /// undo step, and select it. A refused edit changes nothing.
    pub(in crate::editor) fn insert_filter(&mut self, port: &mut Port<'_>, filter: &str) {
        if !port.read.insertion_available {
            return;
        }
        let Ok(at) = port.playhead() else {
            let text = port.tr(Text::InsertFailed);
            port.toast(text, None);
            return;
        };
        // A refusal is already on the error line through `apply_result`.
        let Ok(outcome) = port.apply_result(engine::Command::FiltersInsert {
            placement: engine::Placement::Auto,
            filter: filter.to_owned(),
            start: at,
            duration: FILTER_DURATION,
            intensity: 100.0,
        }) else {
            return;
        };
        let created = outcome.created.into_iter().find_map(|id| match id {
            CreatedId::Item(item) => Some(item),
            _ => None,
        });
        let Some(item) = created else {
            return;
        };
        port.select(BTreeSet::from([item]));
        port.reset_selection(true);
    }

    /// Build the thumbnail previews that share playback's byte budget with the native
    /// effect adapter. The frame attaches the adapter to playback itself.
    pub(in crate::editor) fn effect_previews_for(
        budget: &engine::ByteBudget,
        effects: Arc<dyn render::effects::EffectCompositor>,
    ) -> Result<EffectPreviews> {
        let cache = engine::effect_thumbnails::EffectThumbnails::new(effects, budget.clone());
        EffectPreviews::new(cache, budget.clone())
    }

    pub(in crate::editor) fn add_effect_from_selection(&mut self, port: &mut Port<'_>, kind: &str) {
        match single(port.session.selection()) {
            Some(item) => port.request(Request::AddEffect {
                item,
                kind: kind.to_owned(),
            }),
            None => {
                let text = port.tr(Text::EffectSelectClip);
                port.toast(text, None);
            }
        }
    }

    /// Finish the tile drag in the same frame the pointer is released; only a
    /// clip under the pointer accepts the effect.
    pub(in crate::editor) fn finish_effect_drag(
        &mut self,
        port: &mut Port<'_>,
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
        let target = position.and_then(|position| {
            port.read
                .item_rects
                .iter()
                .find(|(_, rect)| rect.contains(position))
                .map(|(item, _)| *item)
        });
        if let Some(item) = target {
            port.request(Request::AddEffect { item, kind });
        } else {
            let text = port.tr(Text::EffectDropMiss);
            port.toast(text, None);
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

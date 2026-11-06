//! Applies numeric editing gestures.

use crate::editor::properties::{
    Editor, ItemId, ItemParams, ScrubDrag, ScrubSpec, ScrubTarget, Text, Tick,
};

impl Editor {
    pub(in crate::editor) fn item_value(&self, item: ItemId, field: &str) -> Option<f64> {
        let item = self
            .session
            .project()
            .data()
            .scene()
            .timeline
            .iter()
            .flat_map(|track| &track.items)
            .find(|candidate| candidate.id == item)?;
        if let Some(transform) = project::TransformField::ALL
            .into_iter()
            .find(|f| f.path() == field)
        {
            return transform.value_at(&item.animations, Tick::ZERO).ok();
        }
        match (field, &item.content) {
            ("opacity", content) if !matches!(content, ItemParams::Audio(_)) => {
                let path = project::AnimationPath::new("opacity").ok()?;
                Some(
                    match item.animations.value_at(&path, Tick::ZERO).ok().flatten() {
                        Some(project::ParamValue::Scalar(value)) => value.clamp(0.0, 1.0),
                        _ => 1.0,
                    },
                )
            }
            ("text_size", ItemParams::Text(params)) => Some(f64::from(params.font_size)),
            ("audio_gain", ItemParams::Audio(params)) => Some(f64::from(params.volume)),
            ("audio_gain", ItemParams::Video(params)) => Some(f64::from(params.volume)),
            ("speed", ItemParams::Audio(_) | ItemParams::Video(_)) => {
                Some(item.retime.speed.as_f64())
            }
            _ => None,
        }
    }
    pub(in crate::editor) fn start_scrub(&mut self, item: ItemId, spec: ScrubSpec, value: f64) {
        self.start_scrub_target(item, ScrubTarget::Item(spec.field), value);
    }
    pub(in crate::editor) fn start_effect_scrub(
        &mut self,
        item: ItemId,
        effect: &str,
        key: &str,
        value: f64,
    ) {
        self.start_scrub_target(
            item,
            ScrubTarget::Effect {
                effect: effect.to_owned(),
                key: key.to_owned(),
            },
            value,
        );
    }
    pub(in crate::editor) fn start_scrub_target(
        &mut self,
        item: ItemId,
        target: ScrubTarget,
        value: f64,
    ) {
        if self.scrub.is_some() {
            self.finish_scrub(false);
        }
        self.session.end_merge();
        if self.session.transaction_active()
            && let Err(error) = self.session.cancel_gesture()
        {
            self.error = Some(error.to_string());
            return;
        }
        if let Err(error) = self
            .session
            .set_selection(self.selection.clone())
            .and_then(|_| self.session.begin_gesture())
        {
            self.error = Some(error.to_string());
            return;
        }
        self.property_drafts.remove(&target.draft_key());
        self.property_focus = None;
        self.scrub = Some(ScrubDrag {
            item,
            target,
            start: value,
            last: value,
        });
    }
    pub(in crate::editor) fn finish_scrub(&mut self, commit: bool) {
        let Some(scrub) = self.scrub.take() else {
            return;
        };
        let changed = scrub.last != scrub.start;
        let result = if commit && changed {
            self.session.commit_gesture()
        } else {
            self.session.cancel_gesture()
        };
        if let Err(error) = result {
            self.error = Some(error.to_string());
        }
        self.property_drafts.remove(&scrub.target.draft_key());
        if changed {
            self.sync_preview();
        }
    }
}

pub(in crate::editor::properties) fn transform_scrub(field: project::TransformField) -> ScrubSpec {
    use project::TransformField as F;
    let (min, max) = field.bounds();
    let label = match field {
        F::PositionX => Text::PositionX,
        F::PositionY => Text::PositionY,
        F::ScaleX => Text::ScaleX,
        F::ScaleY => Text::ScaleY,
        F::Rotate => Text::Rotate,
    };
    let scale = matches!(field, F::ScaleX | F::ScaleY);
    ScrubSpec {
        field: field.path(),
        label,
        min,
        max,
        step: if scale { 0.01 } else { 1.0 },
        pixels_per_step: 2.0,
        decimals: if scale { 2 } else { 0 },
        suffix: match field {
            F::PositionX | F::PositionY => "px",
            F::Rotate => "°",
            _ => "",
        },
    }
}

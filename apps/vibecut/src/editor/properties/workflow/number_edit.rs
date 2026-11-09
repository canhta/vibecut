//! Applies numeric editing gestures.

use crate::editor::properties::{Editor, Result, ScrubSpec, Text, apply};

impl Editor {
    /// Open the engine gesture for the scrub the panel recorded.
    pub(in crate::editor::properties) fn begin_gesture(&mut self) -> Result<()> {
        let result = (|| -> Result<()> {
            self.session.end_merge();
            if self.session.transaction_active() {
                self.session.cancel_gesture()?;
            }
            self.desk().push_selection()?;
            self.session.begin_gesture()?;
            Ok(())
        })();
        if result.is_err() {
            self.properties.scrub = None;
        }
        result
    }
    pub(in crate::editor::properties) fn end_gesture(
        &mut self,
        commit: bool,
        changed: bool,
    ) -> Result<()> {
        let result = if commit && changed {
            self.session.commit_gesture()
        } else {
            self.session.cancel_gesture()
        };
        if changed {
            self.desk().sync();
        }
        result.map(|_| ()).map_err(Into::into)
    }
    pub(in crate::editor) fn finish_scrub(&mut self, commit: bool) {
        let actions = self.properties.end_scrub(commit);
        apply(&mut self.desk(), actions);
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
        step: if scale || field == F::Rotate {
            0.01
        } else {
            1.0
        },
        pixels_per_step: if field == F::Rotate { 0.02 } else { 2.0 },
        decimals: if scale || field == F::Rotate { 2 } else { 0 },
        suffix: match field {
            F::Rotate => "°",
            _ => "",
        },
    }
}

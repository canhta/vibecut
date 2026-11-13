//! Applies numeric editing gestures.
use super::super::{State, gesture::ScrubSpec};
use crate::editor::{Desk, Result, Text};

impl State {
    /// Open the engine gesture for the scrub the panel recorded.
    pub(super) fn begin_gesture(&mut self, desk: &mut Desk<'_>) -> Result<()> {
        let result = (|| -> Result<()> {
            if desk.view().session.transaction_active() {
                desk.with_session(|session| Ok(session.cancel_gesture().map(|_| ())?))?;
            }
            desk.with_session(|session| Ok(session.begin_gesture().map(|_| ())?))?;
            Ok(())
        })();
        if result.is_err() {
            self.scrub = None;
        }
        result
    }

    pub(super) fn end_gesture(
        &mut self,
        desk: &mut Desk<'_>,
        commit: bool,
        changed: bool,
    ) -> Result<()> {
        let result = if commit && changed {
            desk.with_session(|session| Ok(session.commit_gesture().map(|_| ())?))
        } else {
            desk.with_session(|session| Ok(session.cancel_gesture().map(|_| ())?))
        };
        if changed {
            desk.sync();
        }
        result
    }
}

pub(in crate::editor) fn transform_scrub(field: project::TransformField) -> ScrubSpec {
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
        scale: if scale { 100.0 } else { 1.0 },
        field: field.builtin().as_str(),
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
            F::ScaleX | F::ScaleY => "%",
            _ => "",
        },
    }
}

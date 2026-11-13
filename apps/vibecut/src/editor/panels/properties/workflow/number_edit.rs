//! Applies numeric editing gestures.
use super::super::{State, gesture::ScrubSpec};
use crate::editor::{Port, Result, Text};

impl State {
    /// Open the engine gesture for the scrub the panel recorded.
    pub(super) fn begin_gesture(&mut self, port: &mut Port<'_>) -> Result<()> {
        let result = (|| -> Result<()> {
            if port.view().session.transaction_active() {
                port.with_session(|session| Ok(session.cancel_gesture().map(|_| ())?))?;
            }
            port.with_session(|session| Ok(session.begin_gesture().map(|_| ())?))?;
            Ok(())
        })();
        if result.is_err() {
            self.scrub = None;
        }
        result
    }

    pub(super) fn end_gesture(
        &mut self,
        port: &mut Port<'_>,
        commit: bool,
        changed: bool,
    ) -> Result<()> {
        let result = if commit && changed {
            port.with_session(|session| Ok(session.commit_gesture().map(|_| ())?))
        } else {
            port.with_session(|session| Ok(session.cancel_gesture().map(|_| ())?))
        };
        if changed {
            port.sync();
        }
        result
    }
}

pub(in crate::editor) fn transform_scrub(field: project::TransformField) -> ScrubSpec {
    use project::TransformField as F;
    let label = match field {
        F::PositionX => Text::PositionX,
        F::PositionY => Text::PositionY,
        F::ScaleX => Text::ScaleX,
        F::ScaleY => Text::ScaleY,
        F::Rotate => Text::Rotate,
    };
    let scale = matches!(field, F::ScaleX | F::ScaleY);
    ScrubSpec::of(
        field.builtin().as_str(),
        label,
        if scale || field == F::Rotate {
            0.01
        } else {
            1.0
        },
        if field == F::Rotate { 0.02 } else { 2.0 },
        if scale || field == F::Rotate { 2 } else { 0 },
    )
}

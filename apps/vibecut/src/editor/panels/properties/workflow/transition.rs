//! Applies transition inspector edits through the transition commands.
use crate::editor::{Desk, Error, Result};
use engine::Command;
use project::{TransitionAlignment, TransitionId};
use time::Tick;

/// Resize through `transition.set_duration`; the engine clamps to the handles
/// on both sides and refuses a span with none, leaving the project unchanged.
pub(super) fn set_duration(
    desk: &mut Desk<'_>,
    transition: TransitionId,
    duration: Tick,
) -> Result<()> {
    desk.apply_result(
        Command::SetTransitionDuration {
            transition,
            duration,
        },
        false,
    )?;
    Ok(())
}

/// Alignment has no command of its own: it is a `transition.move` to the span
/// position that alignment names, batched with a shrink when the handle on the
/// side the span must grow into is shorter than the span.
pub(super) fn set_alignment(
    desk: &mut Desk<'_>,
    transition: TransitionId,
    alignment: TransitionAlignment,
) -> Result<()> {
    let commands = alignment_commands(desk.view().session.project(), transition, alignment)?;
    if !commands.is_empty() {
        desk.apply_result(Command::Batch { commands }, false)?;
    }
    Ok(())
}

pub(super) fn remove(desk: &mut Desk<'_>, transition: TransitionId) -> Result<()> {
    desk.apply_result(Command::RemoveTransition { transition }, false)?;
    if desk.view().transition == Some(transition) {
        desk.clear_transition_scope();
    }
    Ok(())
}

/// The commands that realign one transition; empty when it already is.
fn alignment_commands(
    project: &project::Project,
    id: TransitionId,
    alignment: TransitionAlignment,
) -> Result<Vec<Command>> {
    let data = project.data();
    let timeline = &data.scene().timeline;
    let tr = timeline
        .transitions
        .iter()
        .find(|tr| tr.id == id)
        .ok_or(Error::Invalid("transition selection"))?;
    if tr.alignment == alignment {
        return Ok(Vec::new());
    }
    let (lo, cut, hi) = tr.bounds(timeline, data)?;
    let left = cut.0.saturating_sub(lo.0);
    let right = hi.0.saturating_sub(cut.0);
    // The longest span each alignment fits in the handles, and its lead before the cut.
    let cap = match alignment {
        TransitionAlignment::Start => right,
        TransitionAlignment::End => left,
        TransitionAlignment::Center => left
            .saturating_mul(2)
            .saturating_add(1)
            .min(right.saturating_mul(2)),
        TransitionAlignment::Custom => return Err(Error::Invalid("transition alignment")),
    };
    let duration = tr.duration.0.min(cap);
    if duration <= 0 {
        return Err(Error::Invalid("transition handles"));
    }
    let before = match alignment {
        TransitionAlignment::Start => 0,
        TransitionAlignment::End => duration,
        _ => duration / 2,
    };
    let start = Tick(cut.0.saturating_sub(before));
    let mut commands = Vec::new();
    if duration < tr.duration.0 {
        commands.push(Command::SetTransitionDuration {
            transition: id,
            duration: Tick(duration),
        });
    }
    commands.push(Command::MoveTransition {
        transition: id,
        start,
    });
    Ok(commands)
}

//! Applies transition inspector edits through the transition commands.
use crate::editor::{Error, Port, Result};
use engine::Command;
use project::{TransitionAlignment, TransitionId};
use time::Tick;

/// Resize through `transition.set_duration`; the engine clamps to the handles
/// on both sides and refuses a span with none, leaving the project unchanged.
pub(super) fn set_duration(
    port: &mut Port<'_>,
    transition: TransitionId,
    duration: Tick,
) -> Result<()> {
    port.apply_result(Command::SetTransitionDuration {
        transition,
        duration,
    })?;
    Ok(())
}

/// Alignment has no command of its own: it is a `transition.move` to the span
/// position that alignment names, batched with a shrink when the handle on the
/// side the span must grow into is shorter than the span.
pub(super) fn set_alignment(
    port: &mut Port<'_>,
    transition: TransitionId,
    alignment: TransitionAlignment,
) -> Result<()> {
    let commands = alignment_commands(port.view().session.project(), transition, alignment)?;
    if !commands.is_empty() {
        port.apply_result(Command::Batch { commands })?;
    }
    Ok(())
}

/// Copy this transition's kind and duration to every other transition as one undo step.
/// The engine leaves transitions on locked tracks alone; a toast says so when it did.
pub(super) fn apply_to_all(port: &mut Port<'_>, transition: TransitionId) -> Result<()> {
    let outcome = port.apply_result(Command::ApplyTransitionToAll { transition })?;
    if !outcome.skipped.is_empty() {
        let text = port.tr(crate::i18n::Text::TransitionSkippedLocked);
        port.toast(text, None);
    }
    Ok(())
}

pub(super) fn remove(port: &mut Port<'_>, transition: TransitionId) -> Result<()> {
    port.apply_result(Command::RemoveTransition { transition })?;
    if port.view().transition == Some(transition) {
        port.leave_transition();
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

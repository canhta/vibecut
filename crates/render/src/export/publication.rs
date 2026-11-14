//! Publishes a finalized export atomically under the destination name.

use crate::{ExecutionHooks, Result, Stage, check};
use project::atomic::{DirectoryStage, StreamingStage};

/// Finalize the staged frame directory and promote it under the destination name,
/// through the same cancellation fence as a file.
pub(super) fn publish_directory(stage: DirectoryStage, hooks: &dyn ExecutionHooks) -> Result<()> {
    let finalized = stage.finalize()?;
    hooks.progress(Stage::Finalized, 1.0)?;
    check(hooks)?;
    hooks.progress(Stage::Publish, 1.0)?;
    check(hooks)?;
    hooks.begin_publish()?;
    finalized.publish()?;
    Ok(())
}

/// Finalize the staged bytes and publish them under the destination name.
pub(super) fn publish(stage: StreamingStage, hooks: &dyn ExecutionHooks) -> Result<()> {
    let finalized = stage.finalize()?;
    hooks.progress(Stage::Finalized, 1.0)?;
    check(hooks)?;
    hooks.progress(Stage::Publish, 1.0)?;
    check(hooks)?;
    // The engine implementation atomically fences cancellation here. No fallible
    // callback or result allocation runs after successful irreversible publication.
    hooks.begin_publish()?;
    finalized.publish()?;
    Ok(())
}

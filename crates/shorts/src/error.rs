//! Refusals owned by the shorts workflows.
/// Typed reason payload for a clip-boundary refusal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoundaryFailure {
    NoSafeFrame,
    SegmentDuration,
    TotalDuration,
    SegmentOverlap,
    Candidate,
}

/// A shorts workflow failure: a boundary or export refusal, or a kernel failure
/// the workflow passes through.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("clip boundary refused at short {short}, segment {segment}: {reason:?}")]
    ClipBoundary {
        short: usize,
        segment: usize,
        reason: BoundaryFailure,
    },
    #[error("clip export refused at short {short}: {reason}")]
    ClipExport { short: usize, reason: &'static str },
    #[error(transparent)]
    Engine(#[from] engine::EditError),
}

impl Error {
    /// Wrap a failure from a domain crate through the kernel's typed error.
    pub(crate) fn kernel(error: impl Into<engine::EditError>) -> Self {
        Self::Engine(error.into())
    }
}

pub type Result<T> = std::result::Result<T, Error>;

impl From<engine::RuntimeError> for Error {
    fn from(error: engine::RuntimeError) -> Self {
        Self::Engine(error.into())
    }
}

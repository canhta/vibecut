//! States why a turn could not start.

/// A refusal before the first request: no fork exists and nothing was sent.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The live session has an open gesture, preview or group, so it cannot be forked.
    #[error("the session could not be forked or prepared")]
    Session(#[from] engine::EditError),
    /// The evaluation fixture is not a valid project.
    #[error("the project could not be built")]
    Project(#[from] project::Error),
    /// The conversation refused the request before any network call.
    #[error("the conversation refused the request")]
    Chat(#[from] ai::Error),
    /// The turn limits are outside their bounds.
    #[error("invalid turn limits")]
    Limits,
}

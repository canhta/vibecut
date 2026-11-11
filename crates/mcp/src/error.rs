//! States adapter refusals and the engine or transport failures around them.
use engine::Error as EngineError;
use std::io;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Engine(#[from] EngineError),
    #[error("{0}")]
    Rejected(&'static str),
}

impl From<engine::RuntimeError> for Error {
    fn from(error: engine::RuntimeError) -> Self {
        Self::Engine(error.into())
    }
}

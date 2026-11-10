//! States adapter refusals and the engine or transport failures around them.
use engine::EditError;
use std::io;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Engine(#[from] EditError),
    #[error("{0}")]
    Rejected(&'static str),
}
